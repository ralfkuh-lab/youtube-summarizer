//! Fixture V/W/X der Kontextauswahl (Revision 3) fuer `agent_prepare`.

use crate::models::NewChatMessage;
use crate::storage::AppPaths;
use tempfile::TempDir;

use super::super::config::{self, AgentConfig};
use super::{integration_video, temp_paths};

/// Fixture V: Video mit Transkript, Metadaten und `videos.summary`, drei
/// Versionen S1 < S2 < S3 (`created_at`) und zwei Chats C1 < C2; C1 enthaelt
/// zusaetzlich eine Tool-Nachricht und einen leeren Assistant-Turn. Fixture W:
/// zweites Video mit einer Version und einem Chat. Fixture X: Video ohne
/// Transkript, ohne Zusammenfassung, ohne Chat.
pub(crate) struct Revision3 {
    pub video: i64,
    /// S1, S2, S3 — aelteste zuerst.
    pub summaries: [i64; 3],
    /// C1, C2 — aelteste zuerst.
    pub chats: [i64; 2],
    pub other_video: i64,
    pub other_summary: i64,
    pub other_chat: i64,
    pub bare_video: i64,
    pub base: std::path::PathBuf,
}

pub(crate) fn revision3_fixture() -> (TempDir, AppPaths, Revision3) {
    let (temp, paths) = temp_paths();
    let base = temp.path().join("agent");
    let config = AgentConfig {
        workdir_base: base.to_string_lossy().into_owned(),
        ..AgentConfig::default()
    };
    config::save(&paths, &config).unwrap();

    let mut metadata = integration_video("vidV00000001", "Video V");
    metadata.published_at = Some("2026-01-02".to_string());
    metadata.description = Some("Beschreibung V".to_string());
    metadata.chapters =
        Some(serde_json::json!([{"time": "0:00", "start": 0.0, "title": "Start"}]).to_string());
    let video = crate::storage::insert_video(&paths, metadata, false).unwrap();

    let s1 = add_summary(&paths, video.id, "Version eins", "2026-01-01T10:00:00Z");
    let s2 = add_summary(&paths, video.id, "Version zwei", "2026-02-01T10:00:00Z");
    let s3 = add_summary(&paths, video.id, "Version drei", "2026-03-01T10:00:00Z");

    let c1 = add_chat(
        &paths,
        video.id,
        "Chat Eins",
        "2026-01-05T10:00:00Z",
        vec![
            NewChatMessage::user("Frage eins"),
            tool_call_message(),
            tool_result_message(),
            NewChatMessage::assistant("Antwort eins"),
            NewChatMessage::assistant("   "),
        ],
    );
    let c2 = add_chat(
        &paths,
        video.id,
        "Chat Zwei",
        "2026-02-05T10:00:00Z",
        vec![NewChatMessage::user("Frage zwei")],
    );

    let other =
        crate::storage::insert_video(&paths, integration_video("vidW00000002", "Video W"), false)
            .unwrap();
    let other_summary = add_summary(&paths, other.id, "Version W", "2026-04-01T10:00:00Z");
    let other_chat = add_chat(
        &paths,
        other.id,
        "Chat W",
        "2026-04-05T10:00:00Z",
        vec![NewChatMessage::user("Frage W")],
    );

    let mut bare = integration_video("vidX00000003", "Video X");
    bare.transcript = None;
    let bare_video = crate::storage::insert_video(&paths, bare, false)
        .unwrap()
        .id;

    (
        temp,
        paths,
        Revision3 {
            video: video.id,
            summaries: [s1, s2, s3],
            chats: [c1, c2],
            other_video: other.id,
            other_summary,
            other_chat,
            bare_video,
            base,
        },
    )
}

/// Assistant-Turn mit Tool-Aufruf (kein exportierbarer Text).
pub(crate) fn tool_call_message() -> NewChatMessage {
    NewChatMessage {
        role: "assistant".to_string(),
        content: String::new(),
        tool_calls: Some(serde_json::json!([{"id": "1", "type": "function"}])),
        tool_call_id: None,
        provider: None,
        model: None,
    }
}

/// Tool-Ergebnis (kein exportierbarer Text).
pub(crate) fn tool_result_message() -> NewChatMessage {
    NewChatMessage {
        role: "tool".to_string(),
        content: "Suchergebnis".to_string(),
        tool_calls: None,
        tool_call_id: Some("1".to_string()),
        provider: None,
        model: None,
    }
}

/// Legt eine Zusammenfassung an und setzt `created_at` ausdruecklich: die
/// Reihenfolge der Versionen ist Teil der Referenzfaelle.
pub(crate) fn add_summary(paths: &AppPaths, video_id: i64, text: &str, created_at: &str) -> i64 {
    crate::storage::update_summary(
        paths,
        video_id,
        text,
        Some("Anbieter"),
        Some("Modell"),
        None,
    )
    .unwrap();
    let conn = rusqlite::Connection::open(&paths.db_path).unwrap();
    let id: i64 = conn
        .query_row(
            "SELECT id FROM summaries WHERE video_id = ?1 AND summary = ?2",
            rusqlite::params![video_id, text],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute(
        "UPDATE summaries SET created_at = ?1 WHERE id = ?2",
        rusqlite::params![created_at, id],
    )
    .unwrap();
    id
}

pub(crate) fn add_chat(
    paths: &AppPaths,
    video_id: i64,
    title: &str,
    created_at: &str,
    messages: Vec<NewChatMessage>,
) -> i64 {
    let (chat, _) =
        crate::storage::append_chat_turn(paths, video_id, None, title, messages, None).unwrap();
    let conn = rusqlite::Connection::open(&paths.db_path).unwrap();
    conn.execute(
        "UPDATE chats SET created_at = ?1 WHERE id = ?2",
        rusqlite::params![created_at, chat.id],
    )
    .unwrap();
    chat.id
}
