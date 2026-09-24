//! Referenzfälle des Client-Datenmodells aus `docs/spec-sync.md` (temporäre
//! DB). Migration: C1, C19 (Migrationsteil), C21, C22; Outbox und
//! Privatisierung: C2–C8; Transaktionen und Sortierungen: C18, C23; Push:
//! C9, C10; Apply: C11–C17, C19 (Apply-Teil), C20.

mod apply_tests;
mod e2e_early_tests;
mod e2e_late_tests;
mod e2e_support;
mod engine_tests;
mod merge_tests;
mod migration_tests;
mod outbox_tests;
mod push_tests;
mod storage_tests;

use rusqlite::{params, Connection};
use tempfile::TempDir;

use sync_proto::{
    ChatData, CollectionData, ContextOptions, GoneReason, Op, PullResponse, RoundData,
    RoundMessage, ServerState, State, SummaryData, VideoData,
};

use crate::models::NewVideo;
use crate::storage::{self, AppPaths};
use crate::sync::apply::{self, Applied};
use crate::sync::pull;
use crate::sync::snapshot::{self, Pending};

pub(crate) fn temp_paths() -> (TempDir, AppPaths) {
    let temp = TempDir::new().unwrap();
    let paths = AppPaths {
        db_path: temp.path().join("videos.db"),
        config_path: temp.path().join("config.json"),
    };
    storage::init_db(&paths).unwrap();
    (temp, paths)
}

pub(crate) fn open(paths: &AppPaths) -> Connection {
    storage::open_db(paths).unwrap()
}

pub(crate) fn sample_video(video_id: &str) -> NewVideo {
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

/// Outbox-Eintrag ohne `seq`/`changed_at`: `(entity, key, owner, parent, tomb)`.
pub(crate) type Entry = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Outbox in `seq`-Reihenfolge.
pub(crate) fn outbox(conn: &Connection) -> Vec<Entry> {
    let mut stmt = conn
        .prepare("SELECT entity, key, owner, parent, tomb FROM sync_outbox ORDER BY seq")
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap();
    rows.collect::<Result<_, _>>().unwrap()
}

/// `(entity, key)` der Outbox, sortiert (Reihenfolge egal).
pub(crate) fn outbox_keys(conn: &Connection) -> Vec<(String, String)> {
    let mut keys: Vec<(String, String)> = outbox(conn)
        .into_iter()
        .map(|(entity, key, ..)| (entity, key))
        .collect();
    keys.sort();
    keys
}

pub(crate) fn clear_outbox(conn: &Connection) {
    conn.execute("DELETE FROM sync_outbox", []).unwrap();
}

pub(crate) fn text(conn: &Connection, sql: &str, value: impl rusqlite::ToSql) -> String {
    conn.query_row(sql, params![value], |row| row.get(0))
        .unwrap()
}

pub(crate) fn strings(conn: &Connection, sql: &str, value: impl rusqlite::ToSql) -> Vec<String> {
    let mut stmt = conn.prepare(sql).unwrap();
    let rows = stmt.query_map(params![value], |row| row.get(0)).unwrap();
    rows.collect::<Result<_, _>>().unwrap()
}

pub(crate) fn collection_uid(conn: &Connection, id: i64) -> String {
    text(conn, "SELECT uid FROM collections WHERE id = ?1", id)
}

pub(crate) fn video_uid(conn: &Connection, id: i64) -> String {
    text(conn, "SELECT uid FROM videos WHERE id = ?1", id)
}

/// Vollständiger Abzug der Datenbank (Schema und alle Zeilen aller Tabellen
/// einschließlich `sqlite_sequence`), um „unverändert“ zu prüfen.
pub(crate) fn dump(conn: &Connection) -> Vec<String> {
    let mut lines = Vec::new();
    let mut tables = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT type, name, sql FROM sqlite_master ORDER BY type, name")
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .unwrap();
        for row in rows {
            let (kind, name, sql) = row.unwrap();
            if kind == "table" {
                tables.push(name.clone());
            }
            lines.push(format!("{kind} {name}: {sql:?}"));
        }
    }
    for table in tables {
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM \"{table}\" ORDER BY rowid"))
            .unwrap();
        let columns = stmt.column_count();
        let rows = stmt
            .query_map([], |row| {
                (0..columns)
                    .map(|index| row.get::<_, rusqlite::types::Value>(index))
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap();
        for row in rows {
            lines.push(format!("{table}: {:?}", row.unwrap()));
        }
    }
    lines
}

/// uid aus einer Zahl (32 Hex-Zeichen).
pub(crate) fn uid(n: u32) -> String {
    format!("{n:032x}")
}

pub(crate) const T0: &str = "2026-09-24T10:00:00.000Z";

pub(crate) fn video_state(uid: &str, youtube_id: &str, title: &str) -> State {
    State::Video(VideoData {
        uid: uid.into(),
        youtube_id: youtube_id.into(),
        url: format!("https://www.youtube.com/watch?v={youtube_id}"),
        title: title.into(),
        thumbnail_url: "https://example.com/t.jpg".into(),
        thumbnail_data: None,
        transcript: Some("[]".into()),
        chapters: None,
        published_at: None,
        description: None,
        transcript_error: None,
        created_at: T0.into(),
    })
}

pub(crate) fn summary_state(uid: &str, video_uid: &str, created_at: &str, text: &str) -> State {
    State::Summary(SummaryData {
        uid: uid.into(),
        video_uid: video_uid.into(),
        created_at: created_at.into(),
        summary: text.into(),
        provider: None,
        model: None,
        options: None,
    })
}

pub(crate) fn chat_state(uid: &str, video_uid: &str, summary_uids: Option<Vec<String>>) -> State {
    State::Chat(ChatData {
        uid: uid.into(),
        video_uid: video_uid.into(),
        title: "Chat".into(),
        created_at: T0.into(),
        updated_at: T0.into(),
        context_options: ContextOptions {
            transcript: true,
            summary_uids,
        },
    })
}

pub(crate) fn round_state(uid: &str, chat_uid: &str, video_uid: &str, text: &str) -> State {
    State::Round(RoundData {
        uid: uid.into(),
        chat_uid: chat_uid.into(),
        video_uid: video_uid.into(),
        created_at: T0.into(),
        messages: vec![RoundMessage {
            role: "user".into(),
            content: text.into(),
            tool_calls: None,
            tool_call_id: None,
            provider: None,
            model: None,
        }],
    })
}

pub(crate) fn collection_state(uid: &str, name: &str) -> State {
    State::Collection(CollectionData {
        uid: uid.into(),
        name: name.into(),
        created_at: T0.into(),
    })
}

pub(crate) fn membership_state(video_uid: &str, collection_uid: &str, present: bool) -> State {
    State::Membership {
        video_uid: video_uid.into(),
        collection_uid: collection_uid.into(),
        present,
    }
}

pub(crate) fn gone(uid: &str, reason: GoneReason, merged_into: Option<&str>) -> State {
    State::VideoGone {
        uid: uid.into(),
        reason,
        merged_into: merged_into.map(str::to_string),
    }
}

/// Server ohne Zusammenführung: macht aus gesendeten Operationen die
/// Zustände, die ein anderes Gerät beim Pull bekäme.
pub(crate) fn as_states(ops: &[Pending]) -> Vec<State> {
    ops.iter()
        .map(|pending| match pending.op.clone() {
            Op::Video { data, .. } => State::Video(data),
            Op::VideoGone { uid, reason } => State::VideoGone {
                uid,
                reason,
                merged_into: None,
            },
            Op::Summary(data) => State::Summary(data),
            Op::SummaryDelete { uid, .. } => State::SummaryGone { uid },
            Op::Chat { data, .. } => State::Chat(data),
            Op::ChatDelete { uid, .. } => State::ChatGone { uid },
            Op::Round(data) => State::Round(data),
            Op::Collection { data, .. } => State::Collection(data),
            Op::CollectionDelete { uid } => State::CollectionGone {
                uid,
                reason: GoneReason::Deleted,
                merged_into: None,
            },
            Op::Membership {
                video_uid,
                collection_uid,
                present,
                ..
            } => State::Membership {
                video_uid,
                collection_uid,
                present,
            },
        })
        .collect()
}

/// Legt Zustände als eine Pull-Seite ab (`seq` fortlaufend ab dem Cursor).
pub(crate) fn stage(conn: &mut Connection, states: Vec<State>) {
    let start = pull::cursor(conn).unwrap();
    let states: Vec<ServerState> = states
        .into_iter()
        .enumerate()
        .map(|(index, state)| ServerState {
            seq: start + 1 + index as i64,
            state,
        })
        .collect();
    let next = states.last().map_or(start, |state| state.seq);
    pull::stage_page(
        conn,
        &PullResponse {
            states,
            next,
            more: false,
        },
    )
    .unwrap();
}

pub(crate) fn apply_states(conn: &mut Connection, states: Vec<State>) -> Applied {
    stage(conn, states);
    apply::apply(conn).unwrap()
}

/// Snapshot und alles als `ok` quittiert (wie ein erfolgreicher Push).
pub(crate) fn push_all(conn: &mut Connection) -> Vec<Pending> {
    let snapshot = snapshot::snapshot(conn).unwrap();
    let sent: Vec<Pending> = snapshot
        .deletes
        .into_iter()
        .chain(snapshot.upserts)
        .collect();
    let ok = sync_proto::OpResult {
        status: sync_proto::OpStatus::Ok,
        reason: None,
        missing: None,
    };
    snapshot::acknowledge(conn, &sent, &vec![ok; sent.len()]).unwrap();
    sent
}

pub(crate) fn inbox_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM sync_inbox", [], |row| row.get(0))
        .unwrap()
}
