use std::collections::BTreeMap;

use crate::ai::auth::AuthStore;
use crate::ai::catalog as ai_catalog;
use crate::ai::types::AiModelRef;
use crate::storage::{self, AppPaths};

// Migration from old config.json (best effort, once)
// Order: keys first (with error handling), THEN atomic ai.json write (done marker).
// Customs: any id not in the 4 known hosted ones (incl. "ollama") treated as custom.
pub(crate) fn ensure_migrated(paths: &AppPaths) {
    let ai_path = storage::ai_json_path(paths);
    if ai_path.exists() {
        return; // already migrated or fresh
    }
    // try read old config.json for ai block (raw, no type)
    let old_cfg_text = match std::fs::read_to_string(&paths.config_path) {
        Ok(t) => t,
        Err(_) => {
            // even no config, write marker
            let _ = crate::ai::config::save_json_atomic(
                &ai_path,
                &crate::ai::types::AiConfig::default(),
            );
            return;
        }
    };
    let old: serde_json::Value = match serde_json::from_str(&old_cfg_text) {
        Ok(v) => v,
        Err(_) => {
            let _ = crate::ai::config::save_json_atomic(
                &ai_path,
                &crate::ai::types::AiConfig::default(),
            );
            return;
        }
    };
    let old_ai = match old.get("ai") {
        Some(a) if a.is_object() => a,
        _ => {
            let _ = crate::ai::config::save_json_atomic(
                &ai_path,
                &crate::ai::types::AiConfig::default(),
            );
            return;
        }
    };

    let known_hosted: [&str; 4] = ["ollama_cloud", "openrouter", "opencode_zen", "opencode_go"];
    let is_hosted = |id: &str| known_hosted.contains(&id);

    // Build minimal old shape
    let old_provider = old_ai
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let old_model = old_ai
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let old_key = old_ai
        .get("api_key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let old_endpoint = old_ai
        .get("endpoint_override")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut providers_map: BTreeMap<String, crate::ai::types::AiProviderConfig> = BTreeMap::new();
    let mut default_model: Option<AiModelRef> = None;
    let mut migrated_ids: Vec<String> = vec![];

    let catalog = ai_catalog::load(paths).catalog;

    // active provider -> provider entry (custom or mapped), even without key
    if !old_provider.is_empty() {
        let is_custom = !is_hosted(&old_provider);
        let mapped = if is_custom {
            old_provider.clone()
        } else {
            map_old_provider_id(&old_provider).unwrap_or_else(|| old_provider.clone())
        };
        if catalog.contains_key(&mapped) || is_custom {
            let mut pcfg = crate::ai::types::AiProviderConfig {
                enabled: true,
                custom: is_custom,
                ..Default::default()
            };
            if is_custom {
                if let Some(ep) = old_endpoint.clone().filter(|e| !e.trim().is_empty()) {
                    pcfg.options = Some(crate::ai::types::AiProviderOptions {
                        base_url: strip_chat_suffix(&ep),
                    });
                }
            }
            if !old_model.is_empty() {
                pcfg.whitelist = vec![old_model.clone()];
                default_model = Some(AiModelRef {
                    provider: mapped.clone(),
                    model: old_model.clone(),
                });
            }
            providers_map.insert(mapped.clone(), pcfg);
            migrated_ids.push(mapped.clone());
        }
    }

    // other providers from list: add entry regardless of key (b), custom if not hosted
    if let Some(provs) = old_ai.get("providers").and_then(|v| v.as_array()) {
        for prov in provs {
            let pid = prov
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if pid.is_empty() {
                continue;
            }
            let is_custom = !is_hosted(&pid);
            let mapped = if is_custom {
                pid.clone()
            } else {
                map_old_provider_id(&pid).unwrap_or_else(|| pid.clone())
            };
            if catalog.contains_key(&mapped) || is_custom {
                let entry = providers_map.entry(mapped.clone()).or_insert_with(|| {
                    crate::ai::types::AiProviderConfig {
                        enabled: true,
                        custom: is_custom,
                        ..Default::default()
                    }
                });
                entry.enabled = true;
                if is_custom {
                    if let Some(ep) = prov
                        .get("endpoint_override")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                    {
                        entry.options = Some(crate::ai::types::AiProviderOptions {
                            base_url: strip_chat_suffix(ep),
                        });
                    }
                }
                if !migrated_ids.contains(&mapped) {
                    migrated_ids.push(mapped);
                }
            }
            // keys handled separately below
        }
    }

    // FIRST: migrate keys (a) - proper error handling, never swallow silently for keys
    let mut auth_store = AuthStore::load(paths);
    if !old_key.trim().is_empty() {
        if let Some(m) = (if is_hosted(&old_provider) {
            map_old_provider_id(&old_provider)
        } else {
            None
        })
        .or_else(|| {
            if !is_hosted(&old_provider) {
                Some(old_provider.clone())
            } else {
                None
            }
        }) {
            if let Err(e) = auth_store.set(m.clone(), old_key.clone()) {
                eprintln!(
                    "AI migration: auth key set error for provider (id redacted): {}",
                    e
                );
            }
        }
    }
    if let Some(provs) = old_ai.get("providers").and_then(|v| v.as_array()) {
        for prov in provs {
            let pid = prov
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let pkey = prov
                .get("api_key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if pkey.trim().is_empty() {
                continue;
            }
            let target = if is_hosted(&pid) {
                map_old_provider_id(&pid).unwrap_or(pid.clone())
            } else {
                pid.clone()
            };
            if let Err(e) = auth_store.set(target, pkey) {
                eprintln!(
                    "AI migration: auth key set error for listed provider (id redacted): {}",
                    e
                );
            }
        }
    }

    // if nothing at all, still write marker (d)
    if providers_map.is_empty() && old_key.trim().is_empty() {
        let _ =
            crate::ai::config::save_json_atomic(&ai_path, &crate::ai::types::AiConfig::default());
        return;
    }

    let new_ai = crate::ai::types::AiConfig {
        provider: providers_map,
        default_model,
        translate: Default::default(),
    };

    // THEN atomic ai.json (a, d) -- this marks done
    if let Err(e) = crate::ai::config::save_json_atomic(&ai_path, &new_ai) {
        eprintln!("AI migration: failed to write ai.json marker: {}", e);
    }

    eprintln!("AI migration completed for providers: {:?}", migrated_ids);
}

pub(crate) fn map_old_provider_id(old: &str) -> Option<String> {
    match old {
        "opencode_zen" => Some("opencode".into()),
        "opencode_go" => Some("opencode-go".into()),
        "ollama_cloud" => Some("ollama-cloud".into()),
        "openrouter" => Some("openrouter".into()),
        _ => None,
    }
}

// Old endpoint overrides stored the full chat URL; the new baseURL schema expects the API root.
pub(crate) fn strip_chat_suffix(endpoint: &str) -> String {
    endpoint
        .trim()
        .trim_end_matches('/')
        .trim_end_matches("/chat/completions")
        .to_string()
}
