//! SQLite-Datenhaltung: Schema, Echo-Zähler, Datensatz-ID, Pull und Backup.
//!
//! Lebende Existenzen stehen in je einer Tabelle, Grabsteine und Aliase aller
//! Entitäten in `gone`. `seqs` hält je Schlüssel genau eine Server-`seq`; der
//! Zustand wird beim Pull aus dem aktuellen Bestand gerendert.

use crate::merge::video_data;
use crate::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use sync_proto::{GoneReason, PullResponse, ServerState, State};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value NOT NULL);
-- AUTOINCREMENT: Geräte-IDs entscheiden LWW-Gleichstände und werden nie
-- wiederverwendet, auch nicht nach einem Widerruf.
CREATE TABLE IF NOT EXISTS devices (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL UNIQUE,
    token_hash TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS seqs (
    entity TEXT NOT NULL,
    key    TEXT NOT NULL,
    seq    INTEGER NOT NULL UNIQUE,
    PRIMARY KEY (entity, key)
);
CREATE TABLE IF NOT EXISTS gone (
    entity      TEXT NOT NULL,
    uid         TEXT NOT NULL,
    reason      TEXT NOT NULL,
    merged_into TEXT,
    PRIMARY KEY (entity, uid)
);
CREATE INDEX IF NOT EXISTS gone_merged_into ON gone (entity, merged_into);
CREATE TABLE IF NOT EXISTS videos (
    uid        TEXT PRIMARY KEY,
    youtube_id TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    fields     TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS summaries (
    uid       TEXT PRIMARY KEY,
    video_uid TEXT NOT NULL,
    data      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS summaries_video ON summaries (video_uid);
CREATE TABLE IF NOT EXISTS chats (
    uid        TEXT PRIMARY KEY,
    video_uid  TEXT NOT NULL,
    changed_at TEXT NOT NULL,
    dev        INTEGER NOT NULL,
    data       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS chats_video ON chats (video_uid);
CREATE TABLE IF NOT EXISTS rounds (
    uid       TEXT PRIMARY KEY,
    chat_uid  TEXT NOT NULL,
    video_uid TEXT NOT NULL,
    data      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS rounds_chat ON rounds (chat_uid);
CREATE INDEX IF NOT EXISTS rounds_video ON rounds (video_uid);
CREATE TABLE IF NOT EXISTS collections (
    uid        TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    name_key   TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    changed_at TEXT NOT NULL,
    dev        INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS memberships (
    video_uid      TEXT NOT NULL,
    collection_uid TEXT NOT NULL,
    present        INTEGER NOT NULL,
    changed_at     TEXT NOT NULL,
    dev            INTEGER NOT NULL,
    PRIMARY KEY (video_uid, collection_uid)
);
CREATE INDEX IF NOT EXISTS memberships_collection ON memberships (collection_uid);
INSERT OR IGNORE INTO meta (key, value) VALUES ('seq', 0);
";

/// Öffnet die Datenbank und legt Schema und Datensatz-ID bei Bedarf an.
pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch(SCHEMA)?;
    conn.execute(
        "INSERT OR IGNORE INTO meta (key, value) VALUES ('dataset_id', ?1)",
        [random_hex(16)],
    )?;
    Ok(conn)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Hex-Text aus `bytes` Zufallsbytes des Betriebssystems.
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::getrandom(&mut buf).expect("Zufallsquelle des Betriebssystems nicht verfügbar");
    hex(&buf)
}

fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// Legt ein Gerät an und gibt sein Token zurück; gespeichert wird nur der
/// SHA-256-Hash.
pub fn add_device(conn: &Connection, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Malformed("Gerätename ist leer".into()));
    }
    let token = random_hex(32);
    conn.execute(
        "INSERT INTO devices (name, token_hash, created_at) VALUES (?1, ?2, ?3)",
        params![name, token_hash(&token), sync_proto::now_canonical()],
    )
    .map_err(|err| match err {
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Error::Malformed(format!("Gerät '{name}' existiert bereits"))
        }
        other => Error::Db(other),
    })?;
    Ok(token)
}

/// `(id, name, created_at)` aller Geräte.
pub fn list_devices(conn: &Connection) -> Result<Vec<(i64, String, String)>> {
    let mut stmt = conn.prepare("SELECT id, name, created_at FROM devices ORDER BY id")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Widerruft ein Gerät; `false`, wenn es den Namen nicht gibt.
pub fn revoke_device(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn.execute("DELETE FROM devices WHERE name = ?1", [name.trim()])? > 0)
}

/// Geräte-ID zum Token, gesucht über den Hash-Index.
pub(crate) fn authenticate(conn: &Connection, token: &str) -> Result<i64> {
    conn.query_row(
        "SELECT id FROM devices WHERE token_hash = ?1",
        [token_hash(token)],
        |row| row.get(0),
    )
    .optional()?
    .ok_or(Error::Unauthorized)
}

pub fn dataset_id(conn: &Connection) -> Result<String> {
    Ok(conn.query_row(
        "SELECT value FROM meta WHERE key = 'dataset_id'",
        [],
        |row| row.get(0),
    )?)
}

/// Neue Datensatz-ID (nach einem Restore Pflicht).
pub fn rotate_dataset(conn: &Connection) -> Result<String> {
    let id = random_hex(16);
    conn.execute("UPDATE meta SET value = ?1 WHERE key = 'dataset_id'", [&id])?;
    Ok(id)
}

/// Prüft `X-Sync-Dataset`; muss in der Transaktion der Operation laufen.
pub(crate) fn check_dataset(conn: &Connection, expected: &str) -> Result<()> {
    let current = dataset_id(conn)?;
    if current == expected {
        Ok(())
    } else {
        Err(Error::DatasetMismatch(current))
    }
}

pub(crate) fn current_seq(conn: &Connection) -> Result<i64> {
    Ok(
        conn.query_row("SELECT value FROM meta WHERE key = 'seq'", [], |row| {
            row.get(0)
        })?,
    )
}

/// Gibt dem Schlüssel eine neue, höchste `seq`.
pub(crate) fn echo(conn: &Connection, entity: &str, key: &str) -> Result<()> {
    let seq: i64 = conn.query_row(
        "UPDATE meta SET value = value + 1 WHERE key = 'seq' RETURNING value",
        [],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO seqs (entity, key, seq) VALUES (?1, ?2, ?3)
         ON CONFLICT (entity, key) DO UPDATE SET seq = excluded.seq",
        params![entity, key, seq],
    )?;
    Ok(())
}

pub(crate) fn reason_name(reason: GoneReason) -> &'static str {
    match reason {
        GoneReason::Deleted => "deleted",
        GoneReason::Withdrawn => "withdrawn",
        GoneReason::Merged => "merged",
    }
}

/// Grabstein einer uid: Grund und bei `merged` die Wurzel.
pub(crate) fn gone(
    conn: &Connection,
    entity: &str,
    uid: &str,
) -> Result<Option<(GoneReason, Option<String>)>> {
    let row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT reason, merged_into FROM gone WHERE entity = ?1 AND uid = ?2",
            params![entity, uid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(reason, merged_into)| {
        let reason = match reason.as_str() {
            "deleted" => GoneReason::Deleted,
            "withdrawn" => GoneReason::Withdrawn,
            "merged" => GoneReason::Merged,
            other => return Err(Error::Corrupt(format!("Grabstein-Grund {other}"))),
        };
        Ok((reason, merged_into))
    })
    .transpose()
}

/// Aktueller Zustand eines Schlüssels; `None`, wenn er keinen hat.
pub(crate) fn render(conn: &Connection, entity: &str, key: &str) -> Result<Option<State>> {
    if entity == "membership" {
        let Some((video_uid, collection_uid)) = key.split_once('/') else {
            return Err(Error::Corrupt(format!("Zuordnungsschlüssel {key}")));
        };
        let present: Option<bool> = conn
            .query_row(
                "SELECT present FROM memberships WHERE video_uid = ?1 AND collection_uid = ?2",
                [video_uid, collection_uid],
                |row| row.get(0),
            )
            .optional()?;
        return Ok(present.map(|present| State::Membership {
            video_uid: video_uid.to_owned(),
            collection_uid: collection_uid.to_owned(),
            present,
        }));
    }
    if let Some((reason, merged_into)) = gone(conn, entity, key)? {
        let uid = key.to_owned();
        return Ok(Some(match entity {
            "video" => State::VideoGone {
                uid,
                reason,
                merged_into,
            },
            "summary" => State::SummaryGone { uid },
            "chat" => State::ChatGone { uid },
            "collection" => State::CollectionGone {
                uid,
                reason,
                merged_into,
            },
            _ => return Err(Error::Corrupt(format!("Grabstein für {entity}"))),
        }));
    }
    let data = |sql: &str| -> Result<Option<String>> {
        Ok(conn.query_row(sql, [key], |row| row.get(0)).optional()?)
    };
    Ok(match entity {
        "video" => conn
            .query_row(
                "SELECT youtube_id, created_at, fields FROM videos WHERE uid = ?1",
                [key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .map(
                |(youtube_id, created_at, fields): (String, String, String)| {
                    video_data(key, &youtube_id, &created_at, &fields).map(State::Video)
                },
            )
            .transpose()?,
        "summary" => data("SELECT data FROM summaries WHERE uid = ?1")?
            .map(|json| serde_json::from_str(&json).map(State::Summary))
            .transpose()?,
        "chat" => data("SELECT data FROM chats WHERE uid = ?1")?
            .map(|json| serde_json::from_str(&json).map(State::Chat))
            .transpose()?,
        "round" => data("SELECT data FROM rounds WHERE uid = ?1")?
            .map(|json| serde_json::from_str(&json).map(State::Round))
            .transpose()?,
        "collection" => conn
            .query_row(
                "SELECT name, created_at FROM collections WHERE uid = ?1",
                [key],
                |row| {
                    Ok(State::Collection(sync_proto::CollectionData {
                        uid: key.to_owned(),
                        name: row.get(0)?,
                        created_at: row.get(1)?,
                    }))
                },
            )
            .optional()?,
        _ => return Err(Error::Corrupt(format!("Entität {entity}"))),
    })
}

/// Eine Pull-Seite: Zustände mit `seq > since` nach `seq`, höchstens
/// `max_states` und etwa `max_bytes`, mindestens einer. Prüft Datensatz und
/// Cursor in derselben Lesetransaktion.
pub fn pull_page(
    conn: &mut Connection,
    dataset: &str,
    since: i64,
    max_states: usize,
    max_bytes: usize,
) -> Result<PullResponse> {
    let tx = conn.transaction()?;
    check_dataset(&tx, dataset)?;
    if since > current_seq(&tx)? {
        return Err(Error::CursorAhead);
    }
    let mut states: Vec<ServerState> = Vec::new();
    let mut bytes = 0;
    let mut more = false;
    {
        let mut stmt =
            tx.prepare("SELECT entity, key, seq FROM seqs WHERE seq > ?1 ORDER BY seq")?;
        let mut rows = stmt.query([since])?;
        while let Some(row) = rows.next()? {
            if states.len() == max_states {
                more = true;
                break;
            }
            let (entity, key, seq): (String, String, i64) = (row.get(0)?, row.get(1)?, row.get(2)?);
            let state = render(&tx, &entity, &key)?
                .ok_or_else(|| Error::Corrupt(format!("kein Zustand für {entity}/{key}")))?;
            let item = ServerState { seq, state };
            let size = serde_json::to_vec(&item)?.len();
            if !states.is_empty() && bytes + size > max_bytes {
                more = true;
                break;
            }
            bytes += size;
            states.push(item);
        }
    }
    let next = states.last().map_or(since, |item| item.seq);
    Ok(PullResponse { states, next, more })
}

/// Tägliches Backup nach `<dir>/<today>.db`. Entfernt vorher alte `.tmp`,
/// tut nichts, wenn die heutige Datei existiert, und löscht erst nach Erfolg
/// die ältesten über 7 hinaus. `Ok(true)` = neue Datei geschrieben.
pub fn backup(conn: &Connection, dir: &Path, today: &str) -> Result<bool> {
    let mut backups: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        match name.strip_suffix(".tmp") {
            Some(base) if is_backup_name(base) => fs::remove_file(&path)?,
            _ if is_backup_name(name) => backups.push(path),
            _ => {}
        }
    }
    let target = dir.join(format!("{today}.db"));
    if target.exists() {
        return Ok(false);
    }
    let tmp = dir.join(format!("{today}.db.tmp"));
    let tmp_name = tmp
        .to_str()
        .ok_or_else(|| Error::Corrupt(format!("Backup-Pfad nicht UTF-8: {}", tmp.display())))?;
    conn.execute("VACUUM INTO ?1", [tmp_name])?;
    fs::rename(&tmp, &target)?;
    backups.push(target);
    backups.sort();
    let excess = backups.len().saturating_sub(7);
    for old in &backups[..excess] {
        fs::remove_file(old)?;
    }
    Ok(true)
}

/// `backup` mit Protokoll statt Fehler: der Dienst läuft in jedem Fall weiter.
pub fn backup_logged(conn: &Connection, dir: &Path, today: &str) {
    match backup(conn, dir, today) {
        Ok(true) => eprintln!("Backup {today} geschrieben"),
        Ok(false) => {}
        Err(err) => eprintln!("Backup {today} fehlgeschlagen: {err}"),
    }
}

/// `YYYY-MM-DD.db` mit gültigem Datum; schützt fremde Dateien im
/// Backup-Verzeichnis vor Aufbewahrung und Aufräumen.
fn is_backup_name(name: &str) -> bool {
    name.strip_suffix(".db").is_some_and(|date| {
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .is_ok_and(|day| day.format("%Y-%m-%d").to_string() == date)
    })
}
