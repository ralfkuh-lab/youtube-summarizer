use super::types::{
    AiConfig, AiConfiguredModel, AiModelRef, AiProviderConfig, AiProviderOptions,
    CustomProviderDefinition,
};
use crate::storage::{self, AppPaths};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use thiserror::Error;

static ATOMIC_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

pub(crate) fn atomic_tmp_path(path: &Path) -> PathBuf {
    let pid = std::process::id();
    let n = ATOMIC_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    path.with_file_name(format!("{name}.{pid}.{n}.tmp"))
}

#[derive(Debug, Error)]
pub enum AiConfigError {
    #[error("KI-Konfiguration konnte nicht gespeichert werden: {0}")]
    Persist(#[from] io::Error),
    #[error("Ungültige Provider-ID '{0}': erlaubt sind nur Kleinbuchstaben, Zahlen, '-' und '_'")]
    InvalidSlug(String),
    #[error("Anzeigename des Custom-Providers darf nicht leer sein")]
    EmptyName,
    #[error("Basis-URL des Custom-Providers darf nicht leer sein")]
    EmptyBaseUrl,
    #[error("Provider '{0}' ist kein Custom-Provider")]
    NotCustom(String),
    #[error("Provider und Modell müssen entweder beide gesetzt oder beide leer sein")]
    IncompleteDefaultModel,
    #[error("Provider- und Modell-ID dürfen nicht leer sein")]
    EmptyIdentifier,
}

#[derive(Debug, Clone)]
pub struct AiConfigService {
    data: AiConfig,
    path: PathBuf,
}

impl AiConfigService {
    pub fn load(paths: &AppPaths) -> Self {
        Self::load_from(storage::ai_json_path(paths))
    }

    pub fn load_from(path: PathBuf) -> Self {
        Self {
            data: load_ai_json(&path),
            path,
        }
    }

    pub fn data(&self) -> AiConfig {
        self.data.clone()
    }

    pub fn provider_enable(
        &mut self,
        provider_id: String,
        enabled: bool,
    ) -> Result<(), AiConfigError> {
        validate_nonempty_id(&provider_id)?;
        if !enabled && !self.data.provider.contains_key(&provider_id) {
            return Ok(());
        }
        let provider = self.data.provider.entry(provider_id).or_default();
        if provider.enabled == enabled {
            return Ok(());
        }
        provider.enabled = enabled;
        self.save()
    }

    pub fn model_toggle(
        &mut self,
        provider_id: String,
        model_id: String,
        on: bool,
    ) -> Result<(), AiConfigError> {
        validate_nonempty_id(&provider_id)?;
        validate_nonempty_id(&model_id)?;
        if !on && !self.data.provider.contains_key(&provider_id) {
            return Ok(());
        }
        let whitelist = &mut self.data.provider.entry(provider_id).or_default().whitelist;
        let before = whitelist.clone();
        if on {
            let mut found = false;
            whitelist.retain(|id| {
                if id != &model_id {
                    return true;
                }
                let keep = !found;
                found = true;
                keep
            });
            if !found {
                whitelist.push(model_id);
            }
        } else {
            whitelist.retain(|id| id != &model_id);
        }
        if *whitelist == before {
            return Ok(());
        }
        self.save()
    }

    pub fn custom_upsert(
        &mut self,
        mut definition: CustomProviderDefinition,
    ) -> Result<(), AiConfigError> {
        validate_slug(&definition.id)?;
        definition.name = definition.name.trim().to_string();
        definition.base_url = definition.base_url.trim().to_string();
        if definition.name.is_empty() {
            return Err(AiConfigError::EmptyName);
        }
        if definition.base_url.is_empty() {
            return Err(AiConfigError::EmptyBaseUrl);
        }
        if self
            .data
            .provider
            .get(&definition.id)
            .is_some_and(|provider| !provider.custom)
        {
            return Err(AiConfigError::NotCustom(definition.id));
        }

        let provider =
            self.data
                .provider
                .entry(definition.id)
                .or_insert_with(|| AiProviderConfig {
                    enabled: true,
                    custom: true,
                    ..AiProviderConfig::default()
                });
        provider.name = Some(definition.name);
        provider.custom = true;
        provider.options = Some(AiProviderOptions {
            base_url: definition.base_url,
        });
        self.save()
    }

    pub fn custom_delete(&mut self, id: &str) -> Result<(), AiConfigError> {
        validate_slug(id)?;
        let Some(provider) = self.data.provider.get(id) else {
            return Ok(());
        };
        if !provider.custom {
            return Err(AiConfigError::NotCustom(id.to_string()));
        }
        self.data.provider.remove(id);
        if self
            .data
            .default_model
            .as_ref()
            .is_some_and(|default| default.provider == id)
        {
            self.data.default_model = None;
        }
        // Der zugehoerige Auth-Eintrag bleibt bewusst bestehen: auth.json
        // ist ein separater Store und wird nur ueber Auth-Commands veraendert.
        self.save()
    }

    pub fn custom_models_replace(
        &mut self,
        id: &str,
        model_ids: impl IntoIterator<Item = String>,
    ) -> Result<(), AiConfigError> {
        validate_slug(id)?;
        let Some(provider) = self.data.provider.get_mut(id) else {
            return Err(AiConfigError::NotCustom(id.to_string()));
        };
        if !provider.custom {
            return Err(AiConfigError::NotCustom(id.to_string()));
        }

        let existing = provider.models.clone();
        let models = model_ids
            .into_iter()
            .map(|model_id| {
                let configured = existing.get(&model_id).cloned().unwrap_or_default();
                (model_id, configured)
            })
            .collect::<BTreeMap<String, AiConfiguredModel>>();
        if models == existing {
            return Ok(());
        }
        provider.models = models;
        self.save()
    }

    pub fn default_model_set(
        &mut self,
        provider_id: Option<String>,
        model_id: Option<String>,
    ) -> Result<(), AiConfigError> {
        let next = match (provider_id, model_id) {
            (None, None) => None,
            (Some(provider), Some(model)) => {
                validate_nonempty_id(&provider)?;
                validate_nonempty_id(&model)?;
                Some(AiModelRef { provider, model })
            }
            _ => return Err(AiConfigError::IncompleteDefaultModel),
        };
        if self.data.default_model == next {
            return Ok(());
        }
        self.data.default_model = next;
        self.save()
    }

    fn save(&self) -> Result<(), AiConfigError> {
        save_json_atomic(&self.path, &self.data)?;
        Ok(())
    }
}

fn load_ai_json(path: &PathBuf) -> AiConfig {
    // load_json like logic, default if missing/empty/invalid
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                serde_json::from_str(trimmed).ok()
            }
        })
        .unwrap_or_default()
}

pub(crate) fn save_json_atomic<T: serde::Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = atomic_tmp_path(path);
    let bytes = serde_json::to_vec_pretty(value)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

fn validate_nonempty_id(id: &str) -> Result<(), AiConfigError> {
    if id.trim().is_empty() {
        Err(AiConfigError::EmptyIdentifier)
    } else {
        Ok(())
    }
}

pub fn validate_slug(id: &str) -> Result<(), AiConfigError> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte))
    {
        return Err(AiConfigError::InvalidSlug(id.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn custom(id: &str) -> CustomProviderDefinition {
        CustomProviderDefinition {
            id: id.to_string(),
            name: "Test Provider".to_string(),
            base_url: "http://localhost:1234/v1".to_string(),
        }
    }

    #[test]
    fn config_round_trip_and_partial_migration() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("ai.json");
        std::fs::write(&path, r#"{"provider":{"openai":{"enabled":true}}}"#).unwrap();
        let service = AiConfigService::load_from(path.clone());
        let migrated = service.data();
        assert!(migrated.provider["openai"].enabled);
        assert!(migrated.default_model.is_none());
        assert!(migrated.translate.recent_languages.is_empty());

        // no recent_languages_set in this project (no translate), but test data roundtrip
        // simulate
        let reloaded = AiConfigService::load_from(path).data();
        assert!(reloaded.provider["openai"].enabled);
    }

    #[test]
    fn empty_file_uses_defaults() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("ai.json");
        std::fs::write(&path, "").unwrap();
        assert_eq!(AiConfig::default(), AiConfigService::load_from(path).data());
    }

    #[test]
    fn whitelist_toggle_is_idempotent_and_ordered() {
        let temp = TempDir::new().unwrap();
        let mut service = AiConfigService::load_from(temp.path().join("ai.json"));
        service
            .model_toggle("openai".into(), "first".into(), true)
            .unwrap();
        service
            .model_toggle("openai".into(), "second".into(), true)
            .unwrap();
        service
            .model_toggle("openai".into(), "first".into(), true)
            .unwrap();
        assert_eq!(
            vec!["first", "second"],
            service.data().provider["openai"].whitelist
        );

        service
            .model_toggle("openai".into(), "first".into(), false)
            .unwrap();
        service
            .model_toggle("openai".into(), "first".into(), false)
            .unwrap();
        service
            .model_toggle("openai".into(), "first".into(), true)
            .unwrap();
        assert_eq!(
            vec!["second", "first"],
            service.data().provider["openai"].whitelist
        );
    }

    #[test]
    fn whitelist_toggle_repairs_existing_duplicates() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("ai.json");
        std::fs::write(
            &path,
            r#"{"provider":{"openai":{"whitelist":["a","b","a","a"]}}}"#,
        )
        .unwrap();
        let mut service = AiConfigService::load_from(path);
        service
            .model_toggle("openai".into(), "a".into(), true)
            .unwrap();
        assert_eq!(vec!["a", "b"], service.data().provider["openai"].whitelist);
    }

    #[test]
    fn slug_validation_accepts_and_rejects_expected_values() {
        for valid in ["a", "openai", "my-provider_2"] {
            assert!(validate_slug(valid).is_ok(), "{valid}");
        }
        for invalid in ["", "OpenAI", "with space", "ümlaut", "a.b", "/path"] {
            assert!(validate_slug(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn custom_delete_clears_matching_default_model() {
        let temp = TempDir::new().unwrap();
        let mut service = AiConfigService::load_from(temp.path().join("ai.json"));
        service.custom_upsert(custom("local")).unwrap();
        service
            .default_model_set(Some("local".into()), Some("model".into()))
            .unwrap();
        service.custom_delete("local").unwrap();
        assert!(service.data().default_model.is_none());
        assert!(!service.data().provider.contains_key("local"));
    }

    #[test]
    fn custom_model_replace_preserves_existing_names_and_removes_stale_models() {
        let temp = TempDir::new().unwrap();
        let mut service = AiConfigService::load_from(temp.path().join("ai.json"));
        service.custom_upsert(custom("local")).unwrap();
        {
            let provider = service.data.provider.get_mut("local").unwrap();
            provider.models.insert(
                "keep".into(),
                AiConfiguredModel {
                    name: Some("Lesbarer Name".into()),
                },
            );
            provider
                .models
                .insert("stale".into(), AiConfiguredModel::default());
        }

        service
            .custom_models_replace("local", ["new".to_string(), "keep".to_string()])
            .unwrap();

        let data = service.data();
        let models = &data.provider["local"].models;
        assert_eq!(Some("Lesbarer Name"), models["keep"].name.as_deref());
        assert!(models.contains_key("new"));
        assert!(!models.contains_key("stale"));
    }

    #[test]
    fn custom_provider_serializes_opencode_field_names() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("ai.json");
        let mut service = AiConfigService::load_from(path.clone());
        service.custom_upsert(custom("local")).unwrap();
        service
            .default_model_set(Some("local".into()), Some("model".into()))
            .unwrap();

        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(
            Some("http://localhost:1234/v1"),
            json.pointer("/provider/local/options/baseURL")
                .and_then(serde_json::Value::as_str)
        );
        assert_eq!(
            Some("model"),
            json.pointer("/defaultModel/model")
                .and_then(serde_json::Value::as_str)
        );
    }
}
