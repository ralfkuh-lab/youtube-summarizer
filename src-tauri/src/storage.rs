use std::fs;
use std::path::PathBuf;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::models::{AppConfig, Chapter, NewVideo, Video};

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
