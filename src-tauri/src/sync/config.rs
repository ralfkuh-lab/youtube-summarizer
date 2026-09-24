//! Einstellungen des Syncs (`sync.json` neben `config.json`, docs/spec-sync.md
//! „Einstellungen“). Atomar und privat geschrieben wie `auth.json`; das Token
//! verlässt das Backend nie (Frontend sieht nur `hasToken`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::storage::{self, AppPaths, AppResult};

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub server_url: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub new_videos_local: bool,
}

// Kein `Debug` mit Token: nur die unkritischen Felder.
impl std::fmt::Debug for SyncConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncConfig")
            .field("enabled", &self.enabled)
            .field("server_url", &self.server_url)
            .field("has_token", &!self.token.is_empty())
            .field("new_videos_local", &self.new_videos_local)
            .finish()
    }
}

impl SyncConfig {
    /// Eingeschaltet und vollständig konfiguriert.
    pub fn is_active(&self) -> bool {
        self.enabled && !self.server_url.is_empty() && !self.token.is_empty()
    }
}

/// Was das Frontend sieht und schickt. Beim Schreiben bedeutet ein leeres
/// oder fehlendes `token` „unverändert“.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfigView {
    pub enabled: bool,
    pub server_url: String,
    pub has_token: bool,
    pub new_videos_local: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfigInput {
    pub enabled: bool,
    pub server_url: String,
    #[serde(default)]
    pub token: Option<String>,
    pub new_videos_local: bool,
}

impl From<&SyncConfig> for SyncConfigView {
    fn from(config: &SyncConfig) -> Self {
        Self {
            enabled: config.enabled,
            server_url: config.server_url.clone(),
            has_token: !config.token.is_empty(),
            new_videos_local: config.new_videos_local,
        }
    }
}

pub fn config_path(paths: &AppPaths) -> PathBuf {
    storage::ai_data_file(paths, "sync.json")
}

/// Fehlende oder defekte Datei → Sync aus.
pub fn load(paths: &AppPaths) -> SyncConfig {
    load_from(&config_path(paths))
}

fn load_from(path: &Path) -> SyncConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Übernimmt eine Eingabe (leeres Token = unverändert), prüft sie und
/// schreibt atomar mit Rechten 0600.
pub fn save(paths: &AppPaths, input: &SyncConfigInput) -> AppResult<SyncConfig> {
    let current = load(paths);
    let token = match input.token.as_deref().map(str::trim) {
        Some(token) if !token.is_empty() => token.to_string(),
        _ => current.token,
    };
    let next = SyncConfig {
        enabled: input.enabled,
        server_url: normalize_url(&input.server_url)?,
        token,
        new_videos_local: input.new_videos_local,
    };
    if next.enabled && (next.server_url.is_empty() || next.token.is_empty()) {
        return Err("Für die Synchronisation sind Server-URL und Token nötig".to_string());
    }
    crate::ai::auth::save_secure_json_atomic(&config_path(paths), &next)
        .map_err(|err| format!("Sync-Einstellungen konnten nicht gespeichert werden: {err}"))?;
    Ok(next)
}

/// Trimmt; eine nichtleere URL muss `https` sein, `http` nur für `localhost`
/// und `127.0.0.1`. Ein abschließender `/` entfällt.
pub fn normalize_url(url: &str) -> AppResult<String> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let invalid = || "Ungültige Server-URL (https, http nur für localhost)".to_string();
    let parsed = url::Url::parse(trimmed).map_err(|_| invalid())?;
    let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1"));
    let allowed = match parsed.scheme() {
        "https" => parsed.host_str().is_some(),
        "http" => local,
        _ => false,
    };
    if !allowed || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(invalid());
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

    fn input(url: &str, token: Option<&str>) -> SyncConfigInput {
        SyncConfigInput {
            enabled: true,
            server_url: url.to_string(),
            token: token.map(str::to_string),
            new_videos_local: false,
        }
    }

    #[test]
    fn missing_file_means_disabled() {
        let (_temp, paths) = temp_paths();
        assert_eq!(load(&paths), SyncConfig::default());
        assert!(!load(&paths).is_active());
    }

    #[test]
    fn empty_token_keeps_stored_token_and_view_hides_it() {
        let (_temp, paths) = temp_paths();
        save(&paths, &input("https://sync.example.com/", Some("geheim"))).unwrap();
        let saved = save(&paths, &input("https://sync.example.com", Some("  "))).unwrap();
        assert_eq!(saved.token, "geheim");
        assert_eq!(saved.server_url, "https://sync.example.com");
        let view = serde_json::to_string(&SyncConfigView::from(&load(&paths))).unwrap();
        assert!(view.contains(r#""hasToken":true"#), "{view}");
        assert!(!view.contains("geheim"));
        assert!(!format!("{:?}", load(&paths)).contains("geheim"));
    }

    #[test]
    fn url_rules() {
        for good in [
            "https://yt-sync.example.com",
            "http://localhost:8080",
            "http://127.0.0.1:9",
        ] {
            assert!(normalize_url(good).is_ok(), "{good}");
        }
        for bad in [
            "http://example.com",
            "ftp://localhost",
            "kein-url",
            "https://example.com/?a=1",
        ] {
            assert!(normalize_url(bad).is_err(), "{bad}");
        }
        let (_temp, paths) = temp_paths();
        let error = save(&paths, &input("", Some("t"))).unwrap_err();
        assert!(error.contains("nötig"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let (_temp, paths) = temp_paths();
        save(&paths, &input("http://localhost:1", Some("t"))).unwrap();
        let mode = std::fs::metadata(config_path(&paths))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
