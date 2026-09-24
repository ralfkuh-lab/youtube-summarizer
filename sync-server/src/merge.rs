//! Push-Verarbeitung nach den Server-Regeln der Spec: Aliase, Grabsteine,
//! Feld-Zusammenführung, LWW und Echo.

use crate::db::{check_dataset, echo, gone, reason_name};
use crate::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use sync_proto::{
    membership_key, name_key, validate_all, ChatData, CollectionData, GoneReason, Op, OpResult,
    OpStatus, RoundData, SummaryData, VideoData,
};

/// Verarbeitet einen Push-Batch in einer `IMMEDIATE`-Transaktion: zuerst die
/// Datensatz-Prüfung, dann die Strukturprüfung, dann alle Operationen. Jeder
/// Fehler rollt den ganzen Batch zurück.
pub fn push(
    conn: &mut Connection,
    device: i64,
    dataset: &str,
    ops: &[Op],
) -> Result<Vec<OpResult>> {
    push_parsed(conn, device, dataset, || Ok(ops))
}

/// Wie [`push`], parst die Operationen aber erst nach der Datensatz-Prüfung
/// (ein veralteter Datensatz ergibt so immer 409, nie 400).
pub(crate) fn push_parsed<O: AsRef<[Op]>>(
    conn: &mut Connection,
    device: i64,
    dataset: &str,
    parse: impl FnOnce() -> Result<O>,
) -> Result<Vec<OpResult>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    check_dataset(&tx, dataset)?;
    let ops = parse()?;
    let ops = ops.as_ref();
    validate_all(ops).map_err(|(index, message)| Error::Invalid(index, message))?;
    let results = ops
        .iter()
        .map(|op| apply(&tx, device, op))
        .collect::<Result<Vec<_>>>()?;
    tx.commit()?;
    Ok(results)
}

fn apply(tx: &Connection, dev: i64, op: &Op) -> Result<OpResult> {
    match op {
        Op::Video { data, changed_at } => video(tx, dev, data, changed_at),
        Op::VideoGone { uid, reason } => delete(tx, "video", uid, *reason),
        Op::Summary(data) => summary(tx, data),
        Op::SummaryDelete { uid, .. } => delete(tx, "summary", uid, GoneReason::Deleted),
        Op::Chat { data, changed_at } => chat(tx, dev, data, changed_at),
        Op::ChatDelete { uid, .. } => delete(tx, "chat", uid, GoneReason::Deleted),
        Op::Round(data) => round(tx, data),
        Op::Collection { data, changed_at } => collection(tx, dev, data, changed_at),
        Op::CollectionDelete { uid } => delete(tx, "collection", uid, GoneReason::Deleted),
        Op::Membership {
            video_uid,
            collection_uid,
            present,
            changed_at,
        } => membership(tx, dev, video_uid, collection_uid, *present, changed_at),
    }
}

// ---------------------------------------------------------------------------
// Ergebnisse, Status, Aliase
// ---------------------------------------------------------------------------

fn ok() -> OpResult {
    OpResult {
        status: OpStatus::Ok,
        reason: None,
        missing: None,
    }
}

fn rejected(reason: &str) -> OpResult {
    OpResult {
        status: OpStatus::Rejected,
        reason: Some(reason.to_owned()),
        missing: None,
    }
}

enum Status {
    Live,
    Gone,
    Unknown,
}

fn table(entity: &str) -> &'static str {
    match entity {
        "video" => "videos",
        "summary" => "summaries",
        "chat" => "chats",
        "round" => "rounds",
        "collection" => "collections",
        "membership" => "memberships",
        _ => unreachable!("unbekannte Entität {entity}"),
    }
}

fn status(tx: &Connection, entity: &str, uid: &str) -> Result<Status> {
    if gone(tx, entity, uid)?.is_some() {
        return Ok(Status::Gone);
    }
    let sql = format!("SELECT 1 FROM {} WHERE uid = ?1", table(entity));
    let live = tx.query_row(&sql, [uid], |_| Ok(())).optional()?;
    Ok(if live.is_some() {
        Status::Live
    } else {
        Status::Unknown
    })
}

/// Bildet eine Alias-uid auf ihre Wurzel ab (Ketten sind flach gehalten).
fn resolve(tx: &Connection, entity: &str, uid: &str) -> Result<String> {
    Ok(match gone(tx, entity, uid)? {
        Some((GoneReason::Merged, Some(root))) => root,
        _ => uid.to_owned(),
    })
}

/// `from` wird Alias von `root`; bestehende Aliase auf `from` zeigen danach
/// direkt auf `root` und werden, da sich ihr Zustand ändert, ebenfalls geechot.
fn make_alias(tx: &Connection, entity: &str, from: &str, root: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO gone (entity, uid, reason, merged_into) VALUES (?1, ?2, 'merged', ?3)",
        params![entity, from, root],
    )?;
    let flattened = tx
        .prepare(
            "UPDATE gone SET merged_into = ?3 WHERE entity = ?1 AND merged_into = ?2 RETURNING uid",
        )?
        .query_map(params![entity, from, root], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for alias in flattened {
        echo(tx, entity, &alias)?;
    }
    echo(tx, entity, from)
}

/// Prüft die direkten Eltern eines Kindes. Grabstein → `rejected` mit Echo
/// des Grabsteins (hat Vorrang), unbekannt → `retry` mit `missing`.
fn check_parents(tx: &Connection, parents: &[(&str, &str)]) -> Result<Option<OpResult>> {
    let mut result = None;
    let mut missing = None;
    for &(entity, uid) in parents {
        match status(tx, entity, uid)? {
            Status::Live => {}
            Status::Gone => {
                echo(tx, entity, uid)?;
                result = Some(rejected(&format!("{entity}Gone")));
            }
            Status::Unknown => {
                missing.get_or_insert_with(|| OpResult {
                    status: OpStatus::Retry,
                    reason: None,
                    missing: Some(format!("{entity}/{uid}")),
                });
            }
        }
    }
    Ok(result.or(missing))
}

/// Bekanntes unveränderliches Kind: `ok` ohne Änderung, solange alle
/// Elternobjekte (`(Spalte, erwartete uid)`) gleich bleiben; das Kind wird in
/// jedem Fall geechot.
fn known_child(
    tx: &Connection,
    entity: &str,
    uid: &str,
    parents: &[(&str, &str)],
) -> Result<OpResult> {
    let mut unchanged = true;
    for &(column, parent) in parents {
        let sql = format!("SELECT {column} FROM {} WHERE uid = ?1", table(entity));
        let stored: String = tx.query_row(&sql, [uid], |row| row.get(0))?;
        unchanged &= stored == parent;
    }
    echo(tx, entity, uid)?;
    Ok(if unchanged {
        ok()
    } else {
        rejected("Elternobjekt unveränderlich")
    })
}

fn echo_all(tx: &Connection, entity: &str, sql: &str, uid: &str) -> Result<()> {
    let keys = tx
        .prepare(sql)?
        .query_map([uid], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for key in keys {
        echo(tx, entity, &key)?;
    }
    Ok(())
}

const MEMBERSHIP_KEY: &str = "video_uid || '/' || collection_uid";

/// Löscht Zeilen ohne Grabstein, samt ihren `seqs`-Einträgen.
fn drop_rows(tx: &Connection, entity: &str, filter: &str, uid: &str) -> Result<()> {
    let table = table(entity);
    let key = if entity == "membership" {
        MEMBERSHIP_KEY
    } else {
        "uid"
    };
    tx.execute(
        &format!(
            "DELETE FROM seqs WHERE entity = ?1
             AND key IN (SELECT {key} FROM {table} WHERE {filter} = ?2)"
        ),
        params![entity, uid],
    )?;
    tx.execute(&format!("DELETE FROM {table} WHERE {filter} = ?1"), [uid])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Löschungen (nie `retry`)
// ---------------------------------------------------------------------------

/// Grabstein für eine lebende, unbekannte oder schon begrabene uid; der erste
/// Grund bleibt. Kinder werden ohne eigene Grabsteine entfernt.
fn delete(tx: &Connection, entity: &str, addressed: &str, reason: GoneReason) -> Result<OpResult> {
    let uid = resolve(tx, entity, addressed)?;
    if uid != addressed {
        echo(tx, entity, addressed)?;
    }
    if let Status::Live = status(tx, entity, &uid)? {
        match entity {
            "video" => {
                for child in ["summary", "chat", "round", "membership"] {
                    drop_rows(tx, child, "video_uid", &uid)?;
                }
            }
            "chat" => drop_rows(tx, "round", "chat_uid", &uid)?,
            "collection" => drop_rows(tx, "membership", "collection_uid", &uid)?,
            _ => {}
        }
        tx.execute(
            &format!("DELETE FROM {} WHERE uid = ?1", table(entity)),
            [&uid],
        )?;
    }
    tx.execute(
        "INSERT OR IGNORE INTO gone (entity, uid, reason) VALUES (?1, ?2, ?3)",
        params![entity, uid, reason_name(reason)],
    )?;
    echo(tx, entity, &uid)?;
    Ok(ok())
}

// ---------------------------------------------------------------------------
// Videos
// ---------------------------------------------------------------------------

const LWW_FIELDS: [&str; 4] = ["title", "url", "thumbnailUrl", "publishedAt"];
/// Nie durch `null` ersetzt; leeres Feld wird aus jedem Datensatz gefüllt.
const FILL_FIELDS: [&str; 4] = ["transcript", "chapters", "description", "thumbnailData"];

/// Feldwert mit dem Zeitstempel und Gerät seines Gewinners.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Field {
    v: Value,
    at: String,
    dev: i64,
}

type Fields = BTreeMap<String, Field>;

fn newer(a: &Field, b: &Field) -> bool {
    (&a.at, a.dev) > (&b.at, b.dev)
}

fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        _ => false,
    }
}

fn incoming_fields(data: &VideoData, at: &str, dev: i64) -> Result<Fields> {
    let Value::Object(mut map) = serde_json::to_value(data)? else {
        return Err(Error::Corrupt("VideoData ist kein Objekt".into()));
    };
    Ok(LWW_FIELDS
        .iter()
        .chain(&FILL_FIELDS)
        .chain(&["transcriptError"])
        .map(|&name| {
            let field = Field {
                v: map.remove(name).unwrap_or(Value::Null),
                at: at.to_owned(),
                dev,
            };
            (name.to_owned(), field)
        })
        .collect())
}

/// Feld-Zusammenführung: LWW-Felder nach (Zeitstempel, Gerät); Füllfelder
/// werden nie geleert, leere aus jedem Datensatz gefüllt, zwei Werte → LWW.
/// `transcriptError` ist `null`, sobald ein Transkript da ist, sonst LWW.
fn merge_fields(stored: &mut Fields, incoming: Fields) {
    for (name, new) in incoming {
        let wins = match stored.get(&name) {
            None => true,
            Some(old) if FILL_FIELDS.contains(&name.as_str()) => {
                !is_empty(&new.v) && (is_empty(&old.v) || newer(&new, old))
            }
            Some(old) => newer(&new, old),
        };
        if wins {
            stored.insert(name, new);
        }
    }
    let has_transcript = stored
        .get("transcript")
        .is_some_and(|field| !is_empty(&field.v));
    if has_transcript {
        if let Some(error) = stored.get_mut("transcriptError") {
            error.v = Value::Null;
        }
    }
}

/// Setzt aus gespeicherten Feldern den übertragenen Video-Zustand zusammen.
pub(crate) fn video_data(
    uid: &str,
    youtube_id: &str,
    created_at: &str,
    fields: &str,
) -> Result<VideoData> {
    let fields: Fields = serde_json::from_str(fields)?;
    let mut map = serde_json::Map::new();
    map.insert("uid".into(), uid.into());
    map.insert("youtubeId".into(), youtube_id.into());
    map.insert("createdAt".into(), created_at.into());
    for (name, field) in fields {
        map.insert(name, field.v);
    }
    Ok(serde_json::from_value(Value::Object(map))?)
}

fn merge_into_video(tx: &Connection, uid: &str, incoming: Fields) -> Result<()> {
    let stored: String =
        tx.query_row("SELECT fields FROM videos WHERE uid = ?1", [uid], |row| {
            row.get(0)
        })?;
    let mut fields: Fields = serde_json::from_str(&stored)?;
    merge_fields(&mut fields, incoming);
    tx.execute(
        "UPDATE videos SET fields = ?2 WHERE uid = ?1",
        params![uid, serde_json::to_string(&fields)?],
    )?;
    Ok(())
}

fn video(tx: &Connection, dev: i64, data: &VideoData, at: &str) -> Result<OpResult> {
    let uid = resolve(tx, "video", &data.uid)?;
    if uid != data.uid {
        echo(tx, "video", &data.uid)?;
    }
    let incoming = incoming_fields(data, at, dev)?;
    match status(tx, "video", &uid)? {
        Status::Gone => {
            echo(tx, "video", &uid)?;
            Ok(rejected("videoGone"))
        }
        Status::Live => {
            let youtube_id: String = tx.query_row(
                "SELECT youtube_id FROM videos WHERE uid = ?1",
                [&uid],
                |row| row.get(0),
            )?;
            echo(tx, "video", &uid)?;
            if youtube_id != data.youtube_id {
                return Ok(rejected("youtubeId unveränderlich"));
            }
            merge_into_video(tx, &uid, incoming)?;
            Ok(ok())
        }
        Status::Unknown => {
            let root: Option<String> = tx
                .query_row(
                    "SELECT uid FROM videos WHERE youtube_id = ?1",
                    [&data.youtube_id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(root) = root {
                make_alias(tx, "video", &uid, &root)?;
                merge_into_video(tx, &root, incoming)?;
                echo_video_tree(tx, &root)?;
            } else {
                let mut fields = Fields::new();
                merge_fields(&mut fields, incoming);
                tx.execute(
                    "INSERT INTO videos (uid, youtube_id, created_at, fields) VALUES (?1, ?2, ?3, ?4)",
                    params![uid, data.youtube_id, data.created_at, serde_json::to_string(&fields)?],
                )?;
                echo(tx, "video", &uid)?;
            }
            Ok(ok())
        }
    }
}

/// Echo eines Videos und seines ganzen Teilbaums (nach dem Verschmelzen).
fn echo_video_tree(tx: &Connection, uid: &str) -> Result<()> {
    echo(tx, "video", uid)?;
    for entity in ["summary", "chat", "round"] {
        let sql = format!("SELECT uid FROM {} WHERE video_uid = ?1", table(entity));
        echo_all(tx, entity, &sql, uid)?;
    }
    let sql = format!("SELECT {MEMBERSHIP_KEY} FROM memberships WHERE video_uid = ?1");
    echo_all(tx, "membership", &sql, uid)
}

// ---------------------------------------------------------------------------
// Kinder
// ---------------------------------------------------------------------------

fn summary(tx: &Connection, data: &SummaryData) -> Result<OpResult> {
    let video_uid = resolve(tx, "video", &data.video_uid)?;
    match status(tx, "summary", &data.uid)? {
        Status::Gone => {
            echo(tx, "summary", &data.uid)?;
            return Ok(rejected("summaryGone"));
        }
        Status::Live => {
            return known_child(tx, "summary", &data.uid, &[("video_uid", &video_uid)]);
        }
        Status::Unknown => {}
    }
    if let Some(result) = check_parents(tx, &[("video", &video_uid)])? {
        return Ok(result);
    }
    let stored = SummaryData {
        video_uid: video_uid.clone(),
        ..data.clone()
    };
    tx.execute(
        "INSERT INTO summaries (uid, video_uid, data) VALUES (?1, ?2, ?3)",
        params![data.uid, video_uid, serde_json::to_string(&stored)?],
    )?;
    echo(tx, "summary", &data.uid)?;
    drop_from_foreign_chats(tx, &data.uid, &video_uid)?;
    Ok(ok())
}

/// Chats anderer Videos, die die neue Summary als noch unbekannte uid führten,
/// verlieren sie (sie ist jetzt fremd) und werden geechot.
fn drop_from_foreign_chats(tx: &Connection, summary_uid: &str, video_uid: &str) -> Result<()> {
    let chats = tx
        .prepare(
            "SELECT uid, data FROM chats WHERE video_uid != ?1 AND EXISTS (
                SELECT 1 FROM json_each(data, '$.contextOptions.summaryUids') WHERE value = ?2)",
        )?
        .query_map([video_uid, summary_uid], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<(String, String)>>>()?;
    for (chat_uid, stored) in chats {
        let mut chat: ChatData = serde_json::from_str(&stored)?;
        if let Some(uids) = &mut chat.context_options.summary_uids {
            uids.retain(|uid| uid != summary_uid);
        }
        tx.execute(
            "UPDATE chats SET data = ?2 WHERE uid = ?1",
            params![chat_uid, serde_json::to_string(&chat)?],
        )?;
        echo(tx, "chat", &chat_uid)?;
    }
    Ok(())
}

/// `summaryUids` ohne Zusammenfassungen fremder Videos; unbekannte bleiben
/// (sie können noch ankommen).
fn own_summaries(
    tx: &Connection,
    video_uid: &str,
    uids: &Option<Vec<String>>,
) -> Result<Option<Vec<String>>> {
    let Some(uids) = uids else {
        return Ok(None);
    };
    let mut kept = Vec::new();
    for uid in uids {
        let owner: Option<String> = tx
            .query_row(
                "SELECT video_uid FROM summaries WHERE uid = ?1",
                [uid],
                |row| row.get(0),
            )
            .optional()?;
        if owner.is_none_or(|owner| owner == video_uid) {
            kept.push(uid.clone());
        }
    }
    Ok(Some(kept))
}

fn chat(tx: &Connection, dev: i64, data: &ChatData, at: &str) -> Result<OpResult> {
    let video_uid = resolve(tx, "video", &data.video_uid)?;
    let mut incoming = ChatData {
        video_uid: video_uid.clone(),
        ..data.clone()
    };
    incoming.context_options.summary_uids =
        own_summaries(tx, &video_uid, &data.context_options.summary_uids)?;
    match status(tx, "chat", &data.uid)? {
        Status::Gone => {
            echo(tx, "chat", &data.uid)?;
            Ok(rejected("chatGone"))
        }
        Status::Live => {
            let (stored_video, stored_at, stored_dev, stored): (String, String, i64, String) = tx
                .query_row(
                "SELECT video_uid, changed_at, dev, data FROM chats WHERE uid = ?1",
                [&data.uid],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
            echo(tx, "chat", &data.uid)?;
            if stored_video != video_uid {
                return Ok(rejected("Elternobjekt unveränderlich"));
            }
            if (at, dev) > (stored_at.as_str(), stored_dev) {
                let mut merged: ChatData = serde_json::from_str(&stored)?;
                merged.title = incoming.title;
                merged.context_options = incoming.context_options;
                merged.updated_at = incoming.updated_at;
                tx.execute(
                    "UPDATE chats SET changed_at = ?2, dev = ?3, data = ?4 WHERE uid = ?1",
                    params![data.uid, at, dev, serde_json::to_string(&merged)?],
                )?;
            }
            Ok(ok())
        }
        Status::Unknown => {
            if let Some(result) = check_parents(tx, &[("video", &video_uid)])? {
                return Ok(result);
            }
            tx.execute(
                "INSERT INTO chats (uid, video_uid, changed_at, dev, data) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![data.uid, video_uid, at, dev, serde_json::to_string(&incoming)?],
            )?;
            echo(tx, "chat", &data.uid)?;
            Ok(ok())
        }
    }
}

fn round(tx: &Connection, data: &RoundData) -> Result<OpResult> {
    let video_uid = resolve(tx, "video", &data.video_uid)?;
    if let Status::Live = status(tx, "round", &data.uid)? {
        let parents = [
            ("chat_uid", data.chat_uid.as_str()),
            ("video_uid", &video_uid),
        ];
        return known_child(tx, "round", &data.uid, &parents);
    }
    let parents = [
        ("video", video_uid.as_str()),
        ("chat", data.chat_uid.as_str()),
    ];
    if let Some(result) = check_parents(tx, &parents)? {
        return Ok(result);
    }
    let chat_video: String = tx.query_row(
        "SELECT video_uid FROM chats WHERE uid = ?1",
        [&data.chat_uid],
        |row| row.get(0),
    )?;
    if chat_video != video_uid {
        echo(tx, "chat", &data.chat_uid)?;
        return Ok(rejected("chatUid gehört nicht zu videoUid"));
    }
    let stored = RoundData {
        video_uid: video_uid.clone(),
        ..data.clone()
    };
    tx.execute(
        "INSERT INTO rounds (uid, chat_uid, video_uid, data) VALUES (?1, ?2, ?3, ?4)",
        params![
            data.uid,
            data.chat_uid,
            video_uid,
            serde_json::to_string(&stored)?
        ],
    )?;
    echo(tx, "round", &data.uid)?;
    Ok(ok())
}

// ---------------------------------------------------------------------------
// Sammlungen und Zuordnungen
// ---------------------------------------------------------------------------

fn collection_by_key(tx: &Connection, key: &str) -> Result<Option<String>> {
    Ok(tx
        .query_row(
            "SELECT uid FROM collections WHERE name_key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()?)
}

fn collection(tx: &Connection, dev: i64, data: &CollectionData, at: &str) -> Result<OpResult> {
    let uid = resolve(tx, "collection", &data.uid)?;
    if uid != data.uid {
        echo(tx, "collection", &data.uid)?;
    }
    let key = name_key(&data.name);
    match status(tx, "collection", &uid)? {
        Status::Gone => {
            echo(tx, "collection", &uid)?;
            return Ok(rejected("collectionGone"));
        }
        Status::Unknown => match collection_by_key(tx, &key)? {
            Some(root) => merge_collection(tx, &uid, &root)?,
            None => {
                tx.execute(
                    "INSERT INTO collections (uid, name, name_key, created_at, changed_at, dev)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![uid, data.name, key, data.created_at, at, dev],
                )?;
                echo(tx, "collection", &uid)?;
            }
        },
        Status::Live => {
            let (stored_at, stored_dev): (String, i64) = tx.query_row(
                "SELECT changed_at, dev FROM collections WHERE uid = ?1",
                [&uid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            // Erst Namens-LWW; nur ein gewinnender Name kann verschmelzen.
            if (at, dev) > (stored_at.as_str(), stored_dev) {
                match collection_by_key(tx, &key)? {
                    Some(other) if other != uid => merge_collection(tx, &uid, &other)?,
                    _ => {
                        tx.execute(
                            "UPDATE collections SET name = ?2, name_key = ?3, changed_at = ?4, dev = ?5
                             WHERE uid = ?1",
                            params![uid, data.name, key, at, dev],
                        )?;
                        echo(tx, "collection", &uid)?;
                    }
                }
            } else {
                echo(tx, "collection", &uid)?;
            }
        }
    }
    Ok(ok())
}

/// Verschmilzt `from` in die lebende Wurzel `root` (deren Name bleibt):
/// Zuordnungen umhängen, Alias anlegen, Wurzel und alle ihre Zuordnungen echoen.
fn merge_collection(tx: &Connection, from: &str, root: &str) -> Result<()> {
    let rows = tx
        .prepare(
            "SELECT video_uid, present, changed_at, dev FROM memberships WHERE collection_uid = ?1",
        )?
        .query_map([from], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<(String, bool, String, i64)>>>()?;
    for (video_uid, present, at, dev) in rows {
        upsert_membership(tx, &video_uid, root, present, &at, dev, true)?;
    }
    drop_rows(tx, "membership", "collection_uid", from)?;
    tx.execute("DELETE FROM collections WHERE uid = ?1", [from])?;
    make_alias(tx, "collection", from, root)?;
    echo(tx, "collection", root)?;
    let sql = format!("SELECT {MEMBERSHIP_KEY} FROM memberships WHERE collection_uid = ?1");
    echo_all(tx, "membership", &sql, root)
}

/// LWW auf `present`. Beim Umhängen (`pair_merge`) gewinnt bei gleichem
/// Zeitstempel `present = true`, sonst entscheidet das Gerät.
fn upsert_membership(
    tx: &Connection,
    video_uid: &str,
    collection_uid: &str,
    present: bool,
    at: &str,
    dev: i64,
    pair_merge: bool,
) -> Result<()> {
    let stored: Option<(bool, String, i64)> = tx
        .query_row(
            "SELECT present, changed_at, dev FROM memberships
             WHERE video_uid = ?1 AND collection_uid = ?2",
            [video_uid, collection_uid],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let wins = match &stored {
        None => true,
        Some((stored_present, stored_at, _)) if pair_merge && at == stored_at => {
            present && !stored_present
        }
        Some((_, stored_at, stored_dev)) => (at, dev) > (stored_at.as_str(), *stored_dev),
    };
    if wins {
        tx.execute(
            "INSERT INTO memberships (video_uid, collection_uid, present, changed_at, dev)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (video_uid, collection_uid)
             DO UPDATE SET present = excluded.present, changed_at = excluded.changed_at, dev = excluded.dev",
            params![video_uid, collection_uid, present, at, dev],
        )?;
    }
    Ok(())
}

fn membership(
    tx: &Connection,
    dev: i64,
    video_uid: &str,
    collection_uid: &str,
    present: bool,
    at: &str,
) -> Result<OpResult> {
    let video_uid = resolve(tx, "video", video_uid)?;
    let collection_uid = resolve(tx, "collection", collection_uid)?;
    let parents = [
        ("video", video_uid.as_str()),
        ("collection", collection_uid.as_str()),
    ];
    if let Some(result) = check_parents(tx, &parents)? {
        return Ok(result);
    }
    upsert_membership(tx, &video_uid, &collection_uid, present, at, dev, false)?;
    echo(
        tx,
        "membership",
        &membership_key(&video_uid, &collection_uid),
    )?;
    Ok(ok())
}
