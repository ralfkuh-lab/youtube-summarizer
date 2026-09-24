//! Apply der gestagten Server-Zustände (docs/spec-sync.md, „Apply“).
//!
//! Eine `IMMEDIATE`-Transaktion mit `applying = '1'` (Trigger schweigen); die
//! Inbox wird je Schlüssel auf die höchste `seq` verdichtet. Phase 1:
//! Grabsteine (zuerst Aliase, dann `deleted`/`withdrawn`), Phase 2: lebende
//! Zustände. Grundregel: Ein lebender Zustand wird übersprungen, wenn für
//! seinen Schlüssel ein Outbox-Eintrag aussteht, auch ohne lokale Zeile.
//! Zurückgestellte Zustände bleiben in `sync_inbox`, alle übrigen werden
//! entfernt. Eingehende Zeitstempel werden unverändert gespeichert.

mod merge;

use std::collections::{BTreeSet, HashMap};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sync_proto::{
    name_key, now_canonical, ChatData, CollectionData, GoneReason, RoundData, State, SummaryData,
    VideoData,
};

use super::{db_error, privatize, state_set, Checkpoints};
use crate::storage::{self, AppResult};

/// Was sich geändert hat (für `sync://applied`).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Applied {
    /// Lokale ids der betroffenen Videos (auch gelöschte).
    pub video_ids: BTreeSet<i64>,
    pub collections: bool,
}

pub(crate) fn apply(conn: &mut Connection) -> AppResult<Applied> {
    apply_until(conn, None)
}

/// Wie [`apply`]; `abort_after` bricht in Tests nach dem n-ten Zwischenstand
/// ab (Referenzfall C19).
pub(crate) fn apply_until(conn: &mut Connection, abort_after: Option<usize>) -> AppResult<Applied> {
    // Löschungen verlassen sich auf die Kaskaden.
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(db_error)?;
    let mut steps = Checkpoints::new(abort_after);
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    state_set(&tx, "applying", "1")?;
    let (states, loaded) = load_inbox(&tx)?;
    let mut run = Run {
        conn: &tx,
        applied: Applied::default(),
        deferred: BTreeSet::new(),
        deferred_collections: BTreeSet::new(),
    };

    // Phase 1: Aliase zuerst (auf die Wurzel im Batch aufgelöst), dann die
    // übrigen Grabsteine, damit sie auch umbenannte Zeilen treffen.
    let mut video_roots = HashMap::new();
    let mut collection_roots = HashMap::new();
    for (_, state) in &states {
        match state {
            State::VideoGone {
                uid,
                reason: GoneReason::Merged,
                merged_into: Some(root),
            } => {
                video_roots.insert(uid.clone(), root.clone());
            }
            State::CollectionGone {
                uid,
                reason: GoneReason::Merged,
                merged_into: Some(root),
            } => {
                collection_roots.insert(uid.clone(), root.clone());
            }
            _ => {}
        }
    }
    for (_, state) in &states {
        if let State::VideoGone { uid, .. } = state {
            if video_roots.contains_key(uid) {
                run.video_merged(uid, &resolve_root(&video_roots, uid))?;
                steps.pass()?;
            }
        }
    }
    for (_, state) in &states {
        if let State::CollectionGone { uid, .. } = state {
            if collection_roots.contains_key(uid) {
                run.collection_merged(uid, &resolve_root(&collection_roots, uid))?;
                steps.pass()?;
            }
        }
    }
    for rank in 0..4 {
        for (_, state) in &states {
            let handled = match (rank, state) {
                (0, State::VideoGone { uid, reason, .. }) if !video_roots.contains_key(uid) => {
                    run.video_gone(uid, *reason)?;
                    true
                }
                (1, State::CollectionGone { uid, .. }) if !collection_roots.contains_key(uid) => {
                    run.collection_gone(uid)?;
                    true
                }
                (2, State::ChatGone { uid }) => {
                    run.chat_gone(uid)?;
                    true
                }
                (3, State::SummaryGone { uid }) => {
                    run.summary_gone(uid)?;
                    true
                }
                _ => false,
            };
            if handled {
                steps.pass()?;
            }
        }
    }

    // Phase 2: collection, video, summary, chat, round, membership.
    let collections: Vec<(i64, &CollectionData)> = states
        .iter()
        .filter_map(|(seq, state)| match state {
            State::Collection(data) => Some((*seq, data)),
            _ => None,
        })
        .collect();
    run.collections(collections)?;
    steps.pass()?;
    for rank in 0..5 {
        for (seq, state) in &states {
            let handled = match (rank, state) {
                (0, State::Video(data)) => run.video(data).map(|_| true)?,
                (1, State::Summary(data)) => run.summary(data).map(|_| true)?,
                (2, State::Chat(data)) => run.chat(data).map(|_| true)?,
                (3, State::Round(data)) => run.round(data).map(|_| true)?,
                (
                    4,
                    State::Membership {
                        video_uid,
                        collection_uid,
                        present,
                    },
                ) => run
                    .membership(*seq, video_uid, collection_uid, *present)
                    .map(|_| true)?,
                _ => false,
            };
            if handled {
                steps.pass()?;
            }
        }
    }

    let now = now_canonical();
    for video_id in run.applied.video_ids.clone() {
        resolve_pending_summaries(&tx, video_id)?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM videos WHERE id = ?1)",
                params![video_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if exists {
            storage::refresh_latest_summary(&tx, video_id, &now)?;
        }
    }
    steps.pass()?;

    for seq in loaded {
        if !run.deferred.contains(&seq) {
            tx.execute("DELETE FROM sync_inbox WHERE seq = ?1", params![seq])
                .map_err(db_error)?;
        }
    }
    tx.execute("DELETE FROM sync_state WHERE key = 'applying'", [])
        .map_err(db_error)?;
    steps.pass()?;
    let applied = run.applied;
    tx.commit().map_err(db_error)?;
    Ok(applied)
}

/// Verdichtete Inbox (höchste `seq` je Schlüssel, nach `seq` sortiert) und
/// alle geladenen `seq`s. Unlesbare Zustände werden verworfen.
fn load_inbox(conn: &Connection) -> AppResult<(Vec<(i64, State)>, Vec<i64>)> {
    let mut stmt = conn
        .prepare("SELECT seq, entity, key, payload FROM sync_inbox ORDER BY seq")
        .map_err(db_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(db_error)?;
    let mut latest: HashMap<(String, String), (i64, State)> = HashMap::new();
    let mut loaded = Vec::new();
    for row in rows {
        let (seq, entity, key, payload) = row.map_err(db_error)?;
        loaded.push(seq);
        match serde_json::from_str::<State>(&payload) {
            Ok(state) => {
                latest.insert((entity, key), (seq, state));
            }
            Err(err) => eprintln!("Sync: Zustand {seq} ist unlesbar und wird verworfen: {err}"),
        }
    }
    let mut states: Vec<(i64, State)> = latest.into_values().collect();
    states.sort_by_key(|(seq, _)| *seq);
    Ok((states, loaded))
}

/// Folgt Alias-Ketten innerhalb des Batches bis zur Wurzel.
fn resolve_root(roots: &HashMap<String, String>, uid: &str) -> String {
    let mut current = uid;
    for _ in 0..=roots.len() {
        match roots.get(current) {
            Some(next) => current = next,
            None => break,
        }
    }
    current.to_string()
}

struct Run<'a> {
    conn: &'a Connection,
    applied: Applied,
    /// `seq`s, die in der Inbox bleiben.
    deferred: BTreeSet<i64>,
    /// uids zurückgestellter Sammlungen (ihre Zuordnungen warten mit).
    deferred_collections: BTreeSet<String>,
}

impl Run<'_> {
    fn pending(&self, entity: &str, key: &str) -> AppResult<bool> {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sync_outbox WHERE entity = ?1 AND key = ?2)",
                params![entity, key],
                |row| row.get(0),
            )
            .map_err(db_error)
    }

    /// `(id, local_only)` des Videos mit dieser uid.
    fn video_row(&self, uid: &str) -> AppResult<Option<(i64, bool)>> {
        self.conn
            .query_row(
                "SELECT id, local_only FROM videos WHERE uid = ?1",
                params![uid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(db_error)
    }

    /// Lokale id eines geteilten Videos; `None` bei privatem oder fehlendem.
    fn shared_video(&self, uid: &str) -> AppResult<Option<i64>> {
        Ok(match self.video_row(uid)? {
            Some((id, false)) => Some(id),
            _ => None,
        })
    }

    fn collection(&self, uid: &str) -> AppResult<Option<i64>> {
        self.conn
            .query_row(
                "SELECT id FROM collections WHERE uid = ?1",
                params![uid],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)
    }

    fn execute(&self, sql: &str, values: impl rusqlite::Params) -> AppResult<usize> {
        self.conn.execute(sql, values).map_err(db_error)
    }

    // --- Phase 1 --------------------------------------------------------

    fn video_gone(&mut self, uid: &str, reason: GoneReason) -> AppResult<()> {
        let Some((id, _)) = self.video_row(uid)? else {
            return Ok(());
        };
        if reason == GoneReason::Withdrawn {
            privatize::make_private(self.conn, id, false)?;
        } else {
            self.delete_video(id, uid)?;
        }
        self.applied.video_ids.insert(id);
        Ok(())
    }

    fn delete_video(&mut self, id: i64, uid: &str) -> AppResult<()> {
        self.execute("DELETE FROM sync_outbox WHERE owner = ?1", params![uid])?;
        self.execute("DELETE FROM videos WHERE id = ?1", params![id])?;
        self.applied.video_ids.insert(id);
        Ok(())
    }

    fn collection_gone(&mut self, uid: &str) -> AppResult<()> {
        if let Some(id) = self.collection(uid)? {
            self.delete_collection(id, uid)?;
        }
        Ok(())
    }

    fn delete_collection(&mut self, id: i64, uid: &str) -> AppResult<()> {
        self.execute(
            "DELETE FROM sync_outbox WHERE (entity = 'collection' AND key = ?1) \
             OR (entity = 'membership' AND parent = ?1)",
            params![uid],
        )?;
        self.execute("DELETE FROM collections WHERE id = ?1", params![id])?;
        self.applied.collections = true;
        Ok(())
    }

    fn chat_gone(&mut self, uid: &str) -> AppResult<()> {
        let video_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT video_id FROM chats WHERE uid = ?1",
                params![uid],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let Some(video_id) = video_id else {
            return Ok(());
        };
        self.execute(
            "DELETE FROM sync_outbox WHERE (entity = 'chat' AND key = ?1) \
             OR (entity = 'round' AND parent = ?1)",
            params![uid],
        )?;
        self.execute("DELETE FROM chats WHERE uid = ?1", params![uid])?;
        self.applied.video_ids.insert(video_id);
        Ok(())
    }

    fn summary_gone(&mut self, uid: &str) -> AppResult<()> {
        let video_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT video_id FROM summaries WHERE uid = ?1",
                params![uid],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let Some(video_id) = video_id else {
            return Ok(());
        };
        self.execute(
            "DELETE FROM sync_outbox WHERE entity = 'summary' AND key = ?1",
            params![uid],
        )?;
        self.execute("DELETE FROM summaries WHERE uid = ?1", params![uid])?;
        self.applied.video_ids.insert(video_id);
        Ok(())
    }

    // --- Phase 2 --------------------------------------------------------

    /// Sammlungen zweiphasig: Kollidiert der Zielname (`name_key`) mit einer
    /// lokalen Sammlung, die in diesem Batch ihren Namen behält, wird der
    /// Zustand zurückgestellt (bis zum Fixpunkt, da Zurückstellen weitere
    /// Namen blockieren kann). Danach geänderte Namen zuerst auf
    /// `\u{1}<uid>`, dann die Endnamen – so stören Tausche den
    /// `NOCASE`-Index nicht.
    fn collections(&mut self, states: Vec<(i64, &CollectionData)>) -> AppResult<()> {
        let mut candidates = Vec::new();
        for (seq, data) in states {
            if !self.pending("collection", &data.uid)? {
                candidates.push((seq, data));
            }
        }
        let local: Vec<(String, String)> = {
            let mut stmt = self
                .conn
                .prepare("SELECT uid, name FROM collections")
                .map_err(db_error)?;
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(db_error)?;
            rows.collect::<Result<_, _>>().map_err(db_error)?
        };
        loop {
            let renamed: BTreeSet<&str> = candidates.iter().map(|(_, d)| d.uid.as_str()).collect();
            let blocked = candidates.iter().position(|(_, data)| {
                local.iter().any(|(uid, name)| {
                    uid != &data.uid
                        && !renamed.contains(uid.as_str())
                        && name_key(name) == name_key(&data.name)
                })
            });
            let Some(index) = blocked else {
                break;
            };
            let (seq, data) = candidates.remove(index);
            self.deferred.insert(seq);
            self.deferred_collections.insert(data.uid.clone());
        }
        for (_, data) in &candidates {
            self.execute(
                "UPDATE collections SET name = char(1) || uid WHERE uid = ?1 AND name != ?2",
                params![data.uid, data.name],
            )?;
        }
        let now = now_canonical();
        for (_, data) in &candidates {
            let updated = self.execute(
                "UPDATE collections SET name = ?1, updated_at = ?2 WHERE uid = ?3 AND name != ?1",
                params![data.name, now, data.uid],
            )?;
            if self.collection(&data.uid)?.is_none() {
                self.execute(
                    "INSERT INTO collections (uid, name, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4)",
                    params![data.uid, data.name, data.created_at, now],
                )?;
                self.applied.collections = true;
            } else if updated > 0 {
                self.applied.collections = true;
            }
        }
        Ok(())
    }

    fn video(&mut self, data: &VideoData) -> AppResult<()> {
        if self.pending("video", &data.uid)? {
            return Ok(());
        }
        let thumbnail = data
            .thumbnail_data
            .as_deref()
            .and_then(|encoded| BASE64.decode(encoded).ok());
        let now = now_canonical();
        match self.video_row(&data.uid)? {
            // Tritt nur durch einen Wettlauf auf.
            Some((_, true)) => {}
            Some((id, false)) => {
                self.execute(
                    "UPDATE videos SET url = ?1, title = ?2, thumbnail_url = ?3, \
                     thumbnail_data = ?4, transcript = ?5, chapters = ?6, published_at = ?7, \
                     description = ?8, transcript_error = ?9, updated_at = ?10 WHERE id = ?11",
                    params![
                        data.url,
                        data.title,
                        data.thumbnail_url,
                        thumbnail,
                        data.transcript,
                        data.chapters,
                        data.published_at,
                        data.description,
                        data.transcript_error,
                        now,
                        id
                    ],
                )?;
                self.applied.video_ids.insert(id);
            }
            // Auch wenn eine private Kopie dieselbe YouTube-ID hat.
            None => {
                self.execute(
                    "INSERT INTO videos (uid, video_id, url, title, thumbnail_url, thumbnail_data, \
                     transcript, chapters, published_at, description, transcript_error, \
                     created_at, updated_at, local_only, published) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0, 1)",
                    params![
                        data.uid,
                        data.youtube_id,
                        data.url,
                        data.title,
                        data.thumbnail_url,
                        thumbnail,
                        data.transcript,
                        data.chapters,
                        data.published_at,
                        data.description,
                        data.transcript_error,
                        data.created_at,
                        now
                    ],
                )?;
                self.applied.video_ids.insert(self.conn.last_insert_rowid());
            }
        }
        Ok(())
    }

    fn summary(&mut self, data: &SummaryData) -> AppResult<()> {
        if self.pending("summary", &data.uid)? {
            return Ok(());
        }
        let Some(video_id) = self.shared_video(&data.video_uid)? else {
            return Ok(());
        };
        let known: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM summaries WHERE uid = ?1)",
                params![data.uid],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if !known {
            self.execute(
                "INSERT INTO summaries (uid, video_id, created_at, summary, provider, model, options) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    data.uid,
                    video_id,
                    data.created_at,
                    data.summary,
                    data.provider,
                    data.model,
                    data.options
                ],
            )?;
            self.applied.video_ids.insert(video_id);
        }
        Ok(())
    }

    fn chat(&mut self, data: &ChatData) -> AppResult<()> {
        if self.pending("chat", &data.uid)? {
            return Ok(());
        }
        let Some(video_id) = self.shared_video(&data.video_uid)? else {
            return Ok(());
        };
        let options = local_context_options(self.conn, video_id, data)?;
        let updated = self.execute(
            "UPDATE chats SET title = ?1, updated_at = ?2, context_options = ?3 \
             WHERE uid = ?4 AND video_id = ?5",
            params![data.title, data.updated_at, options, data.uid, video_id],
        )?;
        if updated == 0 {
            let exists: bool = self
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM chats WHERE uid = ?1)",
                    params![data.uid],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if exists {
                // Gehört lokal zu einem anderen Video: Eltern sind unveränderlich.
                return Ok(());
            }
            self.execute(
                "INSERT INTO chats (uid, video_id, title, created_at, updated_at, context_options) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    data.uid,
                    video_id,
                    data.title,
                    data.created_at,
                    data.updated_at,
                    options
                ],
            )?;
        }
        self.applied.video_ids.insert(video_id);
        Ok(())
    }

    fn round(&mut self, data: &RoundData) -> AppResult<()> {
        if self.pending("round", &data.uid)? {
            return Ok(());
        }
        let Some(video_id) = self.shared_video(&data.video_uid)? else {
            return Ok(());
        };
        let chat_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM chats WHERE uid = ?1 AND video_id = ?2",
                params![data.chat_uid, video_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        let Some(chat_id) = chat_id else {
            return Ok(());
        };
        let known: bool = self
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM chat_messages WHERE round_uid = ?1)",
                params![data.uid],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if known {
            return Ok(());
        }
        for (position, message) in data.messages.iter().enumerate() {
            self.execute(
                "INSERT INTO chat_messages (chat_id, round_uid, position, role, content, \
                 tool_calls, tool_call_id, provider, model, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    chat_id,
                    data.uid,
                    position as i64,
                    message.role,
                    message.content,
                    message.tool_calls.as_ref().map(Value::to_string),
                    message.tool_call_id,
                    message.provider,
                    message.model,
                    data.created_at
                ],
            )?;
        }
        self.applied.video_ids.insert(video_id);
        Ok(())
    }

    fn membership(
        &mut self,
        seq: i64,
        video_uid: &str,
        collection_uid: &str,
        present: bool,
    ) -> AppResult<()> {
        if self.pending(
            "membership",
            &sync_proto::membership_key(video_uid, collection_uid),
        )? {
            return Ok(());
        }
        if self.deferred_collections.contains(collection_uid) {
            self.deferred.insert(seq);
            return Ok(());
        }
        let (Some(video_id), Some(collection_id)) = (
            self.shared_video(video_uid)?,
            self.collection(collection_uid)?,
        ) else {
            return Ok(());
        };
        let changed = if present {
            self.execute(
                "INSERT OR IGNORE INTO video_collections (video_id, collection_id, created_at) \
                 VALUES (?1, ?2, ?3)",
                params![video_id, collection_id, now_canonical()],
            )?
        } else {
            self.execute(
                "DELETE FROM video_collections WHERE video_id = ?1 AND collection_id = ?2",
                params![video_id, collection_id],
            )?
        };
        if changed > 0 {
            self.applied.collections = true;
            self.applied.video_ids.insert(video_id);
        }
        Ok(())
    }
}

/// `summaryUids` → lokale ids desselben Videos; unbekannte uids warten in
/// `pendingSummaryUids`. `null` und `[]` bleiben, wie sie sind.
fn local_context_options(conn: &Connection, video_id: i64, data: &ChatData) -> AppResult<String> {
    let transcript = data.context_options.transcript;
    let Some(uids) = &data.context_options.summary_uids else {
        return Ok(json!({"transcript": transcript, "summaryIds": null}).to_string());
    };
    let mut ids = Vec::new();
    let mut pending = Vec::new();
    for uid in uids {
        match summary_id(conn, video_id, uid)? {
            Some(id) => ids.push(id),
            None => pending.push(uid.clone()),
        }
    }
    let mut options = json!({"transcript": transcript, "summaryIds": ids});
    if !pending.is_empty() {
        options["pendingSummaryUids"] = json!(pending);
    }
    Ok(options.to_string())
}

fn summary_id(conn: &Connection, video_id: i64, uid: &str) -> AppResult<Option<i64>> {
    conn.query_row(
        "SELECT id FROM summaries WHERE uid = ?1 AND video_id = ?2",
        params![uid, video_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_error)
}

/// Löst `pendingSummaryUids` der Chats eines Videos gegen die jetzt
/// vorhandenen Summaries desselben Videos auf.
fn resolve_pending_summaries(conn: &Connection, video_id: i64) -> AppResult<()> {
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
    for (chat_id, raw) in chats {
        let Ok(mut options) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let pending: Vec<String> = options
            .get("pendingSummaryUids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|uid| uid.as_str().map(str::to_string))
            .collect();
        let mut still_pending = Vec::new();
        let mut resolved = Vec::new();
        for uid in pending {
            match summary_id(conn, video_id, &uid)? {
                Some(id) => resolved.push(id),
                None => still_pending.push(uid),
            }
        }
        if resolved.is_empty() {
            continue;
        }
        if let Some(Value::Array(ids)) = options.get_mut("summaryIds") {
            ids.extend(resolved.into_iter().map(Value::from));
        }
        let Some(map) = options.as_object_mut() else {
            continue;
        };
        if still_pending.is_empty() {
            map.remove("pendingSummaryUids");
        } else {
            map.insert("pendingSummaryUids".into(), json!(still_pending));
        }
        conn.execute(
            "UPDATE chats SET context_options = ?1 WHERE id = ?2",
            params![options.to_string(), chat_id],
        )
        .map_err(db_error)?;
    }
    Ok(())
}
