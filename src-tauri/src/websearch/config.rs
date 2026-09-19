//! Konfiguration der Webrecherche (`websearch.json` neben `ai.json`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::storage::{self, AppPaths, AppResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchConfig {
    /// Websuche im Chat erlauben.
    #[serde(default)]
    pub enabled: bool,
    /// Basis-URL der SearXNG-Instanz ("" = nicht konfiguriert).
    #[serde(default)]
    pub searxng_url: String,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            searxng_url: String::new(),
        }
    }
}

impl WebSearchConfig {
    /// Aktiv, wenn eingeschaltet und eine URL konfiguriert ist.
    pub fn is_active(&self) -> bool {
        self.enabled && !self.searxng_url.trim().is_empty()
    }
}

pub fn config_path(paths: &AppPaths) -> PathBuf {
    storage::ai_data_file(paths, "websearch.json")
}

/// Liest die Konfiguration; fehlende, leere oder defekte Datei -> Default.
pub fn load(paths: &AppPaths) -> WebSearchConfig {
    load_from(&config_path(paths))
}

fn load_from(path: &Path) -> WebSearchConfig {
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

/// Normalisiert (URL trimmen, gueltige URL erzwingen) und schreibt atomar.
pub fn save(paths: &AppPaths, config: &WebSearchConfig) -> AppResult<WebSearchConfig> {
    let normalized = normalize(config)?;
    crate::ai::config::save_json_atomic(&config_path(paths), &normalized)
        .map_err(|err| format!("Websuche-Konfiguration konnte nicht gespeichert werden: {err}"))?;
    Ok(normalized)
}

pub fn normalize(config: &WebSearchConfig) -> AppResult<WebSearchConfig> {
    Ok(WebSearchConfig {
        enabled: config.enabled,
        searxng_url: normalize_url(&config.searxng_url)?,
    })
}

/// Trimmt die URL; eine nichtleere URL muss HTTP(S) mit Host sein.
pub fn normalize_url(url: &str) -> AppResult<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let parsed = url::Url::parse(trimmed).map_err(|_| "Ungültige SearXNG-URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("Ungültige SearXNG-URL".to_string());
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_paths() -> (TempDir, AppPaths) {
        let temp = TempDir::new().unwrap();
        let paths = AppPaths {
            db_path: temp.path().join("videos.db"),
            config_path: temp.path().join("config.json"),
        };
        (temp, paths)
    }

    #[test]
    fn l9_missing_file_yields_defaults() {
        let (_temp, paths) = temp_paths();
        assert_eq!(load(&paths), WebSearchConfig::default());
        assert!(!load(&paths).is_active());
    }

    #[test]
    fn l9_roundtrip_trims_and_persists() {
        let (_temp, paths) = temp_paths();
        let saved = save(
            &paths,
            &WebSearchConfig {
                enabled: true,
                searxng_url: "  http://127.0.0.1:8080/  ".to_string(),
            },
        )
        .unwrap();
        assert_eq!(saved.searxng_url, "http://127.0.0.1:8080/");
        assert!(saved.is_active());
        assert_eq!(load(&paths), saved);
        // Datei liegt neben ai.json.
        assert_eq!(config_path(&paths).file_name().unwrap(), "websearch.json");
        assert!(config_path(&paths).exists());
    }

    #[test]
    fn l9_invalid_urls_are_rejected() {
        let (_temp, paths) = temp_paths();
        for bad in [
            "not-a-url",
            "ftp://example.com",
            "file:///etc/passwd",
            "http://",
        ] {
            let error = save(
                &paths,
                &WebSearchConfig {
                    enabled: true,
                    searxng_url: bad.to_string(),
                },
            )
            .unwrap_err();
            assert_eq!(error, "Ungültige SearXNG-URL", "{bad}");
        }
        // Leer bleibt erlaubt (nicht konfiguriert).
        assert_eq!(normalize_url("   ").unwrap(), "");
    }

    #[test]
    fn l9_broken_file_falls_back_to_defaults() {
        let (_temp, paths) = temp_paths();
        std::fs::write(config_path(&paths), "{kaputt").unwrap();
        assert_eq!(load(&paths), WebSearchConfig::default());

        std::fs::write(config_path(&paths), "   ").unwrap();
        assert_eq!(load(&paths), WebSearchConfig::default());

        std::fs::write(
            config_path(&paths),
            r#"{"enabled":true,"searxngUrl":"http://127.0.0.1:8080"}"#,
        )
        .unwrap();
        let loaded = load(&paths);
        assert!(loaded.enabled);
        assert_eq!(loaded.searxng_url, "http://127.0.0.1:8080");
    }
}
