use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub time: String,
    pub start: f64,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSnippet {
    pub text: String,
    pub start: f64,
    pub time: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Video {
    pub id: i64,
    pub video_id: String,
    pub url: String,
    pub title: String,
    pub thumbnail_url: String,
    pub thumbnail: Option<String>,
    pub transcript: Option<String>,
    pub chapters: Option<Vec<Chapter>>,
    pub summary: Option<String>,
    pub summary_provider: Option<String>,
    pub summary_model: Option<String>,
    pub published_at: Option<String>,
    pub description: Option<String>,
    pub collection_ids: Vec<i64>,
    pub created_at: String,
    pub updated_at: String,
    pub transcript_error: Option<String>,
    pub has_transcript: bool,
    pub has_summary: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub video_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub id: i64,
    pub video_id: i64,
    pub created_at: String,
    pub summary: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub options: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    pub id: i64,
    pub video_id: i64,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessageRecord {
    pub id: i64,
    pub chat_id: i64,
    pub role: String,
    pub content: String,
    pub tool_calls: Option<Value>,
    pub tool_call_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub created_at: String,
}

/// Chat-Nachricht ohne Speicheridentitaet: Eingabe fuer `append_chat_turn`
/// und Verlaufsnachricht beim Prompt-Aufbau.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewChatMessage {
    pub role: String,
    pub content: String,
    pub tool_calls: Option<Value>,
    pub tool_call_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
}

impl NewChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            provider: None,
            model: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
            provider: None,
            model: None,
        }
    }

    /// Verliert die Speicheridentitaet (id/chatId/createdAt) einer gespeicherten
    /// Nachricht; fuer den Prompt-Aufbau werden nur die Inhaltsfelder gebraucht.
    pub fn from_record(record: ChatMessageRecord) -> Self {
        Self {
            role: record.role,
            content: record.content,
            tool_calls: record.tool_calls,
            tool_call_id: record.tool_call_id,
            provider: record.provider,
            model: record.model,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatTurnResult {
    pub chat: Chat,
    pub messages: Vec<ChatMessageRecord>,
}

#[derive(Debug, Clone)]
pub struct NewVideo {
    pub video_id: String,
    pub url: String,
    pub title: String,
    pub thumbnail_url: String,
    pub thumbnail_data: Option<Vec<u8>>,
    pub transcript: Option<String>,
    pub chapters: Option<String>,
    pub published_at: Option<String>,
    pub description: Option<String>,
    pub transcript_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VideoInfo {
    pub title: String,
    pub thumbnail_url: String,
    pub published_at: Option<String>,
}
