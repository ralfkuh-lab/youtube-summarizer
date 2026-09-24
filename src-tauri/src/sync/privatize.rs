//! Privatisieren eines Videos (docs/spec-sync.md, „Privat schalten und
//! freigeben“ und Apply `videoGone withdrawn`).

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use super::{db_error, new_uid, new_uids, outbox};
use crate::storage::AppResult;

/// Macht ein Video privat; gemeinsamer Weg für `video_set_local_only` und
/// Apply. Owner-Einträge der Outbox verschwinden. War das Video
/// veröffentlicht (jede dem Server bekannte uid ist es), bekommt der ganze
/// Teilbaum neue uids, sodass die private Kopie keinen Schlüssel mehr mit
/// geteilten Existenzen teilt. `record_tomb` schreibt dann den
/// `withdrawn`-Grabstein der alten uid (lokales Umschalten; Apply hat den
/// Grabstein schon vom Server).
///
/// `local_only = 1` wird zuerst gesetzt, damit die Kind-Trigger beim
/// Neu-Identifizieren schweigen.
pub(crate) fn make_private(conn: &Connection, video_id: i64, record_tomb: bool) -> AppResult<()> {
    let (uid, published): (String, bool) = conn
        .query_row(
            "SELECT uid, published FROM videos WHERE id = ?1",
            params![video_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(db_error)?
        .ok_or_else(|| "Video nicht gefunden".to_string())?;
    conn.execute(
        "UPDATE videos SET local_only = 1 WHERE id = ?1",
        params![video_id],
    )
    .map_err(db_error)?;
    conn.execute("DELETE FROM sync_outbox WHERE owner = ?1", params![uid])
        .map_err(db_error)?;
    if !published {
        return Ok(());
    }
    if record_tomb {
        outbox::put(conn, "video", &uid, Some(&uid), None, Some("withdrawn"))?;
    }
    reidentify(conn, video_id)?;
    conn.execute(
        "UPDATE videos SET published = 0 WHERE id = ?1",
        params![video_id],
    )
    .map_err(db_error)?;
    Ok(())
}

/// Neue uids für Video, Summaries, Chats und Runden. Summaries und Runden
/// bekommen aufsteigend sortierte uids in ihrer bisherigen Reihenfolge
/// (`created_at`, uid), damit Sortierungen und „neueste Summary“ gleich
/// bleiben. `pendingSummaryUids` der Chats folgen der Zuordnung alt → neu.
fn reidentify(conn: &Connection, video_id: i64) -> AppResult<()> {
    conn.execute(
        "UPDATE videos SET uid = ?1 WHERE id = ?2",
        params![new_uid(conn)?, video_id],
    )
    .map_err(db_error)?;

    let summaries = column(
        conn,
        "SELECT uid FROM summaries WHERE video_id = ?1 ORDER BY created_at, uid",
        video_id,
    )?;
    let fresh = new_uids(conn, summaries.len())?;
    let mut renamed = HashMap::new();
    for (old, new) in summaries.into_iter().zip(fresh) {
        conn.execute(
            "UPDATE summaries SET uid = ?1 WHERE uid = ?2",
            params![new, old],
        )
        .map_err(db_error)?;
        renamed.insert(old, new);
    }

    let rounds = column(
        conn,
        "SELECT m.round_uid FROM chat_messages m JOIN chats c ON c.id = m.chat_id \
         WHERE c.video_id = ?1 GROUP BY m.round_uid ORDER BY MIN(m.created_at), m.round_uid",
        video_id,
    )?;
    let fresh = new_uids(conn, rounds.len())?;
    for (old, new) in rounds.into_iter().zip(fresh) {
        conn.execute(
            "UPDATE chat_messages SET round_uid = ?1 WHERE round_uid = ?2",
            params![new, old],
        )
        .map_err(db_error)?;
    }

    conn.execute(
        "UPDATE chats SET uid = lower(hex(randomblob(16))) WHERE video_id = ?1",
        params![video_id],
    )
    .map_err(db_error)?;
    rename_pending_summaries(conn, video_id, &renamed)
}

fn rename_pending_summaries(
    conn: &Connection,
    video_id: i64,
    renamed: &HashMap<String, String>,
) -> AppResult<()> {
    let chats: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT id, context_options FROM chats \
                 WHERE video_id = ?1 AND context_options LIKE '%pendingSummaryUids%'",
            )
            .map_err(db_error)?;
        let rows = stmt
            .query_map(params![video_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(db_error)?;
        rows.collect::<Result<_, _>>().map_err(db_error)?
    };
    for (id, raw) in chats {
        let Ok(mut options) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let Some(Value::Array(pending)) = options.get_mut("pendingSummaryUids") else {
            continue;
        };
        for entry in pending.iter_mut() {
            if let Some(new) = entry.as_str().and_then(|old| renamed.get(old)) {
                *entry = Value::String(new.clone());
            }
        }
        conn.execute(
            "UPDATE chats SET context_options = ?1 WHERE id = ?2",
            params![options.to_string(), id],
        )
        .map_err(db_error)?;
    }
    Ok(())
}

fn column(conn: &Connection, sql: &str, video_id: i64) -> AppResult<Vec<String>> {
    let mut stmt = conn.prepare(sql).map_err(db_error)?;
    let rows = stmt
        .query_map(params![video_id], |row| row.get(0))
        .map_err(db_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
}
