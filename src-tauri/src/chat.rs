//! Chat ueber ein Video: Prompt-Aufbau, Domainenfunktion, Laufregister und die
//! zugehoerigen Tauri-Commands. Etappe 1 des Video-Chats (ohne Tool-Calling).

// Bausteine der Schleife (Live-Ereignisse, Werkzeug-Ausfuehrung) liegen im
// eigenen Modul; der Pfad haelt die Datei bei den uebrigen `chat*.rs`.
#[path = "chat_loop.rs"]
mod chat_loop;

use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, State};

use crate::ai::auth::AuthStore;
pub use crate::chat_runs::ChatRuns;

pub(crate) use self::chat_loop::tool_error_text;
#[cfg(test)]
pub(crate) use self::chat_loop::MAX_TOOL_CALLS_PER_ROUND;
use self::chat_loop::{
    discard_round, execute_with_cancel, finish_round, precheck_tool_call, tool_event, ChatDelta,
    ChatToolEvent, CANCELLED_MESSAGE,
};
use crate::ai::catalog as ai_catalog;
use crate::ai::client as ai_client;
use crate::ai::client::ChatMessage;
use crate::ai::config::AiConfigService;
use crate::ai::tool_stream;
pub use crate::chat_final::looks_like_tool_markup;
use crate::chat_final::{final_round_request, RequestBudget, MAX_PROVIDER_REQUESTS};
use crate::chat_prompt::{
    build_messages_from_context, chat_title, ChatContext, ExtraParts, FINAL_ROUND_REQUEST,
    LAST_ROUND_NOTE,
};
use crate::models::{Chat, ChatContextOptions, ChatMessageRecord, ChatTurnResult, NewChatMessage};
use crate::storage::{self, AppPaths, AppResult};
use crate::summarize::{self, SummaryTarget};
use crate::websearch;

/// Hoechstens ein `ai:chat_stream`-Event in diesem Abstand.
const STREAM_EMIT_INTERVAL: Duration = Duration::from_millis(150);
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

const MAX_TOOL_ROUNDS: usize = 5;
/// Meldung, wenn die Schlussanfrage ohne Werkzeuge keinen Text liefert.
const EMPTY_FINAL_ANSWER_MESSAGE: &str =
    "Das Modell hat nach der Recherche keine Antwort geliefert – bitte erneut versuchen";

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
    context_options: Option<ChatContextOptions>,
    mut is_cancelled: impl FnMut() -> bool,
    mut on_delta: impl FnMut(ChatDelta<'_>),
    mut on_tool: impl FnMut(ChatToolEvent),
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

    // Auswahl: mitgegebene Optionen gewinnen, sonst die des Chats, sonst
    // Standard. Gespeichert wird erst mit der Runde (Commit nach Erfolg).
    let stored_options = match chat_id {
        Some(id) => storage::get_chat(paths, id)?
            .map(|chat| chat.context_options)
            .unwrap_or_default(),
        None => ChatContextOptions::default(),
    };
    let effective_options = context_options.clone().unwrap_or(stored_options);
    let summaries = storage::get_summaries(paths, video_id)?;
    let context = ChatContext::resolve(&video, &summaries, &effective_options)?;
    // Rohteile einmal pro Aufruf: in der Schleife wird nur noch entliehen.
    let raw_parts = context.raw_parts();
    let mut round_messages: Vec<NewChatMessage> = Vec::new();
    // Runde der Tool-Schleife (zaehlt nur Anfragen mit Werkzeugen).
    let mut round = 0usize;
    // Live-Runde: zaehlt jede Provider-Anfrage dieser Frage, auch die
    // Schlussanfrage und jede Wiederholung. Die Live-Anzeige ordnet Text und
    // Werkzeug-Schritte darueber einander zu.
    let mut live_round = 0usize;
    // Hartes Budget ueber alle Provider-Aufrufe dieser Frage.
    let mut budget = RequestBudget::new(MAX_PROVIDER_REQUESTS);
    // Wird nach dem ersten 400/422 gemerkt: danach direkt die Rueckfallform.
    let mut tool_choice_supported = true;

    let answer = loop {
        if is_cancelled() {
            return Err(CANCELLED_MESSAGE.to_string());
        }
        // Vor jeder Anfrage neu bauen: Tool-Ergebnisse der Runde koennen
        // Delimiter enthalten und muessen die Kontextbloecke beeinflussen.
        let messages = build_messages_from_context(
            &context,
            &raw_parts,
            &history,
            &round_messages,
            tools.is_some(),
        );
        // Ab der sechsten Anfrage ist es die Schlussanfrage: sie sendet weiter
        // `tools`, aber `tool_choice: "none"` und die nicht gespeicherte
        // Abschluss-Nachricht; Tool-Aufrufe werden dort nicht ausgewertet.
        let with_tools = tools.is_some() && round < MAX_TOOL_ROUNDS;
        // Jede Provider-Anfrage bekommt eine eigene Live-Runde.
        let round_index = live_round;
        live_round += 1;
        let turn = if with_tools {
            budget.take()?;
            let definitions = websearch::tool_definitions();
            let turn = tool_stream::chat_stream_with_tools_cancellable(
                http,
                &target.base_url,
                target.api_key.as_deref(),
                &target.model,
                &messages,
                &definitions,
                &mut |text: &str| {
                    on_delta(ChatDelta {
                        round: round_index,
                        text,
                        final_text: false,
                        discarded: false,
                    })
                },
                &mut is_cancelled,
            )
            .await
            .map_err(|error| error.to_string())?;
            finish_round(&mut on_delta, round_index, &turn.content);
            turn
        } else if tools.is_some() {
            // Schlussanfrage der Websuche.
            let mut final_messages = messages.clone();
            final_messages.push(ChatMessage::user(FINAL_ROUND_REQUEST));
            let definitions = websearch::tool_definitions();
            let turn = final_round_request(
                http,
                &target,
                &final_messages,
                &definitions,
                &mut tool_choice_supported,
                &mut budget,
                &mut |text: &str| {
                    on_delta(ChatDelta {
                        round: round_index,
                        text,
                        final_text: false,
                        discarded: false,
                    })
                },
                &mut is_cancelled,
            )
            .await?;
            finish_round(&mut on_delta, round_index, &turn.content);

            // Tool-Aufrufe der Schlussanfrage werden ignoriert; es zaehlt der
            // Text. Sicherheitsnetz: leerer Text oder Tool-Markup ist keine
            // Antwort - genau ein Wiederholungsversuch, danach der Fehlertext.
            let answer_text = turn.content.clone();
            let usable = |text: &str| !text.trim().is_empty() && !looks_like_tool_markup(text);
            let text = if usable(&answer_text) {
                answer_text
            } else {
                if !answer_text.trim().is_empty() {
                    // Verworfene Antwort: die Live-Runde verschwindet wieder.
                    discard_round(&mut on_delta, round_index);
                }
                let retry_round = live_round;
                live_round += 1;
                let retry = final_round_request(
                    http,
                    &target,
                    &final_messages,
                    &definitions,
                    &mut tool_choice_supported,
                    &mut budget,
                    &mut |text: &str| {
                        on_delta(ChatDelta {
                            round: retry_round,
                            text,
                            final_text: false,
                            discarded: false,
                        })
                    },
                    &mut is_cancelled,
                )
                .await?;
                finish_round(&mut on_delta, retry_round, &retry.content);
                if usable(&retry.content) {
                    retry.content
                } else {
                    return Err(EMPTY_FINAL_ANSWER_MESSAGE.to_string());
                }
            };
            tool_stream::ChatTurn {
                content: text,
                tool_calls: Vec::new(),
            }
        } else {
            // Ohne Websuche bleibt der strenge Pfad (ein Request).
            budget.take()?;
            let text = ai_client::chat_stream_cancellable(
                http,
                &target.base_url,
                target.api_key.as_deref(),
                &target.model,
                &messages,
                &mut |text: &str| {
                    on_delta(ChatDelta {
                        round: round_index,
                        text,
                        final_text: false,
                        discarded: false,
                    })
                },
                &mut is_cancelled,
            )
            .await
            .map_err(|error| error.to_string())?;
            finish_round(&mut on_delta, round_index, &text);
            tool_stream::ChatTurn {
                content: text,
                tool_calls: Vec::new(),
            }
        };

        // Die Schlussanfrage (Websuche ab Runde 5) ist die Antwort.
        if tools.is_some() && round >= MAX_TOOL_ROUNDS {
            break turn.content;
        }

        if turn.tool_calls.is_empty() {
            let content = turn.content;
            // Markup in einer regulaeren Runde (ohne tool_calls): nicht speichern,
            // sondern einmalig die Schlussanfrage stellen.
            if tools.is_some() && looks_like_tool_markup(&content) {
                // Die gestreamte Markup-Antwort wird verworfen: erst danach
                // startet die Wiederholung als neue Live-Runde.
                discard_round(&mut on_delta, round_index);
                let mut final_messages = messages.clone();
                final_messages.push(ChatMessage::user(FINAL_ROUND_REQUEST));
                let definitions = websearch::tool_definitions();
                // Letzte Anfrage dieser Frage: die Wiederholung beendet die
                // Schleife in jedem Fall, deshalb ohne weiteres Hochzaehlen.
                let retry_round = live_round;
                let retry = final_round_request(
                    http,
                    &target,
                    &final_messages,
                    &definitions,
                    &mut tool_choice_supported,
                    &mut budget,
                    &mut |text: &str| {
                        on_delta(ChatDelta {
                            round: retry_round,
                            text,
                            final_text: false,
                            discarded: false,
                        })
                    },
                    &mut is_cancelled,
                )
                .await?;
                finish_round(&mut on_delta, retry_round, &retry.content);
                if retry.tool_calls.is_empty()
                    && !looks_like_tool_markup(&retry.content)
                    && !retry.content.trim().is_empty()
                {
                    break retry.content;
                }
                return Err(EMPTY_FINAL_ANSWER_MESSAGE.to_string());
            }
            break content;
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
        round_messages.push(assistant);

        let runtime = tools.as_ref().expect("tools vorhanden");
        for (index, call) in turn.tool_calls.iter().enumerate() {
            if is_cancelled() {
                return Err(CANCELLED_MESSAGE.to_string());
            }
            // Nicht ausgefuehrte Aufrufe (Limit, unbekanntes Tool, unbrauchbare
            // Argumente): genau ein error-Event mit festem Label und ohne
            // modellgesteuerten Namen.
            let content = match precheck_tool_call(index, &call.name, &call.arguments) {
                Err((reason, label)) => {
                    on_tool(tool_event(round_index, "other", label.to_string(), "error"));
                    tool_error_text(reason)
                }
                Ok(()) => {
                    let label = websearch::tool_label(&call.name, &call.arguments);
                    let kind = websearch::tool_kind(&call.name);
                    on_tool(tool_event(round_index, kind, label.clone(), "start"));
                    let result = execute_with_cancel(
                        runtime,
                        &call.name,
                        &call.arguments,
                        &mut is_cancelled,
                    )
                    .await?;
                    match result {
                        Ok(text) => {
                            let parts = ExtraParts::new(
                                &raw_parts,
                                history.iter().chain(round_messages.iter()),
                            );
                            let refs = parts.refs();
                            on_tool(tool_event(round_index, kind, label, "ok"));
                            summarize::wrap_untrusted("WEB RESULT", &text, &refs)
                        }
                        Err(reason) => {
                            on_tool(tool_event(round_index, kind, label, "error"));
                            tool_error_text(&reason)
                        }
                    }
                }
            };
            // In der letzten erlaubten Runde bekommt die letzte Tool-Nachricht
            // die feste Schlusszeile (ausserhalb des WEB-RESULT-Blocks).
            let is_last_round = round >= MAX_TOOL_ROUNDS;
            let is_last_call = index + 1 == turn.tool_calls.len();
            let content = if is_last_round && is_last_call {
                format!("{content}\n\n{LAST_ROUND_NOTE}")
            } else {
                content
            };
            let tool_message = NewChatMessage {
                role: "tool".to_string(),
                content,
                tool_calls: None,
                tool_call_id: Some(call.id.clone()),
                provider: None,
                model: None,
            };
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

    let (chat, _) = storage::append_chat_turn(
        paths,
        video_id,
        chat_id,
        &chat_title(question),
        turn,
        context_options.as_ref(),
    )?;
    // Die Runde ist gespeichert; der Aufrufer bekommt den vollstaendigen
    // Verlauf nach der Runde (nicht nur die neuen Nachrichten).
    let messages = storage::get_chat_messages(paths, chat.id)?;
    Ok(ChatTurnResult { chat, messages })
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
    context_options: Option<ChatContextOptions>,
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
        context_options,
        || guard.is_cancelled(),
        move |delta: ChatDelta<'_>| {
            // Zwischenstaende hoechstens alle 150 ms; das Rundenende immer.
            if !delta.final_text {
                let now = Instant::now();
                if last_emit.is_some_and(|last| now.duration_since(last) < STREAM_EMIT_INTERVAL) {
                    return;
                }
                last_emit = Some(now);
            }
            if let Err(error) = app.emit(
                "ai:chat_stream",
                serde_json::json!({
                    "requestId": event_request_id,
                    "videoId": video_id,
                    "round": delta.round,
                    "text": delta.text,
                    "final": delta.final_text,
                    "discarded": delta.discarded,
                }),
            ) {
                if !emit_error_logged {
                    eprintln!("ai:chat_stream emit failed: {error}");
                    emit_error_logged = true;
                }
            }
        },
        move |tool: ChatToolEvent| {
            if let Err(error) = tool_app.emit(
                "ai:chat_tool",
                serde_json::json!({
                    "requestId": tool_request_id,
                    "videoId": video_id,
                    "round": tool.round,
                    "kind": tool.event.kind,
                    "label": tool.event.label,
                    "status": tool.event.status,
                }),
            ) {
                eprintln!("ai:chat_tool emit failed: {error}");
            }
        },
    )
    .await
}

/// Aendert die Kontext-Auswahl eines bestehenden Chats ohne zu senden.
#[tauri::command]
pub fn chat_context_set(
    paths: State<'_, AppPaths>,
    chat_id: i64,
    options: ChatContextOptions,
) -> AppResult<Chat> {
    storage::set_chat_context(&paths, chat_id, &options)
}

#[tauri::command]
pub fn chat_cancel(runs: State<'_, ChatRuns>, request_id: String) -> AppResult<()> {
    runs.cancel(&request_id);
    Ok(())
}

#[cfg(test)]
mod tests;
