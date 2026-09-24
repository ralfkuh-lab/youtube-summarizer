//! Push-Seite ohne HTTP: Snapshot der Outbox und Quittung
//! (docs/spec-sync.md, „Push“).

use std::collections::BTreeSet;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use sync_proto::{
    ChatData, CollectionData, ContextOptions, GoneReason, Op, OpResult, OpStatus, RoundData,
    RoundMessage, SummaryData, VideoData,
};

use super::db_error;
use crate::storage::AppResult;

/// Eine Operation mit der `seq` des Outbox-Eintrags, aus dem sie gebaut wurde.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pending {
    pub seq: i64,
    pub op: Op,
}

/// Beide Push-Phasen aus demselben Snapshot: erst alle Löschungen, danach die
/// Upserts in der Reihenfolge collection, video, summary, chat, round,
/// membership.
#[derive(Debug, Default)]
pub(crate) struct Snapshot {
    pub deletes: Vec<Pending>,
    pub upserts: Vec<Pending>,
    /// Ausstehende Löschungen, die als `unsendable` markiert sind: Solange es
    /// sie gibt, darf Phase 2 nicht laufen (Reihenfolge aus E20).
    pub blocked_deletes: usize,
}

struct Entry {
    seq: i64,
    entity: String,
    key: String,
    owner: Option<String>,
    tomb: Option<String>,
    changed_at: String,
    unsendable: bool,
}

/// Liest die Outbox in einer kurzen `IMMEDIATE`-Transaktion und baut aus dem
/// aktuellen Zustand je Eintrag eine Operation: Zeile vorhanden → Upsert,
/// sonst Delete. Upserts privater Videos und ihrer Kinder sowie Einträge, aus
/// denen keine Operation entstehen kann (Runde ohne Nachrichten), werden
/// verworfen und entfernt; strukturell ungültige Operationen werden
/// `unsendable`. In derselben Transaktion wird `published = 1` für jedes Video
/// gesetzt, dessen uid in einer Operation vorkommt.
pub(crate) fn snapshot(conn: &mut Connection) -> AppResult<Snapshot> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    let entries = {
        let mut stmt = tx
            .prepare(
                "SELECT seq, entity, key, owner, tomb, changed_at, unsendable IS NOT NULL \
                 FROM sync_outbox ORDER BY seq",
            )
            .map_err(db_error)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Entry {
                    seq: row.get(0)?,
                    entity: row.get(1)?,
                    key: row.get(2)?,
                    owner: row.get(3)?,
                    tomb: row.get(4)?,
                    changed_at: row.get(5)?,
                    unsendable: row.get(6)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?
    };

    let mut ops = Vec::new();
    let mut published = BTreeSet::new();
    let mut blocked_deletes = 0;
    for entry in entries {
        if entry.unsendable {
            blocked_deletes += usize::from(is_delete_wish(&tx, &entry)?);
            continue;
        }
        let op = match build_op(&tx, &entry)? {
            Some(op) => op,
            None => {
                tx.execute("DELETE FROM sync_outbox WHERE seq = ?1", params![entry.seq])
                    .map_err(db_error)?;
                continue;
            }
        };
        if let Err(reason) = op.validate() {
            tx.execute(
                "UPDATE sync_outbox SET unsendable = ?1 WHERE seq = ?2",
                params![reason, entry.seq],
            )
            .map_err(db_error)?;
            blocked_deletes += usize::from(op.is_delete());
            continue;
        }
        let video_uid = video_uid_of(&op);
        if !video_uid.is_empty() {
            published.insert(video_uid.to_string());
        }
        ops.push(Pending { seq: entry.seq, op });
    }
    for uid in &published {
        tx.execute(
            "UPDATE videos SET published = 1 WHERE uid = ?1",
            params![uid],
        )
        .map_err(db_error)?;
    }
    tx.commit().map_err(db_error)?;

    let (mut deletes, mut upserts): (Vec<Pending>, Vec<Pending>) =
        ops.into_iter().partition(|pending| pending.op.is_delete());
    deletes.sort_by_key(|pending| (phase_rank(&pending.op), pending.seq));
    upserts.sort_by_key(|pending| (phase_rank(&pending.op), pending.seq));
    Ok(Snapshot {
        deletes,
        upserts,
        blocked_deletes,
    })
}

/// Würde der Eintrag eine Löschung ergeben (Zeile fehlt)? Ohne die Operation
/// zu bauen, damit große nicht sendbare Einträge nicht jedes Mal gelesen
/// werden.
fn is_delete_wish(conn: &Connection, entry: &Entry) -> AppResult<bool> {
    let table = match entry.entity.as_str() {
        "video" => "videos",
        "summary" => "summaries",
        "chat" => "chats",
        "collection" => "collections",
        _ => return Ok(false),
    };
    let exists: bool = conn
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE uid = ?1)"),
            params![entry.key],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    Ok(!exists)
}

/// Rang innerhalb der Phase (Löschungen bzw. Upserts).
fn phase_rank(op: &Op) -> u8 {
    match op {
        Op::VideoGone { .. } => 0,
        Op::CollectionDelete { .. } => 1,
        Op::ChatDelete { .. } => 2,
        Op::SummaryDelete { .. } => 3,
        Op::Collection { .. } => 0,
        Op::Video { .. } => 1,
        Op::Summary(_) => 2,
        Op::Chat { .. } => 3,
        Op::Round(_) => 4,
        Op::Membership { .. } => 5,
    }
}

/// Die Video-uid, die eine Operation berührt (für `published`); bei
/// Sammlungen leer.
fn video_uid_of(op: &Op) -> &str {
    match op {
        Op::Video { data, .. } => &data.uid,
        Op::VideoGone { uid, .. } => uid,
        Op::Summary(data) => &data.video_uid,
        Op::SummaryDelete { video_uid, .. } | Op::ChatDelete { video_uid, .. } => video_uid,
        Op::Chat { data, .. } => &data.video_uid,
        Op::Round(data) => &data.video_uid,
        Op::Membership { video_uid, .. } => video_uid,
        Op::Collection { .. } | Op::CollectionDelete { .. } => "",
    }
}

/// `None`: Eintrag verwerfen.
fn build_op(conn: &Connection, entry: &Entry) -> AppResult<Option<Op>> {
    let owner = entry.owner.clone().unwrap_or_default();
    let changed_at = entry.changed_at.clone();
    Ok(match entry.entity.as_str() {
        "video" => match video_data(conn, &entry.key)? {
            Some((_, true)) => None,
            Some((data, false)) => Some(Op::Video { data, changed_at }),
            None => Some(Op::VideoGone {
                uid: entry.key.clone(),
                reason: match entry.tomb.as_deref() {
                    Some("withdrawn") => GoneReason::Withdrawn,
                    _ => GoneReason::Deleted,
                },
            }),
        },
        "summary" => match summary_data(conn, &entry.key)? {
            Some((_, true)) => None,
            Some((data, false)) => Some(Op::Summary(data)),
            None => Some(Op::SummaryDelete {
                uid: entry.key.clone(),
                video_uid: owner,
            }),
        },
        "chat" => match chat_data(conn, &entry.key)? {
            Some((_, true)) => None,
            Some((data, false)) => Some(Op::Chat { data, changed_at }),
            None => Some(Op::ChatDelete {
                uid: entry.key.clone(),
                video_uid: owner,
            }),
        },
        "round" => match round_data(conn, &entry.key)? {
            Some((data, false)) => Some(Op::Round(data)),
            // Runden werden nie einzeln gelöscht; ohne Nachrichten gibt es
            // nichts zu senden.
            _ => None,
        },
        "collection" => match collection_data(conn, &entry.key)? {
            Some(data) => Some(Op::Collection { data, changed_at }),
            None => Some(Op::CollectionDelete {
                uid: entry.key.clone(),
            }),
        },
        "membership" => {
            let Some((video_uid, collection_uid)) = entry.key.split_once('/') else {
                return Ok(None);
            };
            let private: Option<bool> = conn
                .query_row(
                    "SELECT local_only FROM videos WHERE uid = ?1",
                    params![video_uid],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db_error)?;
            if private == Some(true) {
                return Ok(None);
            }
            let present: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM video_collections vc \
                     JOIN videos v ON v.id = vc.video_id \
                     JOIN collections c ON c.id = vc.collection_id \
                     WHERE v.uid = ?1 AND c.uid = ?2)",
                    params![video_uid, collection_uid],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            Some(Op::Membership {
                video_uid: video_uid.to_string(),
                collection_uid: collection_uid.to_string(),
                present,
                changed_at,
            })
        }
        _ => None,
    })
}

/// `(Daten, privat)` eines Videos nach uid.
fn video_data(conn: &Connection, uid: &str) -> AppResult<Option<(VideoData, bool)>> {
    conn.query_row(
        "SELECT uid, video_id, url, title, thumbnail_url, thumbnail_data, transcript, chapters, \
         published_at, description, transcript_error, created_at, local_only \
         FROM videos WHERE uid = ?1",
        params![uid],
        |row| {
            let thumbnail: Option<Vec<u8>> = row.get(5)?;
            Ok((
                VideoData {
                    uid: row.get(0)?,
                    youtube_id: row.get(1)?,
                    url: row.get(2)?,
                    title: row.get(3)?,
                    thumbnail_url: row.get(4)?,
                    thumbnail_data: thumbnail.map(|bytes| BASE64.encode(bytes)),
                    transcript: row.get(6)?,
                    chapters: row.get(7)?,
                    published_at: row.get(8)?,
                    description: row.get(9)?,
                    transcript_error: row.get(10)?,
                    created_at: row.get(11)?,
                },
                row.get(12)?,
            ))
        },
    )
    .optional()
    .map_err(db_error)
}

fn summary_data(conn: &Connection, uid: &str) -> AppResult<Option<(SummaryData, bool)>> {
    conn.query_row(
        "SELECT s.uid, v.uid, s.created_at, s.summary, s.provider, s.model, s.options, v.local_only \
         FROM summaries s JOIN videos v ON v.id = s.video_id WHERE s.uid = ?1",
        params![uid],
        |row| {
            Ok((
                SummaryData {
                    uid: row.get(0)?,
                    video_uid: row.get(1)?,
                    created_at: row.get(2)?,
                    summary: row.get(3)?,
                    provider: row.get(4)?,
                    model: row.get(5)?,
                    options: row.get(6)?,
                },
                row.get(7)?,
            ))
        },
    )
    .optional()
    .map_err(db_error)
}

fn chat_data(conn: &Connection, uid: &str) -> AppResult<Option<(ChatData, bool)>> {
    let row = conn
        .query_row(
            "SELECT c.uid, v.uid, c.title, c.created_at, c.updated_at, c.context_options, \
             v.local_only, c.video_id \
             FROM chats c JOIN videos v ON v.id = c.video_id WHERE c.uid = ?1",
            params![uid],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, bool>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?;
    let Some((uid, video_uid, title, created_at, updated_at, options, private, video_id)) = row
    else {
        return Ok(None);
    };
    let context_options = push_context_options(conn, video_id, options.as_deref())?;
    Ok(Some((
        ChatData {
            uid,
            video_uid,
            title,
            created_at,
            updated_at,
            context_options,
        },
        private,
    )))
}

/// Lokale Summary-ids (nur dieses Videos) und `pendingSummaryUids` werden
/// zusammen zu `summaryUids`; `null` bleibt `null` („neueste“).
fn push_context_options(
    conn: &Connection,
    video_id: i64,
    raw: Option<&str>,
) -> AppResult<ContextOptions> {
    let options: Value = raw
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or(Value::Null);
    let transcript = options
        .get("transcript")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let Some(ids) = options.get("summaryIds").and_then(Value::as_array) else {
        return Ok(ContextOptions {
            transcript,
            summary_uids: None,
        });
    };
    let mut uids: Vec<String> = Vec::new();
    for id in ids.iter().filter_map(Value::as_i64) {
        let uid: Option<String> = conn
            .query_row(
                "SELECT uid FROM summaries WHERE id = ?1 AND video_id = ?2",
                params![id, video_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        uids.extend(uid);
    }
    let pending = options
        .get("pendingSummaryUids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    for uid in pending {
        if !uids.iter().any(|known| known == uid) {
            uids.push(uid.to_string());
        }
    }
    Ok(ContextOptions {
        transcript,
        summary_uids: Some(uids),
    })
}

fn round_data(conn: &Connection, uid: &str) -> AppResult<Option<(RoundData, bool)>> {
    let mut stmt = conn
        .prepare(
            "SELECT c.uid, v.uid, m.created_at, m.role, m.content, m.tool_calls, m.tool_call_id, \
             m.provider, m.model, v.local_only \
             FROM chat_messages m JOIN chats c ON c.id = m.chat_id \
             JOIN videos v ON v.id = c.video_id \
             WHERE m.round_uid = ?1 ORDER BY m.position, m.id",
        )
        .map_err(db_error)?;
    let rows = stmt
        .query_map(params![uid], |row| {
            let tool_calls: Option<String> = row.get(5)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                RoundMessage {
                    role: row.get(3)?,
                    content: row.get(4)?,
                    tool_calls: tool_calls.and_then(|raw| serde_json::from_str(&raw).ok()),
                    tool_call_id: row.get(6)?,
                    provider: row.get(7)?,
                    model: row.get(8)?,
                },
                row.get::<_, bool>(9)?,
            ))
        })
        .map_err(db_error)?;
    let rows = rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?;
    let Some((chat_uid, video_uid, created_at, _, private)) = rows.first().cloned() else {
        return Ok(None);
    };
    Ok(Some((
        RoundData {
            uid: uid.to_string(),
            chat_uid,
            video_uid,
            created_at,
            messages: rows.into_iter().map(|row| row.3).collect(),
        },
        private,
    )))
}

fn collection_data(conn: &Connection, uid: &str) -> AppResult<Option<CollectionData>> {
    conn.query_row(
        "SELECT uid, name, created_at FROM collections WHERE uid = ?1",
        params![uid],
        |row| {
            Ok(CollectionData {
                uid: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(db_error)
}

/// Markiert genau diese Version eines Eintrags als nicht sendbar (eine neue
/// Version setzt die Markierung zurück).
pub(crate) fn mark_unsendable(conn: &Connection, seq: i64, reason: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE sync_outbox SET unsendable = ?1 WHERE seq = ?2",
        params![reason, seq],
    )
    .map_err(db_error)?;
    Ok(())
}

/// Hebt alle `unsendable`-Markierungen auf (Einträge und Löschwünsche
/// bleiben); ein Statement, also eine Transaktion.
pub(crate) fn clear_unsendable(conn: &mut Connection) -> AppResult<usize> {
    conn.execute(
        "UPDATE sync_outbox SET unsendable = NULL WHERE unsendable IS NOT NULL",
        [],
    )
    .map_err(db_error)
}

/// Quittiert die Antworten eines gesendeten Blocks in einer kurzen
/// Schreibtransaktion: `ok`/`rejected` löschen den Eintrag genau dieser
/// `seq` (neuere Versionen bleiben), `retry` behält ihn.
pub(crate) fn acknowledge(
    conn: &mut Connection,
    sent: &[Pending],
    results: &[OpResult],
) -> AppResult<()> {
    if sent.len() != results.len() {
        return Err(format!(
            "Push-Antwort passt nicht: {} Operationen, {} Ergebnisse",
            sent.len(),
            results.len()
        ));
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    for (pending, result) in sent.iter().zip(results) {
        if result.status != OpStatus::Retry {
            tx.execute(
                "DELETE FROM sync_outbox WHERE seq = ?1",
                params![pending.seq],
            )
            .map_err(db_error)?;
        }
    }
    tx.commit().map_err(db_error)
}
