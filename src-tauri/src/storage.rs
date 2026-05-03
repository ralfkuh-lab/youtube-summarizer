use std::fs;
use std::path::PathBuf;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::models::{AppConfig, Chapter, Collection, NewVideo, Video};

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
    let conn = Connection::open(&paths.db_path)
        .map_err(|err| format!("Datenbank konnte nicht geöffnet werden: {err}"))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|err| format!("Datenbank konnte nicht konfiguriert werden: {err}"))?;
    Ok(conn)
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

        CREATE TABLE IF NOT EXISTS collections (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_collections_name_nocase
            ON collections(name COLLATE NOCASE);

        CREATE TABLE IF NOT EXISTS video_collections (
            video_id INTEGER NOT NULL,
            collection_id INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (video_id, collection_id),
            FOREIGN KEY (video_id) REFERENCES videos(id) ON DELETE CASCADE,
            FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_video_collections_video_id
            ON video_collections(video_id);
        CREATE INDEX IF NOT EXISTS idx_video_collections_collection_id
            ON video_collections(collection_id);
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

    let mut videos = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("Video konnte nicht gelesen werden: {err}"))?;
    hydrate_video_collections(&conn, &mut videos)?;
    Ok(videos)
}

pub fn get_video(paths: &AppPaths, id: i64) -> AppResult<Option<Video>> {
    let conn = open_db(paths)?;
    let mut video = conn
        .query_row(
            &format!("SELECT {VIDEO_COLUMNS} FROM videos WHERE id = ?1"),
            params![id],
            row_to_video,
        )
        .optional()
        .map_err(|err| format!("Video konnte nicht geladen werden: {err}"))?;
    if let Some(video) = video.as_mut() {
        video.collection_ids = get_video_collection_ids(&conn, id)?;
    }
    Ok(video)
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

pub fn get_collections(paths: &AppPaths) -> AppResult<Vec<Collection>> {
    let conn = open_db(paths)?;
    let mut stmt = conn
        .prepare(
            r#"
            SELECT
                c.id,
                c.name,
                COUNT(vc.video_id) AS video_count,
                c.created_at,
                c.updated_at
            FROM collections c
            LEFT JOIN video_collections vc ON vc.collection_id = c.id
            GROUP BY c.id
            ORDER BY lower(c.name), c.created_at
            "#,
        )
        .map_err(|err| format!("Sammlungen konnten nicht geladen werden: {err}"))?;
    let rows = stmt
        .query_map([], row_to_collection)
        .map_err(|err| format!("Sammlungen konnten nicht gelesen werden: {err}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("Sammlung konnte nicht gelesen werden: {err}"))
}

pub fn create_collection(paths: &AppPaths, name: &str) -> AppResult<Collection> {
    let name = normalize_collection_name(name)?;
    let conn = open_db(paths)?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO collections (name, created_at, updated_at) VALUES (?1, ?2, ?2)",
        params![name, now],
    )
    .map_err(|err| collection_write_error(err, "Sammlung konnte nicht angelegt werden"))?;
    get_collection(&conn, conn.last_insert_rowid())?
        .ok_or_else(|| "Sammlung nicht gefunden".to_string())
}

pub fn update_collection(paths: &AppPaths, id: i64, name: &str) -> AppResult<Collection> {
    let name = normalize_collection_name(name)?;
    let conn = open_db(paths)?;
    let now = Utc::now().to_rfc3339();
    let changed = conn
        .execute(
            "UPDATE collections SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![name, now, id],
        )
        .map_err(|err| collection_write_error(err, "Sammlung konnte nicht umbenannt werden"))?;
    if changed == 0 {
        return Err("Sammlung nicht gefunden".to_string());
    }
    get_collection(&conn, id)?.ok_or_else(|| "Sammlung nicht gefunden".to_string())
}

pub fn delete_collection(paths: &AppPaths, id: i64) -> AppResult<()> {
    let conn = open_db(paths)?;
    let changed = conn
        .execute("DELETE FROM collections WHERE id = ?1", params![id])
        .map_err(|err| format!("Sammlung konnte nicht gelöscht werden: {err}"))?;
    if changed == 0 {
        return Err("Sammlung nicht gefunden".to_string());
    }
    Ok(())
}

pub fn set_video_collections(
    paths: &AppPaths,
    video_id: i64,
    collection_ids: Vec<i64>,
) -> AppResult<Video> {
    let mut conn = open_db(paths)?;
    if get_video(paths, video_id)?.is_none() {
        return Err("Video nicht gefunden".to_string());
    }

    let tx = conn
        .transaction()
        .map_err(|err| format!("Sammlungen konnten nicht gespeichert werden: {err}"))?;
    tx.execute(
        "DELETE FROM video_collections WHERE video_id = ?1",
        params![video_id],
    )
    .map_err(|err| format!("Sammlungen konnten nicht aktualisiert werden: {err}"))?;

    let now = Utc::now().to_rfc3339();
    let mut unique_ids = collection_ids;
    unique_ids.sort_unstable();
    unique_ids.dedup();
    for collection_id in unique_ids {
        tx.execute(
            "INSERT INTO video_collections (video_id, collection_id, created_at) VALUES (?1, ?2, ?3)",
            params![video_id, collection_id, now],
        )
        .map_err(|err| collection_write_error(err, "Sammlung konnte nicht zugewiesen werden"))?;
    }
    tx.commit()
        .map_err(|err| format!("Sammlungen konnten nicht gespeichert werden: {err}"))?;

    get_video(paths, video_id)?.ok_or_else(|| "Video nicht gefunden".to_string())
}

fn get_collection(conn: &Connection, id: i64) -> AppResult<Option<Collection>> {
    conn.query_row(
        r#"
        SELECT
            c.id,
            c.name,
            COUNT(vc.video_id) AS video_count,
            c.created_at,
            c.updated_at
        FROM collections c
        LEFT JOIN video_collections vc ON vc.collection_id = c.id
        WHERE c.id = ?1
        GROUP BY c.id
        "#,
        params![id],
        row_to_collection,
    )
    .optional()
    .map_err(|err| format!("Sammlung konnte nicht geladen werden: {err}"))
}

fn normalize_collection_name(name: &str) -> AppResult<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Sammlungsname darf nicht leer sein".to_string());
    }
    if trimmed.chars().count() > 80 {
        return Err("Sammlungsname darf höchstens 80 Zeichen lang sein".to_string());
    }
    Ok(trimmed.to_string())
}

fn collection_write_error(err: rusqlite::Error, fallback: &str) -> String {
    if let rusqlite::Error::SqliteFailure(error, _) = &err {
        if error.code == rusqlite::ErrorCode::ConstraintViolation {
            return "Sammlung existiert bereits oder ist ungültig".to_string();
        }
    }
    format!("{fallback}: {err}")
}

fn hydrate_video_collections(conn: &Connection, videos: &mut [Video]) -> AppResult<()> {
    for video in videos {
        video.collection_ids = get_video_collection_ids(conn, video.id)?;
    }
    Ok(())
}

fn get_video_collection_ids(conn: &Connection, video_id: i64) -> AppResult<Vec<i64>> {
    let mut stmt = conn
        .prepare(
            "SELECT collection_id FROM video_collections WHERE video_id = ?1 ORDER BY collection_id",
        )
        .map_err(|err| format!("Video-Sammlungen konnten nicht geladen werden: {err}"))?;
    let rows = stmt
        .query_map(params![video_id], |row| row.get::<_, i64>(0))
        .map_err(|err| format!("Video-Sammlungen konnten nicht gelesen werden: {err}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("Video-Sammlung konnte nicht gelesen werden: {err}"))
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
        collection_ids: Vec::new(),
    })
}

fn row_to_collection(row: &Row<'_>) -> rusqlite::Result<Collection> {
    Ok(Collection {
        id: row.get("id")?,
        name: row.get("name")?,
        video_count: row.get("video_count")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}
