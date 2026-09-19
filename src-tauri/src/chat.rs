//! Chat ueber ein Video: Prompt-Aufbau, Domainenfunktion, Laufregister und die
//! zugehoerigen Tauri-Commands. Etappe 1 des Video-Chats (ohne Tool-Calling).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, State};

use crate::ai::auth::AuthStore;
use crate::ai::catalog as ai_catalog;
use crate::ai::client as ai_client;
use crate::ai::config::AiConfigService;
use crate::ai::tool_stream;
use crate::chat_prompt::{
    build_chat_messages_with, chat_title, extra_parts, to_client_message, ChatContext,
};
use crate::models::{Chat, ChatMessageRecord, ChatTurnResult, NewChatMessage};
use crate::storage::{self, AppPaths, AppResult};
use crate::summarize::{self, SummaryTarget};
use crate::websearch;

/// Hoechstens ein `ai:chat_stream`-Event in diesem Abstand.
const STREAM_EMIT_INTERVAL: Duration = Duration::from_millis(150);
/// Vorab abgebrochene Anfragen ohne Lauf verfallen nach dieser Zeit.
const PENDING_RUN_MAX_AGE: Duration = Duration::from_secs(60);
/// Laenge des aus der Frage gebildeten Chat-Titels in Unicode-Skalarwerten.

/// Testfassung ohne Websuche-Zusatz (P-Faelle).
#[cfg(test)]
pub use crate::chat_prompt::{build_chat_messages, chat_system_prompt};
#[cfg(test)]
pub use crate::chat_prompt::{CHAT_SYSTEM_PROMPT, WEB_SEARCH_PROMPT_ADDENDUM};

/// Websuche ist nur aktiv, wenn sie gewuenscht ist, die Konfiguration passt
/// (aktiv + URL) und das gewaehlte Modell Tool-Calling unterstuetzt. Sonst
/// laeuft die Runde stillschweigend ohne `tools`.
pub fn web_search_runtime(
    paths: &AppPaths,
    catalog: &crate::ai::types::Catalog,
    provider_id: &str,
    model_id: &str,
    requested: Option<bool>,
) -> Option<websearch::ToolRuntime> {
    if requested != Some(true) {
        return None;
    }
    let config = websearch::config::load(paths);
    if !config.is_active() {
        return None;
    }
    let supports_tools = catalog
        .get(provider_id)
        .and_then(|provider| provider.models.get(model_id))
        .and_then(|model| model.tool_call)
        == Some(true);
    if !supports_tools {
        return None;
    }
    Some(websearch::production_runtime(config.searxng_url))
}

const CANCELLED_MESSAGE: &str = "KI-Antwort abgebrochen";
const MAX_TOOL_ROUNDS: usize = 5;
const MAX_TOOL_CALLS_PER_ROUND: usize = 4;

/// Prueft Tool-Name und Argumente, bevor der Executor laeuft.
fn validate_tool_call(name: &str, arguments: &str) -> Result<(), String> {
    match name {
        websearch::WEB_SEARCH_TOOL => websearch::string_argument(arguments, "query").map(|_| ()),
        websearch::FETCH_PAGE_TOOL => websearch::string_argument(arguments, "url").map(|_| ()),
        _ => Err(websearch::UNKNOWN_TOOL_MESSAGE.to_string()),
    }
}

fn tool_error_text(reason: &str) -> String {
    if reason.starts_with("Fehler:") {
        reason.to_string()
    } else {
        format!("Fehler: {reason}")
    }
}

/// Eine Chat-Runde: Verlauf + neue Frage als Prompt senden und erst nach
/// erfolgreichem Abschluss speichern. Der Fehler des Clients wird unveraendert
/// durchgereicht. Mit `tools` laeuft die Tool-Schleife der Etappe 2.
#[allow(clippy::too_many_arguments)]
pub async fn chat_send_impl(
    paths: &AppPaths,
    http: &reqwest::Client,
    video_id: i64,
    chat_id: Option<i64>,
    text: String,
    target: SummaryTarget,
    tools: Option<websearch::ToolRuntime>,
    mut is_cancelled: impl FnMut() -> bool,
    mut on_delta: impl FnMut(&str),
    mut on_tool: impl FnMut(websearch::ToolEvent),
) -> AppResult<ChatTurnResult> {
    let question = text.trim();
    if question.is_empty() {
        return Err("Bitte eine Frage eingeben".to_string());
    }

    let video =
        storage::get_video(paths, video_id)?.ok_or_else(|| "Video nicht gefunden".to_string())?;

    if let Some(id) = chat_id {
        match storage::get_chat(paths, id)? {
            None => return Err("Chat wurde gelöscht".to_string()),
            Some(chat) if chat.video_id != video_id => {
                return Err("Chat gehört nicht zu diesem Video".to_string())
            }
            Some(_) => {}
        }
    }

    let mut history: Vec<NewChatMessage> = match chat_id {
        Some(id) => storage::get_chat_messages(paths, id)?
            .into_iter()
            .map(NewChatMessage::from_record)
            .collect(),
        None => Vec::new(),
    };
    history.push(NewChatMessage::user(question));

    let context = ChatContext::new(&video)?;
    let mut messages = build_chat_messages_with(&video, &history, tools.is_some())?;
    let mut round_messages: Vec<NewChatMessage> = Vec::new();
    let mut round = 0usize;

    let answer = loop {
        if is_cancelled() {
            return Err(CANCELLED_MESSAGE.to_string());
        }
        // In der Schlussrunde (und ohne Websuche) bleibt der strenge Pfad:
        // Tool-Aufrufe werden dort nicht ausgewertet.
        let with_tools = tools.is_some() && round < MAX_TOOL_ROUNDS;
        let turn = if with_tools {
            let definitions = websearch::tool_definitions();
            tool_stream::chat_stream_with_tools_cancellable(
                http,
                &target.base_url,
                target.api_key.as_deref(),
                &target.model,
                &messages,
                &definitions,
                &mut on_delta,
                &mut is_cancelled,
            )
            .await
            .map_err(|error| error.to_string())?
        } else {
            let text = ai_client::chat_stream_cancellable(
                http,
                &target.base_url,
                target.api_key.as_deref(),
                &target.model,
                &messages,
                &mut on_delta,
                &mut is_cancelled,
            )
            .await
            .map_err(|error| error.to_string())?;
            tool_stream::ChatTurn {
                content: text,
                tool_calls: Vec::new(),
            }
        };

        if turn.tool_calls.is_empty() {
            break turn.content;
        }
        round += 1;

        let assistant = NewChatMessage {
            role: "assistant".to_string(),
            content: turn.content.clone(),
            tool_calls: Some(tool_stream::ToolCall::list_to_openai_json(&turn.tool_calls)),
            tool_call_id: None,
            provider: None,
            model: None,
        };
        messages.push(to_client_message(&assistant));
        round_messages.push(assistant);

        let runtime = tools.as_ref().expect("tools vorhanden");
        for (index, call) in turn.tool_calls.iter().enumerate() {
            if is_cancelled() {
                return Err(CANCELLED_MESSAGE.to_string());
            }
            let label = websearch::tool_label(&call.name, &call.arguments);
            let kind = websearch::tool_kind(&call.name);
            on_tool(websearch::ToolEvent {
                kind,
                label: label.clone(),
                status: "start",
            });
            let result = if index >= MAX_TOOL_CALLS_PER_ROUND {
                Err(websearch::TOOL_LIMIT_MESSAGE.to_string())
            } else {
                // Unbekannte Tools und unbrauchbare Argumente werden nicht
                // ausgefuehrt, sondern als Fehler an das Modell zurueckgegeben.
                match validate_tool_call(&call.name, &call.arguments) {
                    Err(reason) => Err(reason),
                    Ok(()) => (runtime.execute)(call.name.clone(), call.arguments.clone()).await,
                }
            };
            let (status, content) = match result {
                Ok(text) => {
                    let parts = extra_parts(&context, history.iter().chain(round_messages.iter()));
                    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
                    ("ok", summarize::wrap_untrusted("WEB RESULT", &text, &refs))
                }
                Err(reason) => ("error", tool_error_text(&reason)),
            };
            on_tool(websearch::ToolEvent {
                kind,
                label,
                status,
            });
            let tool_message = NewChatMessage {
                role: "tool".to_string(),
                content,
                tool_calls: None,
                tool_call_id: Some(call.id.clone()),
                provider: None,
                model: None,
            };
            messages.push(to_client_message(&tool_message));
            round_messages.push(tool_message);
        }
    };

    // Ein Abbruch unmittelbar vor dem Speichern verwirft die ganze Runde.
    if is_cancelled() {
        return Err(CANCELLED_MESSAGE.to_string());
    }

    let mut assistant = NewChatMessage::assistant(answer);
    assistant.provider = Some(target.provider_label.clone());
    assistant.model = Some(target.model.clone());
    let mut turn = vec![NewChatMessage::user(question)];
    turn.extend(round_messages);
    turn.push(assistant);

    let (chat, _) =
        storage::append_chat_turn(paths, video_id, chat_id, &chat_title(question), turn)?;
    // Die Runde ist gespeichert; der Aufrufer bekommt den vollstaendigen
    // Verlauf nach der Runde (nicht nur die neuen Nachrichten).
    let messages = storage::get_chat_messages(paths, chat.id)?;
    Ok(ChatTurnResult { chat, messages })
}

#[derive(Debug)]
struct RunEntry {
    video_id: Option<i64>,
    flag: Arc<AtomicBool>,
    created: Instant,
}

/// Laufregister des Video-Chats: hoechstens eine Anfrage pro Video, jede
/// Anfrage ueber ihre `request_id` abbrechbar.
#[derive(Debug, Clone, Default)]
pub struct ChatRuns {
    entries: Arc<Mutex<HashMap<String, RunEntry>>>,
}

impl ChatRuns {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, RunEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Erste Aktion von `chat_send`: prueft und reserviert den Lauf.
    pub fn begin(&self, request_id: &str, video_id: i64) -> AppResult<ChatRunGuard> {
        if request_id.trim().is_empty() {
            return Err("Ungültige Anfrage-ID".to_string());
        }
        let mut entries = self.lock();
        entries.retain(|_, entry| {
            entry.video_id.is_some() || entry.created.elapsed() < PENDING_RUN_MAX_AGE
        });
        if let Some(entry) = entries.get(request_id) {
            if entry.video_id.is_none() {
                entries.remove(request_id);
                return Err("KI-Antwort abgebrochen".to_string());
            }
            return Err("Anfrage-ID bereits in Verwendung".to_string());
        }
        if entries
            .values()
            .any(|entry| entry.video_id == Some(video_id))
        {
            return Err("Es läuft bereits eine Chat-Anfrage für dieses Video".to_string());
        }
        let flag = Arc::new(AtomicBool::new(false));
        entries.insert(
            request_id.to_string(),
            RunEntry {
                video_id: Some(video_id),
                flag: flag.clone(),
                created: Instant::now(),
            },
        );
        Ok(ChatRunGuard {
            runs: self.clone(),
            request_id: request_id.to_string(),
            flag,
        })
    }

    /// Bricht eine bekannte Anfrage ab. Unbekannte IDs werden als vorab
    /// abgebrochen vermerkt, damit ein spaeterer `chat_send` sofort stoppt.
    pub fn cancel(&self, request_id: &str) {
        let mut entries = self.lock();
        match entries.get_mut(request_id) {
            Some(entry) => entry.flag.store(true, Ordering::SeqCst),
            None => {
                entries.insert(
                    request_id.to_string(),
                    RunEntry {
                        video_id: None,
                        flag: Arc::new(AtomicBool::new(true)),
                        created: Instant::now(),
                    },
                );
            }
        }
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }
}

/// Entfernt beim Verlassen den eigenen Registereintrag, aber nur wenn dort
/// noch das eigene Flag steht (kein Entfernen fremder Laeufe).
#[derive(Debug)]
pub struct ChatRunGuard {
    runs: ChatRuns,
    request_id: String,
    flag: Arc<AtomicBool>,
}

impl ChatRunGuard {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

impl Drop for ChatRunGuard {
    fn drop(&mut self) {
        let mut entries = self.runs.lock();
        let is_own = entries
            .get(&self.request_id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.flag, &self.flag));
        if is_own {
            entries.remove(&self.request_id);
        }
    }
}

#[tauri::command]
pub fn chat_list(paths: State<'_, AppPaths>, video_id: i64) -> AppResult<Vec<Chat>> {
    storage::list_chats(&paths, video_id)
}

#[tauri::command]
pub fn chat_messages(
    paths: State<'_, AppPaths>,
    chat_id: i64,
) -> AppResult<Vec<ChatMessageRecord>> {
    storage::get_chat_messages(&paths, chat_id)
}

#[tauri::command]
pub fn chat_delete(paths: State<'_, AppPaths>, chat_id: i64) -> AppResult<()> {
    storage::delete_chat(&paths, chat_id)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn chat_send(
    app: AppHandle,
    paths: State<'_, AppPaths>,
    cfg: State<'_, std::sync::Mutex<AiConfigService>>,
    auth: State<'_, std::sync::Mutex<AuthStore>>,
    http: State<'_, reqwest::Client>,
    runs: State<'_, ChatRuns>,
    video_id: i64,
    chat_id: Option<i64>,
    text: String,
    provider_id: Option<String>,
    model_id: Option<String>,
    request_id: String,
    web_search: Option<bool>,
) -> AppResult<ChatTurnResult> {
    // Registrierung vor Zielaufloesung und jedem await.
    let guard = runs.begin(&request_id, video_id)?;

    let (target, tools) = {
        let ai = crate::commands::ai_config_data_from_state(&cfg)?;
        let catalog = ai_catalog::load(paths.inner()).catalog;
        let (selected, base_url) =
            summarize::resolve_summary_target(&ai, &catalog, provider_id, model_id)?;
        let key = crate::commands::lock_ai_auth_from_state(&auth)?.get_key(&selected.provider);
        let provider_label = summarize::provider_label(&ai, &catalog, &selected.provider);
        let tools = web_search_runtime(
            &paths,
            &catalog,
            &selected.provider,
            &selected.model,
            web_search,
        );
        (
            SummaryTarget {
                provider_label,
                model: selected.model,
                base_url,
                api_key: key,
            },
            tools,
        )
    };

    let event_request_id = request_id.clone();
    let tool_request_id = request_id.clone();
    let tool_app = app.clone();
    let mut last_emit: Option<Instant> = None;
    let mut emit_error_logged = false;
    chat_send_impl(
        &paths,
        &http,
        video_id,
        chat_id,
        text,
        target,
        tools,
        || guard.is_cancelled(),
        move |accumulated| {
            let now = Instant::now();
            if last_emit.is_some_and(|last| now.duration_since(last) < STREAM_EMIT_INTERVAL) {
                return;
            }
            last_emit = Some(now);
            if let Err(error) = app.emit(
                "ai:chat_stream",
                serde_json::json!({
                    "requestId": event_request_id,
                    "videoId": video_id,
                    "text": accumulated,
                }),
            ) {
                if !emit_error_logged {
                    eprintln!("ai:chat_stream emit failed: {error}");
                    emit_error_logged = true;
                }
            }
        },
        move |tool: websearch::ToolEvent| {
            if let Err(error) = tool_app.emit(
                "ai:chat_tool",
                serde_json::json!({
                    "requestId": tool_request_id,
                    "videoId": video_id,
                    "kind": tool.kind,
                    "label": tool.label,
                    "status": tool.status,
                }),
            ) {
                eprintln!("ai:chat_tool emit failed: {error}");
            }
        },
    )
    .await
}

#[tauri::command]
pub fn chat_cancel(runs: State<'_, ChatRuns>, request_id: String) -> AppResult<()> {
    runs.cancel(&request_id);
    Ok(())
}

#[cfg(test)]
mod tests;
