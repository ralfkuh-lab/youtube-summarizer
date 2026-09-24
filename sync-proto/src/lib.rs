//! Gemeinsame Protokolltypen für den Sync zwischen App und Sync-Server.
//!
//! Verbindlich ist `docs/spec-sync.md` (Abschnitt „Protokoll“). Dieses Crate
//! enthält nur Typen, Konstanten und strukturelle Prüfungen – keine
//! Zusammenführungslogik und keinen Speicherzugriff.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;

pub const HEADER_PROTOCOL: &str = "X-Sync-Protocol";
pub const HEADER_DATASET: &str = "X-Sync-Dataset";

pub const MAX_OPS_PER_PUSH: usize = 500;
pub const MAX_PUSH_BODY_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_OP_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_THUMBNAIL_BYTES: usize = 2 * 1024 * 1024;
pub const PULL_PAGE_BYTES: usize = 8 * 1024 * 1024;
pub const PULL_PAGE_STATES: usize = 500;
pub const MAX_COLLECTION_NAME_CHARS: usize = 80;

/// Rollen, die eine Chat-Runde enthalten darf (wie in `chat.rs`).
pub const CHAT_ROLES: [&str; 3] = ["user", "assistant", "tool"];

// ---------------------------------------------------------------------------
// Nutzdaten
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoData {
    pub uid: String,
    pub youtube_id: String,
    pub url: String,
    pub title: String,
    pub thumbnail_url: String,
    /// Base64 (Standard-Alphabet) des JPEG, dekodiert höchstens
    /// [`MAX_THUMBNAIL_BYTES`].
    pub thumbnail_data: Option<String>,
    /// Transkript als JSON-Text, wie lokal gespeichert.
    pub transcript: Option<String>,
    /// Kapitel als JSON-Text, wie lokal gespeichert.
    pub chapters: Option<String>,
    pub published_at: Option<String>,
    pub description: Option<String>,
    pub transcript_error: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryData {
    pub uid: String,
    pub video_uid: String,
    pub created_at: String,
    pub summary: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub options: Option<String>,
}

/// `summaryUids: null` = neueste Zusammenfassung, `[]` = keine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextOptions {
    pub transcript: bool,
    pub summary_uids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatData {
    pub uid: String,
    pub video_uid: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub context_options: ContextOptions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundMessage {
    pub role: String,
    pub content: String,
    pub tool_calls: Option<Value>,
    pub tool_call_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
}

/// Eine vollständige, unveränderliche Chat-Runde; `messages` in Positionsreihenfolge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundData {
    pub uid: String,
    pub chat_uid: String,
    pub video_uid: String,
    pub created_at: String,
    pub messages: Vec<RoundMessage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionData {
    pub uid: String,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoneReason {
    Deleted,
    Withdrawn,
    Merged,
}

// ---------------------------------------------------------------------------
// Operationen (Client -> Server)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Op {
    #[serde(rename_all = "camelCase")]
    Video {
        #[serde(flatten)]
        data: VideoData,
        changed_at: String,
    },
    /// `reason` ist `deleted` oder `withdrawn`, nie `merged`.
    #[serde(rename_all = "camelCase")]
    VideoGone {
        uid: String,
        reason: GoneReason,
    },
    Summary(SummaryData),
    #[serde(rename_all = "camelCase")]
    SummaryDelete {
        uid: String,
        video_uid: String,
    },
    #[serde(rename_all = "camelCase")]
    Chat {
        #[serde(flatten)]
        data: ChatData,
        changed_at: String,
    },
    #[serde(rename_all = "camelCase")]
    ChatDelete {
        uid: String,
        video_uid: String,
    },
    Round(RoundData),
    #[serde(rename_all = "camelCase")]
    Collection {
        #[serde(flatten)]
        data: CollectionData,
        changed_at: String,
    },
    #[serde(rename_all = "camelCase")]
    CollectionDelete {
        uid: String,
    },
    #[serde(rename_all = "camelCase")]
    Membership {
        video_uid: String,
        collection_uid: String,
        present: bool,
        changed_at: String,
    },
}

impl Op {
    /// Löschungen laufen im Push in Phase 1 und antworten nie mit `retry`.
    pub fn is_delete(&self) -> bool {
        matches!(
            self,
            Op::VideoGone { .. }
                | Op::SummaryDelete { .. }
                | Op::ChatDelete { .. }
                | Op::CollectionDelete { .. }
        )
    }

    /// Strukturelle Prüfung an der Vertrauensgrenze (ohne Beziehungen, die
    /// nur der Server mit seinem Bestand prüfen kann).
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Op::Video { data, changed_at } => {
                check_video(data)?;
                check_time("changedAt", changed_at)
            }
            Op::VideoGone { uid, reason } => {
                check_uid("uid", uid)?;
                if *reason == GoneReason::Merged {
                    return Err("reason: merged ist keine Client-Operation".into());
                }
                Ok(())
            }
            Op::Summary(data) => check_summary(data),
            Op::SummaryDelete { uid, video_uid } | Op::ChatDelete { uid, video_uid } => {
                check_uid("uid", uid)?;
                check_uid("videoUid", video_uid)
            }
            Op::Chat { data, changed_at } => {
                check_chat(data)?;
                check_time("changedAt", changed_at)
            }
            Op::Round(data) => check_round(data),
            Op::Collection { data, changed_at } => {
                check_collection(data)?;
                check_time("changedAt", changed_at)
            }
            Op::CollectionDelete { uid } => check_uid("uid", uid),
            Op::Membership {
                video_uid,
                collection_uid,
                changed_at,
                ..
            } => {
                check_uid("videoUid", video_uid)?;
                check_uid("collectionUid", collection_uid)?;
                check_time("changedAt", changed_at)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushRequest {
    pub ops: Vec<Op>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpStatus {
    Ok,
    Rejected,
    Retry,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpResult {
    pub status: OpStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Bei `retry`: das unbekannte Elternobjekt als `<entity>/<uid>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<String>,
}

/// Antwort auf `POST /v1/push`; `results` hat Länge und Reihenfolge von `ops`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushResponse {
    pub results: Vec<OpResult>,
}

// ---------------------------------------------------------------------------
// Zustände (Server -> Client)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum State {
    Video(VideoData),
    #[serde(rename_all = "camelCase")]
    VideoGone {
        uid: String,
        reason: GoneReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        merged_into: Option<String>,
    },
    Summary(SummaryData),
    #[serde(rename_all = "camelCase")]
    SummaryGone {
        uid: String,
    },
    Chat(ChatData),
    #[serde(rename_all = "camelCase")]
    ChatGone {
        uid: String,
    },
    Round(RoundData),
    Collection(CollectionData),
    #[serde(rename_all = "camelCase")]
    CollectionGone {
        uid: String,
        reason: GoneReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        merged_into: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Membership {
        video_uid: String,
        collection_uid: String,
        present: bool,
    },
}

impl State {
    /// `(entity, key)` wie in der Client-Outbox; membership: `<video>/<collection>`.
    pub fn entity_key(&self) -> (&'static str, String) {
        match self {
            State::Video(d) => ("video", d.uid.clone()),
            State::VideoGone { uid, .. } => ("video", uid.clone()),
            State::Summary(d) => ("summary", d.uid.clone()),
            State::SummaryGone { uid } => ("summary", uid.clone()),
            State::Chat(d) => ("chat", d.uid.clone()),
            State::ChatGone { uid } => ("chat", uid.clone()),
            State::Round(d) => ("round", d.uid.clone()),
            State::Collection(d) => ("collection", d.uid.clone()),
            State::CollectionGone { uid, .. } => ("collection", uid.clone()),
            State::Membership {
                video_uid,
                collection_uid,
                ..
            } => ("membership", membership_key(video_uid, collection_uid)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerState {
    pub seq: i64,
    #[serde(flatten)]
    pub state: State,
}

/// Antwort auf `GET /v1/pull?since=`; `next` = höchste `seq` der Seite, bei
/// leerer Seite `since`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullResponse {
    pub states: Vec<ServerState>,
    pub next: i64,
    pub more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: String,
    pub protocol: u32,
    pub dataset_id: String,
}

/// Fehlerkörper für 4xx: `error` ist z. B. `datasetMismatch`, `cursorAhead`,
/// `protocol`, `unauthorized`, `invalid`, `tooLarge`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_id: Option<String>,
    /// Bei 400: Index der ungültigen Operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<u32>,
}

// ---------------------------------------------------------------------------
// Hilfsfunktionen
// ---------------------------------------------------------------------------

pub fn membership_key(video_uid: &str, collection_uid: &str) -> String {
    format!("{video_uid}/{collection_uid}")
}

/// Kanonische Form eines Zeitpunkts: UTC, Millisekunden, `Z`
/// (`2026-09-24T12:34:56.789Z`). Akzeptiert jedes RFC-3339-Format.
pub fn canonical_time(value: &str) -> Result<String, String> {
    DateTime::parse_from_rfc3339(value.trim())
        .map(|time| {
            time.with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Millis, true)
        })
        .map_err(|err| format!("ungültiger Zeitstempel '{value}': {err}"))
}

/// Aktueller Zeitpunkt in kanonischer Form.
pub fn now_canonical() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn is_canonical_time(value: &str) -> bool {
    canonical_time(value).is_ok_and(|canonical| canonical == value)
}

/// Namensschlüssel für Sammlungen: Trim + ASCII-Kleinschreibung (entspricht
/// dem lokalen `COLLATE NOCASE`, das nur ASCII faltet).
pub fn name_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

pub fn is_uid(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn is_youtube_id(value: &str) -> bool {
    value.len() == 11
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn check_uid(field: &str, value: &str) -> Result<(), String> {
    if is_uid(value) {
        Ok(())
    } else {
        Err(format!("{field}: ungültige uid"))
    }
}

fn check_time(field: &str, value: &str) -> Result<(), String> {
    if is_canonical_time(value) {
        Ok(())
    } else {
        Err(format!("{field}: Zeitstempel nicht kanonisch"))
    }
}

fn check_video(data: &VideoData) -> Result<(), String> {
    check_uid("uid", &data.uid)?;
    if !is_youtube_id(&data.youtube_id) {
        return Err("youtubeId: ungültig".into());
    }
    check_time("createdAt", &data.created_at)?;
    if let Some(encoded) = &data.thumbnail_data {
        let bytes = BASE64
            .decode(encoded)
            .map_err(|_| "thumbnailData: kein gültiges Base64".to_string())?;
        if bytes.len() > MAX_THUMBNAIL_BYTES {
            return Err("thumbnailData: zu groß".into());
        }
    }
    for (field, value) in [
        ("transcript", &data.transcript),
        ("chapters", &data.chapters),
    ] {
        if let Some(text) = value {
            serde_json::from_str::<Value>(text)
                .map_err(|_| format!("{field}: kein gültiges JSON"))?;
        }
    }
    Ok(())
}

fn check_summary(data: &SummaryData) -> Result<(), String> {
    check_uid("uid", &data.uid)?;
    check_uid("videoUid", &data.video_uid)?;
    check_time("createdAt", &data.created_at)
}

fn check_chat(data: &ChatData) -> Result<(), String> {
    check_uid("uid", &data.uid)?;
    check_uid("videoUid", &data.video_uid)?;
    check_time("createdAt", &data.created_at)?;
    check_time("updatedAt", &data.updated_at)?;
    if let Some(uids) = &data.context_options.summary_uids {
        for uid in uids {
            check_uid("contextOptions.summaryUids", uid)?;
        }
    }
    Ok(())
}

fn check_round(data: &RoundData) -> Result<(), String> {
    check_uid("uid", &data.uid)?;
    check_uid("chatUid", &data.chat_uid)?;
    check_uid("videoUid", &data.video_uid)?;
    check_time("createdAt", &data.created_at)?;
    if data.messages.is_empty() {
        return Err("messages: leer".into());
    }
    for message in &data.messages {
        if !CHAT_ROLES.contains(&message.role.as_str()) {
            return Err(format!("messages.role: unbekannt '{}'", message.role));
        }
    }
    Ok(())
}

fn check_collection(data: &CollectionData) -> Result<(), String> {
    check_uid("uid", &data.uid)?;
    check_time("createdAt", &data.created_at)?;
    let trimmed = data.name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_COLLECTION_NAME_CHARS {
        return Err("name: leer oder zu lang".into());
    }
    Ok(())
}

/// Prüft einen ganzen Push; Fehler mit Index der ersten ungültigen Operation.
/// `publishedAt` ist von YouTube frei formatiert und wird nicht geprüft.
pub fn validate_all(ops: &[Op]) -> Result<(), (usize, String)> {
    if ops.len() > MAX_OPS_PER_PUSH {
        return Err((MAX_OPS_PER_PUSH, "zu viele Operationen".into()));
    }
    for (index, op) in ops.iter().enumerate() {
        op.validate().map_err(|message| (index, message))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
