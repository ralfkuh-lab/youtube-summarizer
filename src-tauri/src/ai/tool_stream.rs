//! Tool-Calling-Stream: wie `client::chat_stream_cancellable`, sendet zusaetzlich
//! `tools` und baut gestreamte Tool-Aufrufe zu vollstaendigen Aufrufen zusammen.
//! Nutzt die gemeinsamen Bausteine aus `ai/client.rs` (SSE-Decoder, Anfrage,
//! Fehlertexte, Abbruch- und Timeout-Logik).
//!
//! Etappe 2a: der Aufrufer in der Chat-Schleife folgt in Etappe 2b; bis dahin
//! ist das Modul nur ueber die Tests erreichbar.
#![allow(dead_code)]

use std::collections::BTreeMap;

use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};

use super::client::{self, ChatError, ChatMessage, SseStep};

/// Ein vom Modell angeforderter Tool-Aufruf. `type` wird nicht gespeichert:
/// Tools sind immer Funktionen (fehlendes `type` gilt als `"function"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

impl ToolCall {
    /// OpenAI-Form fuer die naechste Anfrage (`type` ist immer `function`).
    pub fn to_openai_json(&self) -> Value {
        json!({
            "id": self.id,
            "type": "function",
            "function": {"name": self.name, "arguments": self.arguments}
        })
    }

    pub fn list_to_openai_json(calls: &[ToolCall]) -> Value {
        Value::Array(calls.iter().map(ToolCall::to_openai_json).collect())
    }
}

/// Ergebnis einer Runde: Text und/oder Tool-Aufrufe.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChatTurn {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}

pub async fn chat_stream_with_tools_cancellable(
    http: &Client,
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
    messages: &[ChatMessage],
    tools: &[Value],
    on_delta: impl FnMut(&str),
    is_cancelled: impl FnMut() -> bool,
) -> Result<ChatTurn, ChatError> {
    chat_stream_with_tools_and_choice(
        http,
        base_url,
        api_key,
        model,
        messages,
        tools,
        None,
        on_delta,
        is_cancelled,
    )
    .await
}

/// Wie `chat_stream_with_tools_cancellable`, zusaetzlich mit `tool_choice`
/// (`"none"` fuer die Schlussanfrage der Websuche).
#[allow(clippy::too_many_arguments)]
pub async fn chat_stream_with_tools_and_choice(
    http: &Client,
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
    messages: &[ChatMessage],
    tools: &[Value],
    tool_choice: Option<&str>,
    mut on_delta: impl FnMut(&str),
    mut is_cancelled: impl FnMut() -> bool,
) -> Result<ChatTurn, ChatError> {
    let api_key = api_key.map(str::trim).filter(|key| !key.is_empty());
    let response = client::send_chat_request_with_choice(
        http,
        base_url,
        api_key,
        model,
        messages,
        Some(tools),
        tool_choice,
    )
    .await?;

    // JSON-Fallback (kein SSE): auch hier kann das Modell Tool-Aufrufe liefern.
    if !client::response_content_type(&response).starts_with("text/event-stream") {
        let body = response
            .text()
            .await
            .map_err(|error| ChatError::ResponseRead(client::error_chain(&error)))?;
        let turn = parse_tool_chat_response(&body)?;
        if !turn.content.is_empty() {
            on_delta(&turn.content);
        }
        return Ok(turn);
    }

    let mut accumulator = ToolCallAccumulator::default();
    let mut content = String::new();
    let mut completed = false;
    client::read_sse_stream(
        response,
        |event| {
            if event.trim() == "[DONE]" {
                completed = true;
                return Ok(SseStep::Done);
            }
            if let Some(message) = client::stream_error_message(event, api_key) {
                return Err(ChatError::Http {
                    status: StatusCode::BAD_GATEWAY,
                    message,
                });
            }
            let chunk = serde_json::from_str::<ToolStreamResponse>(event)
                .map_err(|error| ChatError::InvalidJson(error.to_string()))?;
            if let Some(choice) = chunk.choices.into_iter().next() {
                if let Some(delta) = choice.delta {
                    for fragment in delta.tool_calls.unwrap_or_default() {
                        accumulator.push(fragment);
                    }
                    if let Some(text) = delta.content {
                        if !text.is_empty() {
                            content.push_str(&text);
                            on_delta(&content);
                        }
                    }
                }
                if let Some(reason) = choice.finish_reason.as_deref() {
                    if reason == "length" {
                        return Err(ChatError::TruncatedOutput);
                    }
                    if !reason.trim().is_empty() {
                        completed = true;
                    }
                }
            }
            Ok(SseStep::Continue)
        },
        &mut is_cancelled,
    )
    .await?;

    // Anders als der strenge Text-Stream ist ein leerer Text zulaessig, solange
    // Tool-Aufrufe vorliegen.
    if !completed {
        return Err(ChatError::IncompleteStream);
    }
    let tool_calls = accumulator.finish();
    if content.trim().is_empty() && tool_calls.is_empty() {
        return Err(ChatError::MissingChoice);
    }
    Ok(ChatTurn {
        content,
        tool_calls,
    })
}

#[derive(Debug, Deserialize)]
struct ToolStreamResponse {
    choices: Vec<ToolStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct ToolStreamChoice {
    delta: Option<ToolStreamDelta>,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ToolStreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCallFragment>>,
}

#[derive(Debug, Deserialize)]
struct ToolCallFragment {
    index: Option<usize>,
    id: Option<String>,
    function: Option<ToolCallFunctionFragment>,
}

#[derive(Debug, Deserialize)]
struct ToolCallFunctionFragment {
    name: Option<String>,
    arguments: Option<String>,
}

/// Baut die ueber viele Deltas verteilten Fragmente zu Tool-Aufrufen zusammen:
/// Gruppierung nach `index` (fehlend -> 0), `id`/`name` erstes nichtleeres
/// Fragment, `arguments` pro Index konkateniert.
#[derive(Debug, Default)]
struct ToolCallAccumulator {
    calls: BTreeMap<usize, ToolCallBuilder>,
}

#[derive(Debug, Default)]
struct ToolCallBuilder {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl ToolCallAccumulator {
    fn push(&mut self, fragment: ToolCallFragment) {
        let index = fragment.index.unwrap_or(0);
        let builder = self.calls.entry(index).or_default();
        if let Some(id) = fragment.id.filter(|id| !id.is_empty()) {
            if builder.id.is_none() {
                builder.id = Some(id);
            }
        }
        if let Some(function) = fragment.function {
            if let Some(name) = function.name.filter(|name| !name.is_empty()) {
                if builder.name.is_none() {
                    builder.name = Some(name);
                }
            }
            if let Some(arguments) = function.arguments {
                builder.arguments.push_str(&arguments);
            }
        }
    }

    fn finish(self) -> Vec<ToolCall> {
        let calls = self
            .calls
            .into_iter()
            .map(|(index, builder)| ToolCall {
                id: builder.id.unwrap_or_else(|| format!("call_{index}")),
                name: builder.name.unwrap_or_default(),
                arguments: builder.arguments,
            })
            .collect();
        dedupe_ids(calls)
    }
}

/// Doppelte IDs innerhalb einer Antwort erhalten `call_{index}` und werden
/// notfalls weiter hochgezaehlt, damit jede Tool-Nachricht eindeutig zuordenbar
/// bleibt.
fn dedupe_ids(calls: Vec<ToolCall>) -> Vec<ToolCall> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut result = Vec::with_capacity(calls.len());
    for (index, mut call) in calls.into_iter().enumerate() {
        if seen.insert(call.id.clone()) {
            result.push(call);
            continue;
        }
        let mut candidate_index = index;
        let unique = loop {
            let candidate = format!("call_{candidate_index}");
            if seen.insert(candidate.clone()) {
                break candidate;
            }
            candidate_index += 1;
        };
        call.id = unique;
        result.push(call);
    }
    result
}

#[derive(Debug, Deserialize)]
struct ToolChatResponse {
    choices: Vec<ToolChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ToolChatChoice {
    message: ToolChatMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ToolChatMessage {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCallFallback>>,
}

#[derive(Debug, Deserialize)]
struct ToolCallFallback {
    id: Option<String>,
    function: Option<ToolCallFunctionFallback>,
}

#[derive(Debug, Default, Deserialize)]
struct ToolCallFunctionFallback {
    name: Option<String>,
    /// Argumente als String oder als Objekt (Objekt -> kompaktes JSON).
    arguments: Option<Value>,
}

fn parse_tool_chat_response(body: &str) -> Result<ChatTurn, ChatError> {
    let response = serde_json::from_str::<ToolChatResponse>(body)
        .map_err(|error| ChatError::InvalidJson(error.to_string()))?;
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or(ChatError::MissingChoice)?;
    if choice.finish_reason.as_deref() == Some("length") {
        return Err(ChatError::TruncatedOutput);
    }
    let content = choice.message.content.unwrap_or_default();
    let tool_calls = choice
        .message
        .tool_calls
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(index, call)| {
            let function = call.function.unwrap_or_default();
            ToolCall {
                id: call
                    .id
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(|| format!("call_{index}")),
                name: function.name.unwrap_or_default(),
                arguments: match function.arguments {
                    Some(Value::String(text)) => text,
                    Some(value) => value.to_string(),
                    None => String::new(),
                },
            }
        })
        .collect::<Vec<_>>();
    let tool_calls = dedupe_ids(tool_calls);
    if content.trim().is_empty() && tool_calls.is_empty() {
        return Err(ChatError::MissingChoice);
    }
    Ok(ChatTurn {
        content,
        tool_calls,
    })
}

#[cfg(test)]
mod tests;
