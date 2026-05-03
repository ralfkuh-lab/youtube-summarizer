use std::fs;
use std::path::PathBuf;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::models::{
    default_ai_provider_configs, AiConfig, AiModel, AiProviderConfig, AppConfig, Chapter, NewVideo,
    Video,
};

pub type AppResult<T> = Result<T, String>;

const VIDEO_COLUMNS: &str = r#"
    id, video_id, url, title, thumbnail_url, thumbnail_data,
    transcript, chapters, summary, summary_provider, summary_model,
    published_at, created_at, updated_at
"#;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub db_path: PathBuf,
    pub config_path: PathBuf,
}

fn open_db(paths: &AppPaths) -> AppResult<Connection> {
    Connection::open(&paths.db_path)
        .map_err(|err| format!("Datenbank konnte nicht geöffnet werden: {err}"))
}

pub fn init_db(paths: &AppPaths) -> AppResult<()> {
    let conn = open_db(paths)?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS videos (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            video_id TEXT NOT NULL UNIQUE,
            url TEXT NOT NULL,
            title TEXT NOT NULL,
            thumbnail_url TEXT NOT NULL,
            thumbnail_data BLOB,
            transcript TEXT,
            chapters TEXT,
            summary TEXT,
            summary_provider TEXT,
            summary_model TEXT,
            published_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        "#,
    )
    .map_err(|err| format!("Datenbank konnte nicht initialisiert werden: {err}"))?;
    Ok(())
}

pub fn load_config(paths: &AppPaths) -> AppResult<AppConfig> {
    if !paths.config_path.exists() {
        let cfg = AppConfig::default();
        save_config(paths, &cfg)?;
        return Ok(cfg);
    }

    let raw = fs::read_to_string(&paths.config_path)
        .map_err(|err| format!("Konfiguration konnte nicht gelesen werden: {err}"))?;
    serde_json::from_str(&raw).map_err(|err| format!("Konfiguration ist ungültig: {err}"))
}

pub fn save_config(paths: &AppPaths, config: &AppConfig) -> AppResult<()> {
    if let Some(parent) = paths.config_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Konfigurationsordner konnte nicht erstellt werden: {err}"))?;
    }
    let raw = serde_json::to_string_pretty(config)
        .map_err(|err| format!("Konfiguration konnte nicht serialisiert werden: {err}"))?;
    fs::write(&paths.config_path, raw)
        .map_err(|err| format!("Konfiguration konnte nicht gespeichert werden: {err}"))
}

pub fn get_ai_config(paths: &AppPaths) -> AppResult<AiConfig> {
    let mut config = load_config(paths)?;
    config.ai = normalize_ai_config(config.ai);
    save_config(paths, &config)?;
    Ok(config.ai)
}

pub fn update_ai_config(paths: &AppPaths, ai: AiConfig) -> AppResult<AiConfig> {
    let mut config = load_config(paths)?;
    config.ai = normalize_ai_config(ai);
    save_config(paths, &config)?;
    Ok(config.ai)
}

pub fn update_provider_config(
    paths: &AppPaths,
    provider_id: String,
    name: Option<String>,
    enabled: bool,
    api_key_required: Option<bool>,
    api_key: String,
    model: String,
    endpoint_override: Option<String>,
    activate: bool,
    account_tier: Option<String>,
) -> AppResult<AiConfig> {
    let mut config = load_config(paths)?;
    let mut ai = normalize_ai_config(config.ai);
    let existing_models = provider_config(&ai, &provider_id)
        .map(|provider| provider.models.clone())
        .unwrap_or_default();
    let existing_models_updated_at =
        provider_config(&ai, &provider_id).and_then(|provider| provider.models_updated_at.clone());
    let existing_name =
        provider_config(&ai, &provider_id).and_then(|provider| provider.name.clone());
    let existing_api_key_required =
        provider_config(&ai, &provider_id).map(|provider| provider.api_key_required);
    let existing_account_tier =
        provider_config(&ai, &provider_id).and_then(|provider| provider.account_tier.clone());
    upsert_provider(
        &mut ai,
        AiProviderConfig {
            id: provider_id.clone(),
            name: normalize_name(name).or(existing_name),
            enabled,
            api_key_required: api_key_required
                .or(existing_api_key_required)
                .unwrap_or(false),
            api_key: api_key.clone(),
            model: model.clone(),
            endpoint_override: normalize_endpoint(endpoint_override.clone()),
            models: existing_models,
            models_updated_at: existing_models_updated_at,
            last_error: None,
            account_tier: account_tier.or(existing_account_tier),
        },
    );
    sync_shared_provider_secrets(&mut ai, &provider_id);
    if activate || ai.provider == provider_id {
        ai.provider = provider_id;
        if let Some(active) = ai
            .providers
            .iter()
            .find(|provider| provider.id == ai.provider)
        {
            ai.api_key = active.api_key.clone();
            ai.model = active.model.clone();
            ai.endpoint_override = active.endpoint_override.clone();
        }
    }
    config.ai = normalize_ai_config(ai);
    save_config(paths, &config)?;
    Ok(config.ai)
}

pub fn add_custom_provider(paths: &AppPaths, local_ollama: bool) -> AppResult<AiConfig> {
    let mut config = load_config(paths)?;
    let mut ai = normalize_ai_config(config.ai);
    if local_ollama {
        upsert_provider(
            &mut ai,
            AiProviderConfig {
                id: "ollama".to_string(),
                name: Some("Ollama local".to_string()),
                enabled: true,
                api_key_required: false,
                api_key: String::new(),
                model: "llama3.2".to_string(),
                endpoint_override: Some("http://localhost:11434/v1/chat/completions".to_string()),
                models: Vec::new(),
                models_updated_at: None,
                last_error: None,
                account_tier: None,
            },
        );
    } else {
        let next_number = ai
            .providers
            .iter()
            .filter(|provider| provider.id.starts_with("custom"))
            .count()
            + 1;
        let id = format!("custom_{next_number}");
        upsert_provider(
            &mut ai,
            AiProviderConfig {
                id,
                name: Some(format!("Custom {next_number}")),
                enabled: true,
                api_key_required: false,
                api_key: String::new(),
                model: String::new(),
                endpoint_override: Some("http://localhost:1234/v1/chat/completions".to_string()),
                models: Vec::new(),
                models_updated_at: None,
                last_error: None,
                account_tier: None,
            },
        );
    }
    config.ai = normalize_ai_config(ai);
    save_config(paths, &config)?;
    Ok(config.ai)
}

pub fn delete_custom_provider(paths: &AppPaths, provider_id: &str) -> AppResult<AiConfig> {
    if !is_user_managed_provider(provider_id) {
        return Err("Only custom/local providers can be deleted".to_string());
    }

    let mut config = load_config(paths)?;
    let mut ai = normalize_ai_config(config.ai);
    let original_len = ai.providers.len();
    ai.providers.retain(|provider| provider.id != provider_id);
    if ai.providers.len() == original_len {
        return Err("Provider not found".to_string());
    }

    if ai.provider == provider_id {
        ai.provider = "ollama_cloud".to_string();
    }

    config.ai = normalize_ai_config(ai);
    save_config(paths, &config)?;
    Ok(config.ai)
}

fn is_user_managed_provider(provider_id: &str) -> bool {
    provider_id == "ollama" || provider_id.starts_with("custom")
}

pub fn update_provider_models(
    paths: &AppPaths,
    provider_id: &str,
    models: Vec<AiModel>,
    updated_at: String,
    last_error: Option<String>,
) -> AppResult<AiConfig> {
    let mut config = load_config(paths)?;
    let mut ai = normalize_ai_config(config.ai);
    if let Some(provider) = ai
        .providers
        .iter_mut()
        .find(|provider| provider.id == provider_id)
    {
        let selected_model = if provider.model.trim().is_empty()
            || !models.iter().any(|model| model.id == provider.model)
        {
            preferred_model_id(&models)
        } else {
            Some(provider.model.clone())
        };

        provider.models = models;
        if let Some(model) = selected_model {
            provider.model = model;
        }
        provider.models_updated_at = Some(updated_at);
        provider.last_error = last_error;
    }
    if ai.provider == provider_id {
        if let Some(active) = ai
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
        {
            ai.model = active.model.clone();
        }
    }
    config.ai = normalize_ai_config(ai);
    save_config(paths, &config)?;
    Ok(config.ai)
}

fn preferred_model_id(models: &[AiModel]) -> Option<String> {
    models
        .iter()
        .find(|model| model.free)
        .or_else(|| models.first())
        .map(|model| model.id.clone())
}

pub fn set_provider_error(
    paths: &AppPaths,
    provider_id: &str,
    error: String,
) -> AppResult<AiConfig> {
    set_provider_last_error(paths, provider_id, Some(error))
}

fn set_provider_last_error(
    paths: &AppPaths,
    provider_id: &str,
    error: Option<String>,
) -> AppResult<AiConfig> {
    let mut config = load_config(paths)?;
    let mut ai = normalize_ai_config(config.ai);
    if let Some(provider) = ai
        .providers
        .iter_mut()
        .find(|provider| provider.id == provider_id)
    {
        provider.last_error = error;
    }
    config.ai = normalize_ai_config(ai);
    save_config(paths, &config)?;
    Ok(config.ai)
}

pub fn normalize_ai_config(mut ai: AiConfig) -> AiConfig {
    let defaults = default_ai_provider_configs();
    if ai.providers.is_empty() {
        ai.providers = defaults.clone();
    }

    for default_provider in defaults {
        if !ai
            .providers
            .iter()
            .any(|provider| provider.id == default_provider.id)
        {
            ai.providers.push(default_provider);
        }
    }

    if let Some(active) = ai
        .providers
        .iter_mut()
        .find(|provider| provider.id == ai.provider)
    {
        if !ai.api_key.is_empty() {
            active.api_key = ai.api_key.clone();
        }
        if !ai.model.is_empty() {
            active.model = ai.model.clone();
        }
        if ai.endpoint_override.is_some() {
            active.endpoint_override = ai.endpoint_override.clone();
        }
    }

    if let Some(active) = ai
        .providers
        .iter()
        .find(|provider| provider.id == ai.provider)
    {
        ai.api_key = active.api_key.clone();
        ai.model = active.model.clone();
        ai.endpoint_override = active.endpoint_override.clone();
    }

    ai
}

pub fn provider_config<'a>(ai: &'a AiConfig, provider_id: &str) -> Option<&'a AiProviderConfig> {
    ai.providers
        .iter()
        .find(|provider| provider.id == provider_id)
}

fn upsert_provider(ai: &mut AiConfig, next: AiProviderConfig) {
    if let Some(provider) = ai
        .providers
        .iter_mut()
        .find(|provider| provider.id == next.id)
    {
        *provider = next;
    } else {
        ai.providers.push(next);
    }
}

fn sync_shared_provider_secrets(ai: &mut AiConfig, provider_id: &str) {
    if !provider_id.starts_with("opencode_") {
        return;
    }

    let shared_key = ai
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .map(|provider| provider.api_key.clone())
        .unwrap_or_default();

    for provider in ai
        .providers
        .iter_mut()
        .filter(|provider| provider.id.starts_with("opencode_"))
    {
        provider.api_key = shared_key.clone();
    }
}

fn normalize_endpoint(endpoint_override: Option<String>) -> Option<String> {
    endpoint_override.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn normalize_name(name: Option<String>) -> Option<String> {
    name.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

pub fn video_exists(paths: &AppPaths, video_id: &str) -> AppResult<bool> {
    let conn = open_db(paths)?;
    let exists = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM videos WHERE video_id = ?1)",
            params![video_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| format!("Video konnte nicht geprüft werden: {err}"))?;
    Ok(exists == 1)
}

pub fn insert_video(paths: &AppPaths, video: NewVideo) -> AppResult<Video> {
    let conn = open_db(paths)?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        r#"
        INSERT INTO videos (
            video_id, url, title, thumbnail_url, thumbnail_data,
            transcript, chapters, summary, created_at, updated_at, published_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?8, ?9)
        "#,
        params![
            video.video_id,
            video.url,
            video.title,
            video.thumbnail_url,
            video.thumbnail_data,
            video.transcript,
            video.chapters,
            now,
            video.published_at,
        ],
    )
    .map_err(|err| format!("Video konnte nicht gespeichert werden: {err}"))?;

    get_video(paths, conn.last_insert_rowid())?
        .ok_or_else(|| "Gespeichertes Video wurde nicht gefunden".to_string())
}

pub fn get_videos(paths: &AppPaths) -> AppResult<Vec<Video>> {
    let conn = open_db(paths)?;
    let mut stmt = conn
        .prepare(&format!(
            r#"
            SELECT {VIDEO_COLUMNS}
            FROM videos
            ORDER BY created_at DESC
            "#
        ))
        .map_err(|err| format!("Videos konnten nicht geladen werden: {err}"))?;

    let rows = stmt
        .query_map([], row_to_video)
        .map_err(|err| format!("Videos konnten nicht gelesen werden: {err}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("Video konnte nicht gelesen werden: {err}"))
}

pub fn get_video(paths: &AppPaths, id: i64) -> AppResult<Option<Video>> {
    let conn = open_db(paths)?;
    conn.query_row(
        &format!("SELECT {VIDEO_COLUMNS} FROM videos WHERE id = ?1"),
        params![id],
        row_to_video,
    )
    .optional()
    .map_err(|err| format!("Video konnte nicht geladen werden: {err}"))
}

pub fn delete_video(paths: &AppPaths, id: i64) -> AppResult<()> {
    let conn = open_db(paths)?;
    conn.execute("DELETE FROM videos WHERE id = ?1", params![id])
        .map_err(|err| format!("Video konnte nicht gelöscht werden: {err}"))?;
    Ok(())
}

pub fn update_summary(
    paths: &AppPaths,
    id: i64,
    summary: &str,
    provider: Option<&str>,
    model: Option<&str>,
) -> AppResult<Video> {
    let conn = open_db(paths)?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE videos SET summary = ?1, summary_provider = ?2, summary_model = ?3, updated_at = ?4 WHERE id = ?5",
        params![summary, provider, model, now, id],
    )
    .map_err(|err| format!("Zusammenfassung konnte nicht gespeichert werden: {err}"))?;
    get_video(paths, id)?.ok_or_else(|| "Video nicht gefunden".to_string())
}

pub fn update_transcript(
    paths: &AppPaths,
    id: i64,
    transcript: &str,
    chapters: Option<&str>,
) -> AppResult<Video> {
    let conn = open_db(paths)?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE videos SET transcript = ?1, chapters = ?2, updated_at = ?3 WHERE id = ?4",
        params![transcript, chapters, now, id],
    )
    .map_err(|err| format!("Transkript konnte nicht gespeichert werden: {err}"))?;
    get_video(paths, id)?.ok_or_else(|| "Video nicht gefunden".to_string())
}

fn row_to_video(row: &Row<'_>) -> rusqlite::Result<Video> {
    let thumbnail_data: Option<Vec<u8>> = row.get("thumbnail_data")?;
    let chapters_raw: Option<String> = row.get("chapters")?;
    let thumbnail =
        thumbnail_data.map(|bytes| format!("data:image/jpeg;base64,{}", BASE64.encode(bytes)));
    let chapters = chapters_raw
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Vec<Chapter>>(raw).ok());

    Ok(Video {
        id: row.get("id")?,
        video_id: row.get("video_id")?,
        url: row.get("url")?,
        title: row.get("title")?,
        thumbnail_url: row.get("thumbnail_url")?,
        thumbnail,
        transcript: row.get("transcript")?,
        chapters,
        summary: row.get("summary")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        summary_provider: row.get("summary_provider")?,
        summary_model: row.get("summary_model")?,
        published_at: row.get("published_at")?,
    })
}
