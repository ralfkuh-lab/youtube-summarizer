use std::fs;
use std::path::PathBuf;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::models::{AiConfig, AppConfig, Chapter, NewVideo, Video};

pub type AppResult<T> = Result<T, String>;

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
    Ok(load_config(paths)?.ai)
}

pub fn update_ai_config(paths: &AppPaths, ai: AiConfig) -> AppResult<AiConfig> {
    let mut config = load_config(paths)?;
    config.ai = ai.clone();
    save_config(paths, &config)?;
    Ok(ai)
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
            transcript, chapters, summary, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?8)
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
        ],
    )
    .map_err(|err| format!("Video konnte nicht gespeichert werden: {err}"))?;

    get_video(paths, conn.last_insert_rowid())?
        .ok_or_else(|| "Gespeichertes Video wurde nicht gefunden".to_string())
}

pub fn get_videos(paths: &AppPaths) -> AppResult<Vec<Video>> {
    let conn = open_db(paths)?;
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, video_id, url, title, thumbnail_url, thumbnail_data,
                   transcript, chapters, summary, created_at, updated_at
            FROM videos
            ORDER BY created_at DESC
            "#,
        )
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
        r#"
        SELECT id, video_id, url, title, thumbnail_url, thumbnail_data,
               transcript, chapters, summary, created_at, updated_at
        FROM videos
        WHERE id = ?1
        "#,
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

pub fn update_summary(paths: &AppPaths, id: i64, summary: &str) -> AppResult<Video> {
    let conn = open_db(paths)?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE videos SET summary = ?1, updated_at = ?2 WHERE id = ?3",
        params![summary, now, id],
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
    let thumbnail_data: Option<Vec<u8>> = row.get(5)?;
    let chapters_raw: Option<String> = row.get(7)?;
    let thumbnail =
        thumbnail_data.map(|bytes| format!("data:image/jpeg;base64,{}", BASE64.encode(bytes)));
    let chapters = chapters_raw
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Vec<Chapter>>(raw).ok());

    Ok(Video {
        id: row.get(0)?,
        video_id: row.get(1)?,
        url: row.get(2)?,
        title: row.get(3)?,
        thumbnail_url: row.get(4)?,
        thumbnail,
        transcript: row.get(6)?,
        chapters,
        summary: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}
