//! OpenAI-kompatibler Chat-Client mit SSE-Streaming + JSON-Fallback (folio 1:1).

use futures_util::StreamExt;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};
use thiserror::Error;

const STREAM_CHUNK_TIMEOUT: Duration = Duration::from_secs(60);
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(250);
const MAX_PROVIDER_ERROR_CHARS: usize = 300;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ChatError {
    #[error("Ungültige Provider-Basis-URL: {0}")]
    InvalidBaseUrl(String),
    #[error("Provider-Basis-URL muss eine HTTP(S)-URL sein")]
    UnsupportedUrl,
    #[error("KI-Anfrage fehlgeschlagen: {0}")]
    Request(String),
    #[error("KI-Antwort konnte nicht gelesen werden: {0}")]
    ResponseRead(String),
    #[error("KI-Provider antwortete mit HTTP-Status {status}: {message}")]
    Http { status: StatusCode, message: String },
    #[error("KI-Antwort enthält ungültiges JSON: {0}")]
    InvalidJson(String),
    #[error("KI-Antwort enthält keine Text-Antwort in choices[0]")]
    MissingChoice,
    #[error("KI-Antwort abgebrochen")]
    Cancelled,
    #[error(
        "Die KI-Antwort wurde am Output-Limit des Modells abgeschnitten \
         (finish_reason=length). Das Dokument in kleinere Dateien teilen \
         oder ein Modell mit größerem Output-Limit wählen."
    )]
    TruncatedOutput,
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatResponseMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatStreamResponse {
    choices: Vec<ChatStreamChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatStreamChoice {
    delta: ChatStreamDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatStreamDelta {
    content: Option<String>,
}

#[derive(Debug, Default)]
struct SseDecoder {
    buffer: Vec<u8>,
    data_lines: Vec<String>,
}

impl SseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, ChatError> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();

        while let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line = self.buffer.drain(..=newline).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = std::str::from_utf8(&line)
                .map_err(|error| ChatError::InvalidJson(error.to_string()))?;
            if line.is_empty() {
                if !self.data_lines.is_empty() {
                    events.push(self.data_lines.join("\n"));
                    self.data_lines.clear();
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                self.data_lines
                    .push(data.strip_prefix(' ').unwrap_or(data).to_string());
            }
        }

        Ok(events)
    }
}

pub async fn chat_stream(
    http: &Client,
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
    messages: &[ChatMessage],
    on_delta: impl FnMut(&str),
) -> Result<String, ChatError> {
    chat_stream_cancellable(http, base_url, api_key, model, messages, on_delta, || false).await
}

pub async fn chat_stream_cancellable(
    http: &Client,
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
    messages: &[ChatMessage],
    mut on_delta: impl FnMut(&str),
    mut is_cancelled: impl FnMut() -> bool,
) -> Result<String, ChatError> {
    let endpoint = chat_url(base_url)?;
    let api_key = api_key.map(str::trim).filter(|key| !key.is_empty());
    let mut request = http.post(endpoint).json(&ChatRequest {
        model,
        messages,
        stream: true,
    });
    if let Some(key) = api_key {
        request = request.bearer_auth(key);
    }

    let response = request
        .send()
        .await
        .map_err(|error| ChatError::Request(error.to_string()))?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if !status.is_success() {
        let body = response
            .text()
            .await
            .map_err(|error| ChatError::ResponseRead(error.to_string()))?;
        return Err(ChatError::Http {
            status,
            message: provider_error_message(&body, api_key),
        });
    }

    if !content_type.starts_with("text/event-stream") {
        let body = response
            .text()
            .await
            .map_err(|error| ChatError::ResponseRead(error.to_string()))?;
        let text = parse_chat_response(&body)?;
        on_delta(&text);
        return Ok(text);
    }

    let mut stream = response.bytes_stream();
    let mut decoder = SseDecoder::default();
    let mut accumulated = String::new();
    let mut chunk_wait_started = Instant::now();
    loop {
        if is_cancelled() {
            return Err(ChatError::Cancelled);
        }
        let elapsed = chunk_wait_started.elapsed();
        if elapsed >= STREAM_CHUNK_TIMEOUT {
            return Err(ChatError::ResponseRead(
                "Zeitüberschreitung beim Warten auf den nächsten Stream-Chunk".to_string(),
            ));
        }
        let wait = CANCEL_POLL_INTERVAL.min(STREAM_CHUNK_TIMEOUT - elapsed);
        let next = match tokio::time::timeout(wait, stream.next()).await {
            Ok(next) => next,
            Err(_) => continue,
        };
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.map_err(|error| ChatError::ResponseRead(error.to_string()))?;
        chunk_wait_started = Instant::now();
        for event in decoder.push(&chunk)? {
            if is_cancelled() {
                return Err(ChatError::Cancelled);
            }
            if event.trim() == "[DONE]" {
                return finish_stream(accumulated);
            }
            if let Some(message) = stream_error_message(&event, api_key) {
                return Err(ChatError::Http {
                    status: StatusCode::BAD_GATEWAY,
                    message,
                });
            }
            let response = serde_json::from_str::<ChatStreamResponse>(&event)
                .map_err(|error| ChatError::InvalidJson(error.to_string()))?;
            if let Some(choice) = response.choices.into_iter().next() {
                if let Some(content) = choice.delta.content {
                    accumulated.push_str(&content);
                    on_delta(&accumulated);
                }
                if choice.finish_reason.as_deref() == Some("length") {
                    return Err(ChatError::TruncatedOutput);
                }
            }
        }
    }
    finish_stream(accumulated)
}

fn finish_stream(accumulated: String) -> Result<String, ChatError> {
    if accumulated.trim().is_empty() {
        Err(ChatError::MissingChoice)
    } else {
        Ok(accumulated)
    }
}

fn stream_error_message(body: &str, api_key: Option<&str>) -> Option<String> {
    serde_json::from_str::<Value>(body)
        .ok()
        .filter(|value| value.get("error").is_some())
        .map(|_| provider_error_message(body, api_key))
}

pub(crate) fn chat_url(base_url: &str) -> Result<Url, ChatError> {
    let mut url = Url::parse(base_url.trim())
        .map_err(|error| ChatError::InvalidBaseUrl(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(ChatError::UnsupportedUrl);
    }
    let path = url.path().trim_end_matches('/').to_string();
    if !path.ends_with("/chat/completions") {
        url.set_path(&format!("{path}/chat/completions"));
    } else {
        url.set_path(&path);
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn parse_chat_response(body: &str) -> Result<String, ChatError> {
    let response = serde_json::from_str::<ChatResponse>(body)
        .map_err(|error| ChatError::InvalidJson(error.to_string()))?;
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or(ChatError::MissingChoice)?;
    if choice.finish_reason.as_deref() == Some("length") {
        return Err(ChatError::TruncatedOutput);
    }
    Some(choice.message.content)
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| ChatError::MissingChoice)
}

fn provider_error_message(body: &str, api_key: Option<&str>) -> String {
    let parsed = serde_json::from_str::<Value>(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/message").and_then(Value::as_str))
        .or_else(|| {
            parsed
                .as_ref()
                .and_then(|value| value.get("message").and_then(Value::as_str))
        })
        .unwrap_or(body);
    let compact = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let redacted = match api_key.filter(|key| !key.is_empty()) {
        Some(key) => compact.replace(key, "[REDACTED]"),
        None => compact,
    };
    let shortened = redacted
        .chars()
        .take(MAX_PROVIDER_ERROR_CHARS)
        .collect::<String>();
    if shortened.is_empty() {
        "keine Fehlermeldung".to_string()
    } else {
        shortened
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_url_appends_path_once_and_strips_url_extras() {
        assert_eq!(
            "http://localhost:11434/v1/chat/completions",
            chat_url("http://localhost:11434/v1").unwrap().as_str()
        );
        assert_eq!(
            "https://example.test/v1/chat/completions",
            chat_url("https://example.test/v1/chat/completions/?token=ignored#fragment")
                .unwrap()
                .as_str()
        );
        assert!(chat_url("file:///tmp/provider").is_err());
    }

    #[test]
    fn request_payload_uses_openai_shape() {
        let messages = vec![ChatMessage::system("rules"), ChatMessage::user("document")];
        let value = serde_json::to_value(ChatRequest {
            model: "test-model",
            messages: &messages,
            stream: false,
        })
        .unwrap();
        assert_eq!("test-model", value["model"]);
        assert_eq!("system", value["messages"][0]["role"]);
        assert_eq!("document", value["messages"][1]["content"]);
        assert!(value.get("stream").is_none());
    }

    #[test]
    fn response_parser_rejects_length_truncated_output() {
        assert!(matches!(
            parse_chat_response(
                r#"{"choices":[{"message":{"content":"Halb"},"finish_reason":"length"}]}"#
            ),
            Err(ChatError::TruncatedOutput)
        ));
        assert_eq!(
            "Ganz",
            parse_chat_response(
                r#"{"choices":[{"message":{"content":"Ganz"},"finish_reason":"stop"}]}"#
            )
            .unwrap()
        );
    }

    #[test]
    fn response_parser_reads_first_choice() {
        let body = r##"{"choices":[{"message":{"role":"assistant","content":"# Summary"}},{"message":{"content":"ignored"}}]}"##;
        assert_eq!("# Summary", parse_chat_response(body).unwrap());
    }

    #[test]
    fn response_parser_rejects_broken_json_and_empty_choices() {
        assert!(matches!(
            parse_chat_response("{broken"),
            Err(ChatError::InvalidJson(_))
        ));
        assert!(matches!(
            parse_chat_response(r#"{"choices":[]}"#),
            Err(ChatError::MissingChoice)
        ));
        assert!(matches!(
            parse_chat_response(r#"{"choices":[{"message":{"content":"  "}}]}"#),
            Err(ChatError::MissingChoice)
        ));
    }

    #[test]
    fn provider_errors_are_compact_bounded_and_redact_keys() {
        let key = "top-secret";
        let body = format!(
            r#"{{"error":{{"message":"failed with {key} {}"}}}}"#,
            "x".repeat(400)
        );
        let message = provider_error_message(&body, Some(key));
        assert!(!message.contains(key));
        assert!(message.contains("[REDACTED]"));
        assert!(message.chars().count() <= MAX_PROVIDER_ERROR_CHARS);
        assert!(!message.contains('\n'));
    }
}
