use super::types::AuthStatus;
use crate::storage::{self, AppPaths};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AuthType {
    Api,
}

#[derive(Clone, Serialize, Deserialize)]
struct AuthEntry {
    #[serde(rename = "type")]
    kind: AuthType,
    key: String,
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Provider-ID darf nicht leer sein")]
    EmptyProviderId,
    #[error("API-Key darf nicht leer sein")]
    EmptyKey,
    #[error("Auth-Daten konnten nicht serialisiert werden: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("Auth-Daten konnten nicht gespeichert werden: {0}")]
    Io(#[from] io::Error),
}

pub struct AuthStore {
    entries: BTreeMap<String, AuthEntry>,
    path: PathBuf,
}

impl AuthStore {
    pub fn load(paths: &AppPaths) -> Self {
        Self::load_from(storage::auth_json_path(paths))
    }

    pub fn load_from(path: PathBuf) -> Self {
        Self {
            entries: load_auth_json(&path),
            path,
        }
    }

    pub fn set(&mut self, provider_id: String, key: String) -> Result<(), AuthError> {
        if provider_id.trim().is_empty() {
            return Err(AuthError::EmptyProviderId);
        }
        if key.trim().is_empty() {
            return Err(AuthError::EmptyKey);
        }
        let mut next = self.entries.clone();
        next.insert(
            provider_id,
            AuthEntry {
                kind: AuthType::Api,
                key,
            },
        );
        self.commit(next)
    }

    pub fn remove(&mut self, provider_id: &str) -> Result<(), AuthError> {
        if provider_id.trim().is_empty() {
            return Err(AuthError::EmptyProviderId);
        }
        if !self.entries.contains_key(provider_id) {
            return Ok(());
        }
        let mut next = self.entries.clone();
        next.remove(provider_id);
        self.commit(next)
    }

    pub fn status(&self) -> AuthStatus {
        self.entries
            .iter()
            .map(|(id, entry)| (id.clone(), !entry.key.is_empty()))
            .collect()
    }

    /// Ausschliesslich fuer den Provider-Client. Nie Keys in UI/Automation.
    pub(crate) fn get_key(&self, provider_id: &str) -> Option<String> {
        self.entries.get(provider_id).map(|entry| entry.key.clone())
    }

    fn commit(&mut self, next: BTreeMap<String, AuthEntry>) -> Result<(), AuthError> {
        save_secure_json_atomic(&self.path, &next)?;
        self.entries = next;
        Ok(())
    }
}

fn load_auth_json(path: &PathBuf) -> BTreeMap<String, AuthEntry> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_secure_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), AuthError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    let tmp = path.with_extension("tmp");
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    // `mode()` wirkt nur beim Erstellen. Ein liegengebliebenes Temp-File
    // wird deshalb vor dem ersten Key-Byte ebenfalls explizit abgesichert.
    set_private_permissions(&file)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)?;
    set_path_private_permissions(path)?;
    Ok(())
}

#[cfg(unix)]
fn set_private_permissions(file: &File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_permissions(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_path_private_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_path_private_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn set_status_and_remove_never_expose_key() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("auth.json");
        let mut store = AuthStore::load_from(path.clone());
        store.set("openai".into(), "top-secret".into()).unwrap();

        assert_eq!(Some(&true), store.status().get("openai"));
        let status_json = serde_json::to_string(&store.status()).unwrap();
        assert!(!status_json.contains("top-secret"));
        assert_eq!(Some("top-secret".to_string()), store.get_key("openai"));
        let persisted = fs::read_to_string(&path).unwrap();
        assert!(persisted.contains(r#""type": "api""#));

        store.remove("openai").unwrap();
        assert!(!store.status().contains_key("openai"));
        let persisted = fs::read_to_string(path).unwrap();
        assert!(!persisted.contains("top-secret"));
    }

    #[cfg(unix)]
    #[test]
    fn auth_file_has_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let path = temp.path().join("auth.json");
        let mut store = AuthStore::load_from(path.clone());
        store.set("openai".into(), "secret".into()).unwrap();

        assert_eq!(
            0o600,
            fs::metadata(path).unwrap().permissions().mode() & 0o777
        );
    }

    #[cfg(unix)]
    #[test]
    fn existing_temp_file_is_secured_before_reuse() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let path = temp.path().join("auth.json");
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, "").unwrap();
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644)).unwrap();
        let mut store = AuthStore::load_from(path);
        store.set("openai".into(), "secret".into()).unwrap();

        assert!(!tmp.exists());
    }

    #[test]
    fn setter_failure_preserves_state_and_retry_fails_again() {
        let temp = TempDir::new().unwrap();
        let regular_file = temp.path().join("not_a_dir");
        fs::write(&regular_file, "blocking file").unwrap();
        let unwriteable_path = regular_file.join("auth.json");

        let mut store = AuthStore::load_from(unwriteable_path);
        assert!(store.status().is_empty());

        let err1 = store.set("openrouter".into(), "sk-test-key".into());
        assert!(err1.is_err(), "Setter must fail when path is unwriteable");
        assert!(
            store.status().is_empty(),
            "AuthStore state must remain unchanged after write failure"
        );

        let err2 = store.set("openrouter".into(), "sk-test-key".into());
        assert!(
            err2.is_err(),
            "Repeated setter call must fail again and not succeed silently"
        );
        assert!(
            store.status().is_empty(),
            "AuthStore state must still be unchanged"
        );
    }
}
