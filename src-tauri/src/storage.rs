use std::path::PathBuf;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::models::{Chapter, Collection, NewVideo, Summary, Video};

pub type AppResult<T> = Result<T, String>;

const VIDEO_COLUMNS: &str = r#"
    id, video_id, url, title, thumbnail_url, thumbnail_data,
    transcript, chapters, summary, summary_provider, summary_model,
    published_at, description, created_at, updated_at, transcript_error
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
            description TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            transcript_error TEXT
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

        CREATE TABLE IF NOT EXISTS summaries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            video_id INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            summary TEXT NOT NULL,
            provider TEXT,
            model TEXT,
            options TEXT,
            FOREIGN KEY (video_id) REFERENCES videos(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_summaries_video_id ON summaries(video_id);
        "#,
    )
    .map_err(|err| format!("Datenbank konnte nicht initialisiert werden: {err}"))?;
    ensure_video_column(&conn, "description", "TEXT")?;
    ensure_video_column(&conn, "transcript_error", "TEXT")?;
    backfill_legacy_summaries(&conn)?;
    Ok(())
}

/// Adds a column to an existing videos table; CREATE TABLE IF NOT EXISTS
/// only covers fresh databases.
fn ensure_video_column(conn: &Connection, name: &str, column_type: &str) -> AppResult<()> {
    let exists = conn
        .prepare("SELECT 1 FROM pragma_table_info('videos') WHERE name = ?1")
        .and_then(|mut stmt| stmt.exists(params![name]))
        .map_err(|err| format!("Videotabelle konnte nicht geprüft werden: {err}"))?;
    if !exists {
        conn.execute(
            &format!("ALTER TABLE videos ADD COLUMN {name} {column_type}"),
            [],
        )
        .map_err(|err| format!("Spalte {name} konnte nicht ergänzt werden: {err}"))?;
    }
    Ok(())
}

fn backfill_legacy_summaries(conn: &Connection) -> AppResult<()> {
    conn.execute(
        r#"
        INSERT INTO summaries (video_id, created_at, summary, provider, model, options)
        SELECT
            v.id,
            COALESCE(NULLIF(v.updated_at, ''), v.created_at),
            v.summary,
            v.summary_provider,
            v.summary_model,
            NULL
        FROM videos v
        WHERE v.summary IS NOT NULL
          AND TRIM(v.summary) != ''
          AND NOT EXISTS (SELECT 1 FROM summaries s WHERE s.video_id = v.id)
        "#,
        [],
    )
    .map_err(|err| format!("Zusammenfassungshistorie konnte nicht nachgezogen werden: {err}"))?;
    Ok(())
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
    let transcript_error = if video.transcript.is_some() {
        None
    } else {
        video.transcript_error
    };
    conn.execute(
        r#"
        INSERT INTO videos (
            video_id, url, title, thumbnail_url, thumbnail_data,
            transcript, chapters, summary, created_at, updated_at, published_at, description,
            transcript_error
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?8, ?9, ?10, ?11)
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
            video.description,
            transcript_error,
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
    options: Option<&str>,
) -> AppResult<Video> {
    let mut conn = open_db(paths)?;
    let now = Utc::now().to_rfc3339();
    let tx = conn
        .transaction()
        .map_err(|err| format!("Zusammenfassung konnte nicht gespeichert werden: {err}"))?;
    let changed = tx
        .execute(
            "UPDATE videos SET summary = ?1, summary_provider = ?2, summary_model = ?3, updated_at = ?4 WHERE id = ?5",
            params![summary, provider, model, now, id],
        )
        .map_err(|err| format!("Zusammenfassung konnte nicht gespeichert werden: {err}"))?;
    if changed == 0 {
        return Err("Video nicht gefunden".to_string());
    }
    tx.execute(
        "INSERT INTO summaries (video_id, created_at, summary, provider, model, options) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, now, summary, provider, model, options],
    )
    .map_err(|err| format!("Zusammenfassungshistorie konnte nicht gespeichert werden: {err}"))?;
    tx.commit()
        .map_err(|err| format!("Zusammenfassung konnte nicht gespeichert werden: {err}"))?;
    get_video(paths, id)?.ok_or_else(|| "Video nicht gefunden".to_string())
}

pub fn get_summaries(paths: &AppPaths, video_id: i64) -> AppResult<Vec<Summary>> {
    let conn = open_db(paths)?;
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, video_id, created_at, summary, provider, model, options
            FROM summaries
            WHERE video_id = ?1
            ORDER BY created_at DESC, id DESC
            "#,
        )
        .map_err(|err| format!("Zusammenfassungen konnten nicht geladen werden: {err}"))?;
    let rows = stmt
        .query_map(params![video_id], row_to_summary)
        .map_err(|err| format!("Zusammenfassungen konnten nicht gelesen werden: {err}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("Zusammenfassung konnte nicht gelesen werden: {err}"))
}

pub fn delete_summary(paths: &AppPaths, id: i64) -> AppResult<()> {
    let mut conn = open_db(paths)?;
    let tx = conn
        .transaction()
        .map_err(|err| format!("Zusammenfassung konnte nicht gelöscht werden: {err}"))?;
    let video_id: i64 = tx
        .query_row(
            "SELECT video_id FROM summaries WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| format!("Zusammenfassung konnte nicht gelöscht werden: {err}"))?
        .ok_or_else(|| "Zusammenfassung nicht gefunden".to_string())?;
    tx.execute("DELETE FROM summaries WHERE id = ?1", params![id])
        .map_err(|err| format!("Zusammenfassung konnte nicht gelöscht werden: {err}"))?;

    let newest = tx
        .query_row(
            r#"
            SELECT summary, provider, model
            FROM summaries
            WHERE video_id = ?1
            ORDER BY created_at DESC, id DESC
            LIMIT 1
            "#,
            params![video_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|err| format!("Zusammenfassung konnte nicht gelöscht werden: {err}"))?;

    let now = Utc::now().to_rfc3339();
    match newest {
        Some((summary, provider, model)) => {
            tx.execute(
                "UPDATE videos SET summary = ?1, summary_provider = ?2, summary_model = ?3, updated_at = ?4 WHERE id = ?5",
                params![summary, provider, model, now, video_id],
            )
        }
        None => tx.execute(
            "UPDATE videos SET summary = NULL, summary_provider = NULL, summary_model = NULL, updated_at = ?1 WHERE id = ?2",
            params![now, video_id],
        ),
    }
    .map_err(|err| format!("Zusammenfassung konnte nicht aktualisiert werden: {err}"))?;
    tx.commit()
        .map_err(|err| format!("Zusammenfassung konnte nicht gelöscht werden: {err}"))?;
    Ok(())
}

pub fn update_transcript(
    paths: &AppPaths,
    id: i64,
    transcript: &str,
    chapters: Option<&str>,
    description: Option<&str>,
) -> AppResult<Video> {
    let conn = open_db(paths)?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE videos SET transcript = ?1, chapters = ?2, description = ?3, transcript_error = NULL, updated_at = ?4 WHERE id = ?5",
        params![transcript, chapters, description, now, id],
    )
    .map_err(|err| format!("Transkript konnte nicht gespeichert werden: {err}"))?;
    get_video(paths, id)?.ok_or_else(|| "Video nicht gefunden".to_string())
}

pub fn set_transcript_error(paths: &AppPaths, id: i64, error: &str) -> AppResult<Video> {
    let conn = open_db(paths)?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE videos SET transcript_error = ?1, updated_at = ?2 WHERE id = ?3 AND transcript IS NULL",
        params![error, now, id],
    )
    .map_err(|err| format!("Transkript-Fehler konnte nicht gespeichert werden: {err}"))?;
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
        description: row.get("description")?,
        collection_ids: Vec::new(),
        transcript_error: row.get("transcript_error")?,
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

fn row_to_summary(row: &Row<'_>) -> rusqlite::Result<Summary> {
    Ok(Summary {
        id: row.get("id")?,
        video_id: row.get("video_id")?,
        created_at: row.get("created_at")?,
        summary: row.get("summary")?,
        provider: row.get("provider")?,
        model: row.get("model")?,
        options: row.get("options")?,
    })
}

/// Returns a sibling file path next to config.json (e.g. ai.json, auth.json, ai-catalog.json).
pub fn ai_data_file(paths: &AppPaths, name: &str) -> PathBuf {
    let dir = paths
        .config_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    dir.join(name)
}

pub fn ai_json_path(paths: &AppPaths) -> PathBuf {
    ai_data_file(paths, "ai.json")
}

pub fn auth_json_path(paths: &AppPaths) -> PathBuf {
    ai_data_file(paths, "auth.json")
}

pub fn ai_catalog_cache_path(paths: &AppPaths) -> PathBuf {
    ai_data_file(paths, "ai-catalog.json")
}

pub fn summary_presets_path(paths: &AppPaths) -> PathBuf {
    ai_data_file(paths, "summary-presets.json")
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
        init_db(&paths).unwrap();
        (temp, paths)
    }

    fn sample_video(video_id: &str) -> NewVideo {
        NewVideo {
            video_id: video_id.into(),
            url: format!("https://www.youtube.com/watch?v={video_id}"),
            title: "Testvideo".into(),
            thumbnail_url: "https://example.com/t.jpg".into(),
            thumbnail_data: None,
            transcript: Some(r#"[{"text":"hi","start":0.0,"time":"0:00"}]"#.into()),
            chapters: None,
            published_at: None,
            description: None,
            transcript_error: None,
        }
    }

    #[test]
    fn summary_history_inserts_lists_and_deletes() {
        let (_temp, paths) = temp_paths();
        let video = insert_video(&paths, sample_video("abcdefghijk")).unwrap();

        let first = update_summary(
            &paths,
            video.id,
            "Erste Zusammenfassung",
            Some("OpenRouter"),
            Some("glm"),
            Some(r#"{"preset":"standard"}"#),
        )
        .unwrap();
        assert_eq!(first.summary.as_deref(), Some("Erste Zusammenfassung"));
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = update_summary(
            &paths,
            video.id,
            "Zweite Zusammenfassung",
            Some("OpenRouter"),
            Some("flash"),
            None,
        )
        .unwrap();
        assert_eq!(second.summary.as_deref(), Some("Zweite Zusammenfassung"));

        let history = get_summaries(&paths, video.id).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].summary, "Zweite Zusammenfassung");
        assert_eq!(history[0].model.as_deref(), Some("flash"));
        assert_eq!(history[1].summary, "Erste Zusammenfassung");
        assert_eq!(
            history[1].options.as_deref(),
            Some(r#"{"preset":"standard"}"#)
        );

        delete_summary(&paths, history[0].id).unwrap();
        let remaining = get_summaries(&paths, video.id).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].summary, "Erste Zusammenfassung");
        let current = get_video(&paths, video.id).unwrap().unwrap();
        assert_eq!(current.summary.as_deref(), Some("Erste Zusammenfassung"));
        assert_eq!(current.summary_model.as_deref(), Some("glm"));

        delete_summary(&paths, remaining[0].id).unwrap();
        assert!(get_summaries(&paths, video.id).unwrap().is_empty());
        let cleared = get_video(&paths, video.id).unwrap().unwrap();
        assert!(cleared.summary.is_none());
        assert!(cleared.summary_provider.is_none());
        assert!(cleared.summary_model.is_none());
    }

    #[test]
    fn init_db_backfills_legacy_summaries_once() {
        let (_temp, paths) = temp_paths();
        let video = insert_video(&paths, sample_video("cdefghijklm")).unwrap();
        let conn = Connection::open(&paths.db_path).unwrap();
        conn.execute(
            "UPDATE videos SET summary = ?1, summary_provider = ?2, summary_model = ?3, updated_at = ?4 WHERE id = ?5",
            params![
                "Altbestand",
                "OpenRouter",
                "glm",
                "2026-01-02T03:04:05Z",
                video.id
            ],
        )
        .unwrap();
        drop(conn);

        assert!(get_summaries(&paths, video.id).unwrap().is_empty());
        init_db(&paths).unwrap();
        let history = get_summaries(&paths, video.id).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].summary, "Altbestand");
        assert_eq!(history[0].provider.as_deref(), Some("OpenRouter"));
        assert_eq!(history[0].model.as_deref(), Some("glm"));
        assert_eq!(history[0].created_at, "2026-01-02T03:04:05Z");

        init_db(&paths).unwrap();
        assert_eq!(get_summaries(&paths, video.id).unwrap().len(), 1);
    }

    #[test]
    fn delete_video_removes_summary_history() {
        let (_temp, paths) = temp_paths();
        let video = insert_video(&paths, sample_video("bcdefghijkl")).unwrap();
        update_summary(&paths, video.id, "Text", None, None, None).unwrap();
        assert_eq!(get_summaries(&paths, video.id).unwrap().len(), 1);

        delete_video(&paths, video.id).unwrap();
        assert!(get_video(&paths, video.id).unwrap().is_none());
        assert!(get_summaries(&paths, video.id).unwrap().is_empty());
    }

    #[test]
    fn delete_summary_rejects_unknown_id() {
        let (_temp, paths) = temp_paths();
        let error = delete_summary(&paths, 99).unwrap_err();
        assert!(error.contains("nicht gefunden"), "unexpected: {error}");
    }

    #[test]
    fn insert_video_with_transcript_error_persists_error() {
        let (_temp, paths) = temp_paths();
        let mut sample = sample_video("errvideo123");
        sample.transcript = None;
        sample.transcript_error =
            Some("LOGIN_REQUIRED: Sign in to confirm you're not a bot".into());

        let video = insert_video(&paths, sample).unwrap();
        assert!(video.transcript.is_none());
        assert_eq!(
            video.transcript_error.as_deref(),
            Some("LOGIN_REQUIRED: Sign in to confirm you're not a bot")
        );

        let fetched = get_video(&paths, video.id).unwrap().unwrap();
        assert!(fetched.transcript.is_none());
        assert_eq!(
            fetched.transcript_error.as_deref(),
            Some("LOGIN_REQUIRED: Sign in to confirm you're not a bot")
        );
    }

    #[test]
    fn set_transcript_error_then_update_transcript_clears_error() {
        let (_temp, paths) = temp_paths();
        let mut sample = sample_video("refreshvid1");
        sample.transcript = None;
        sample.transcript_error = None;
        let video = insert_video(&paths, sample).unwrap();
        assert_eq!(video.transcript_error, None);

        let updated = set_transcript_error(&paths, video.id, "Netzwerkfehler").unwrap();
        assert_eq!(updated.transcript_error.as_deref(), Some("Netzwerkfehler"));
        assert!(updated.transcript.is_none());

        let after_fetch = get_video(&paths, video.id).unwrap().unwrap();
        assert_eq!(
            after_fetch.transcript_error.as_deref(),
            Some("Netzwerkfehler")
        );

        let with_transcript = update_transcript(
            &paths,
            video.id,
            r#"[{"text":"hallo","start":0.0,"time":"0:00"}]"#,
            None,
            None,
        )
        .unwrap();
        assert!(with_transcript.transcript.is_some());
        assert_eq!(with_transcript.transcript_error, None);

        let final_video = get_video(&paths, video.id).unwrap().unwrap();
        assert!(final_video.transcript.is_some());
        assert_eq!(final_video.transcript_error, None);
    }

    #[test]
    fn migration_adds_transcript_error_column_to_legacy_db() {
        let temp = TempDir::new().unwrap();
        let paths = AppPaths {
            db_path: temp.path().join("videos.db"),
            config_path: temp.path().join("config.json"),
        };
        // Create legacy schema without transcript_error column
        {
            let conn = Connection::open(&paths.db_path).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE videos (
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
                    description TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                "#,
            )
            .unwrap();
            conn.execute(
                r#"
                INSERT INTO videos (
                    video_id, url, title, thumbnail_url, transcript, created_at, updated_at
                ) VALUES ('legacy123', 'https://example.com', 'Old Video', 'https://example.com/t.jpg', NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')
                "#,
                [],
            )
            .unwrap();
        }

        // Running init_db must migrate the table and add transcript_error column
        init_db(&paths).unwrap();

        let videos = get_videos(&paths).unwrap();
        assert_eq!(videos.len(), 1);
        assert_eq!(videos[0].video_id, "legacy123");
        assert_eq!(videos[0].transcript_error, None);
    }

    #[test]
    fn set_transcript_error_does_not_overwrite_existing_transcript() {
        let (_temp, paths) = temp_paths();
        let mut sample = sample_video("racevid1");
        sample.transcript = None;
        sample.transcript_error = None;
        let video = insert_video(&paths, sample).unwrap();

        let with_transcript = update_transcript(
            &paths,
            video.id,
            r#"[{"text":"fertig","start":0.0,"time":"0:00"}]"#,
            None,
            None,
        )
        .unwrap();
        let expected_updated_at = with_transcript.updated_at.clone();

        // Late arrival of error (e.g. parallel race condition)
        let after_late_error =
            set_transcript_error(&paths, video.id, "Später Netzwerkfehler").unwrap();
        assert_eq!(after_late_error.transcript_error, None);
        assert_eq!(
            after_late_error.transcript.as_deref(),
            Some(r#"[{"text":"fertig","start":0.0,"time":"0:00"}]"#)
        );
        assert_eq!(after_late_error.updated_at, expected_updated_at);

        let fetched = get_video(&paths, video.id).unwrap().unwrap();
        assert_eq!(fetched.transcript_error, None);
        assert_eq!(
            fetched.transcript.as_deref(),
            Some(r#"[{"text":"fertig","start":0.0,"time":"0:00"}]"#)
        );
        assert_eq!(fetched.updated_at, expected_updated_at);
    }

    #[test]
    fn insert_video_forces_transcript_error_to_none_when_transcript_is_some() {
        let (_temp, paths) = temp_paths();
        let mut sample = sample_video("bothsome1");
        sample.transcript = Some(r#"[{"text":"vorhanden","start":0.0,"time":"0:00"}]"#.into());
        sample.transcript_error = Some("Widersprüchlicher Fehler".into());

        let video = insert_video(&paths, sample).unwrap();
        assert!(video.transcript.is_some());
        assert_eq!(video.transcript_error, None);

        let fetched = get_video(&paths, video.id).unwrap().unwrap();
        assert!(fetched.transcript.is_some());
        assert_eq!(fetched.transcript_error, None);
    }
}
