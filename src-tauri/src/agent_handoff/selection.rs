//! Kontextauswahl einer Uebergabe (Revision 3): das Auswahlmodell, die
//! Vorbelegung aus der Konfiguration, die Aufloesung auf die Daten genau eines
//! Videos und der Katalog der waehlbaren Quellen fuer den Dialog.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::models::{Chat, ChatMessageRecord, Summary, Video};

use super::config::AgentConfig;
use super::context;

/// Auswahl des Kontexts fuer eine einzelne Uebergabe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffSelection {
    #[serde(default = "default_true")]
    pub transcript: bool,
    /// `null` = neueste Fassung (`videos.summary`), `[]` = keine.
    #[serde(default)]
    pub summary_ids: Option<Vec<i64>>,
    #[serde(default)]
    pub chat_ids: Vec<i64>,
}

fn default_true() -> bool {
    true
}

impl Default for HandoffSelection {
    fn default() -> Self {
        Self {
            transcript: true,
            summary_ids: None,
            chat_ids: Vec::new(),
        }
    }
}

/// Wirksame Auswahl plus die zu exportierenden Zeilen in Dateireihenfolge
/// (aelteste zuerst).
pub struct Resolved<'a> {
    pub selection: HandoffSelection,
    pub summaries: Vec<&'a Summary>,
    pub chats: Vec<&'a Chat>,
}

/// Waehlbare und exportierbare Versionen: leere Texte (nach `trim`) sind kein
/// Inhalt und erscheinen deshalb weder im Dialog noch in der Datei.
pub fn exportable_summaries(rows: Vec<Summary>) -> Vec<Summary> {
    rows.into_iter()
        .filter(|row| !row.summary.trim().is_empty())
        .collect()
}

/// Waehlbare und exportierbare Chats: ohne exportierbare Nachricht bliebe nur
/// der Titel uebrig — solche Chats fallen weg.
pub fn exportable_chats(
    rows: Vec<Chat>,
    messages: &BTreeMap<i64, Vec<ChatMessageRecord>>,
) -> Vec<Chat> {
    rows.into_iter()
        .filter(|chat| {
            exported_message_count(messages.get(&chat.id).map(Vec::as_slice).unwrap_or(&[])) > 0
        })
        .collect()
}

/// Vorbelegung aus dem Einstellungs-Tab, wenn der Aufrufer keine Auswahl
/// mitschickt.
pub fn preset(config: &AgentConfig, summaries: &[Summary], chats: &[Chat]) -> HandoffSelection {
    HandoffSelection {
        transcript: config.include_transcript,
        summary_ids: match config.summaries.as_str() {
            "none" => Some(Vec::new()),
            "all" => Some(summaries.iter().map(|row| row.id).collect()),
            _ => None,
        },
        chat_ids: if config.include_chats {
            chats.iter().map(|chat| chat.id).collect()
        } else {
            Vec::new()
        },
    }
}

/// Schraenkt eine Auswahl auf die Daten dieses Videos ein: unbekannte oder
/// fremde IDs und Duplikate entfallen stillschweigend; die Reihenfolge der
/// Ausgabe ist unabhaengig von der Eingabe `created_at` aufsteigend (dann
/// `id`). Eine leere Auswahl ist erlaubt.
pub fn resolve<'a>(
    requested: &HandoffSelection,
    summaries: &'a [Summary],
    chats: &'a [Chat],
) -> Resolved<'a> {
    let summary_rows = match &requested.summary_ids {
        Some(ids) => select_summaries(summaries, ids),
        // `null` bleibt „neueste Fassung“ (aus `videos.summary`).
        None => Vec::new(),
    };
    let chat_rows = select_chats(chats, &requested.chat_ids);
    Resolved {
        selection: HandoffSelection {
            transcript: requested.transcript,
            summary_ids: requested
                .summary_ids
                .as_ref()
                .map(|_| summary_rows.iter().map(|row| row.id).collect()),
            chat_ids: chat_rows.iter().map(|chat| chat.id).collect(),
        },
        summaries: summary_rows,
        chats: chat_rows,
    }
}

fn select_summaries<'a>(all: &'a [Summary], ids: &[i64]) -> Vec<&'a Summary> {
    let wanted: BTreeSet<i64> = ids.iter().copied().collect();
    let mut rows: Vec<&Summary> = all.iter().filter(|row| wanted.contains(&row.id)).collect();
    rows.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then(left.id.cmp(&right.id))
    });
    rows
}

fn select_chats<'a>(all: &'a [Chat], ids: &[i64]) -> Vec<&'a Chat> {
    let wanted: BTreeSet<i64> = ids.iter().copied().collect();
    let mut rows: Vec<&Chat> = all.iter().filter(|row| wanted.contains(&row.id)).collect();
    rows.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then(left.id.cmp(&right.id))
    });
    rows
}

/// Eine waehlbare Zusammenfassungs-Version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryOption {
    pub id: i64,
    pub created_at: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub options: Option<String>,
}

/// Ein waehlbarer Chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatOption {
    pub id: i64,
    pub title: String,
    pub created_at: String,
    /// Genau die Nachrichten, die exportiert wuerden.
    pub message_count: usize,
    /// Erste Frage des Chats (der Titel ist deren gekuerzte Form), fuer den
    /// Tooltip im Dialog; auf `FIRST_QUESTION_MAX_CHARS` begrenzt.
    pub first_question: Option<String>,
}

const FIRST_QUESTION_MAX_CHARS: usize = 1000;

fn first_question(messages: &[ChatMessageRecord]) -> Option<String> {
    let text = messages
        .iter()
        .find(|message| message.role == "user" && !message.content.trim().is_empty())?
        .content
        .trim();
    if text.chars().count() <= FIRST_QUESTION_MAX_CHARS {
        return Some(text.to_string());
    }
    let mut cut: String = text.chars().take(FIRST_QUESTION_MAX_CHARS).collect();
    cut.push('…');
    Some(cut)
}

/// Was der Dialog auswaehlen kann.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    pub has_transcript: bool,
    pub has_latest_summary: bool,
    /// Neueste zuerst.
    pub summaries: Vec<SummaryOption>,
    /// Neueste zuerst (`created_at`).
    pub chats: Vec<ChatOption>,
}

pub fn available(
    video: &Video,
    summaries: &[Summary],
    chats: &[Chat],
    messages: &BTreeMap<i64, Vec<ChatMessageRecord>>,
) -> Available {
    let mut summary_options: Vec<SummaryOption> = summaries
        .iter()
        .map(|row| SummaryOption {
            id: row.id,
            created_at: row.created_at.clone(),
            provider: row.provider.clone(),
            model: row.model.clone(),
            options: row.options.clone(),
        })
        .collect();
    summary_options.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then(right.id.cmp(&left.id))
    });

    let mut chat_options: Vec<ChatOption> = chats
        .iter()
        .map(|chat| {
            let rows = messages.get(&chat.id).map(Vec::as_slice).unwrap_or(&[]);
            ChatOption {
                id: chat.id,
                title: chat.title.clone(),
                created_at: chat.created_at.clone(),
                message_count: exported_message_count(rows),
                first_question: first_question(rows),
            }
        })
        .collect();
    chat_options.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then(right.id.cmp(&left.id))
    });

    Available {
        has_transcript: context::transcript_text(video).is_some(),
        has_latest_summary: has_text(video.summary.as_deref()),
        summaries: summary_options,
        chats: chat_options,
    }
}

/// Zaehlt genau die Nachrichten, die im CHAT-Block landen: nur die Rollen
/// `user`/`assistant` mit nach `trim` nichtleerem Text.
pub fn exported_message_count(messages: &[ChatMessageRecord]) -> usize {
    messages
        .iter()
        .filter(|message| {
            matches!(message.role.as_str(), "user" | "assistant")
                && !message.content.trim().is_empty()
        })
        .count()
}

fn has_text(value: Option<&str>) -> bool {
    value.map(str::trim).is_some_and(|text| !text.is_empty())
}
