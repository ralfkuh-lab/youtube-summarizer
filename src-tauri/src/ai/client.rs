//! OpenAI-kompatibler Chat-Client mit SSE-Streaming + JSON-Fallback (folio 1:1).

use futures_util::StreamExt;
use reqwest::{Client, StatusCode, Url};
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use std::time::{Duration, Instant};
use thiserror::Error;

pub(crate) const STREAM_CHUNK_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(250);
const MAX_PROVIDER_ERROR_CHARS: usize = 300;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    /// Text der Nachricht. `null` wird nur fuer Assistant-Nachrichten mit
    /// Tool-Aufrufen und leerem Text gesendet (siehe `Serialize`).
    pub content: Option<String>,
    pub tool_calls: Option<Value>,
    pub tool_call_id: Option<String>,
}

/// Serialisierung laut Spec: `content` ist immer vorhanden. JSON `null` gilt
/// ausschliesslich fuer `role == "assistant"` mit nichtleeren `tool_calls` und
/// leerem/fehlendem Text; in allen anderen Faellen wird ein String gesendet
/// (leerer Text als `""`). `tool_calls`/`tool_call_id` werden nur bei `Some`
/// mitgesendet.
impl Serialize for ChatMessage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let text = self.content.as_deref().unwrap_or_default();
        let null_content =
            self.role == "assistant" && has_tool_calls(&self.tool_calls) && text.is_empty();
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("role", &self.role)?;
        match null_content {
            true => map.serialize_entry("content", &Option::<String>::None)?,
            false => map.serialize_entry("content", text)?,
        }
        // Leere oder fehlende Tool-Aufrufe werden nicht gesendet (Provider
        // lehnen `"tool_calls": []` ab).
        if has_tool_calls(&self.tool_calls) {
            if let Some(tool_calls) = &self.tool_calls {
                map.serialize_entry("tool_calls", tool_calls)?;
            }
        }
        if let Some(tool_call_id) = &self.tool_call_id {
            map.serialize_entry("tool_call_id", tool_call_id)?;
        }
        map.end()
    }
}

fn has_tool_calls(tool_calls: &Option<Value>) -> bool {
    match tool_calls {
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Null) | None => false,
        Some(_) => true,
    }
}

/// Konstruktoren fuer alle Rollen; `assistant_tool_calls` und `tool` nutzt die
/// Chat-Schleife erst in Etappe 2b.
#[allow(dead_code)]
impl ChatMessage {
    fn text(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::text("system", content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::text("user", content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::text("assistant", content)
    }

    /// Assistant-Nachricht mit Tool-Aufrufen und ohne Text (JSON `null`).
    pub fn assistant_tool_calls(tool_calls: Value) -> Self {
        Self {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        }
    }

    /// Ergebnis eines Tool-Aufrufs.
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }

    pub fn with_tool_calls(mut self, tool_calls: Value) -> Self {
        self.tool_calls = Some(tool_calls);
        self
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
    #[error(
        "Die KI-Antwort endete vorzeitig ohne Abschlusssignal des Providers, das Ergebnis ist unvollständig - bitte erneut versuchen"
    )]
    IncompleteStream,
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [Value]>,
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
pub(crate) struct SseDecoder {
    buffer: Vec<u8>,
    data_lines: Vec<String>,
}

impl SseDecoder {
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, ChatError> {
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

/// Ergebnis der Auswertung eines SSE-Events.
pub(crate) enum SseStep {
    Continue,
    Done,
}

/// Liest den SSE-Stream und uebergibt jeden Event-Block an `handle_event`.
/// Abbruch-Flag und Chunk-Timeout werden hier zentral geprueft, damit der
/// Tool-Stream (ai/tool_stream.rs) dieselbe Logik nutzen kann.
pub(crate) async fn read_sse_stream(
    response: reqwest::Response,
    mut handle_event: impl FnMut(&str) -> Result<SseStep, ChatError>,
    mut is_cancelled: impl FnMut() -> bool,
) -> Result<(), ChatError> {
    let mut stream = response.bytes_stream();
    let mut decoder = SseDecoder::default();
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
        let chunk = chunk.map_err(|error| ChatError::ResponseRead(error_chain(&error)))?;
        chunk_wait_started = Instant::now();
        for event in decoder.push(&chunk)? {
            if is_cancelled() {
                return Err(ChatError::Cancelled);
            }
            if let SseStep::Done = handle_event(&event)? {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Sendet eine Chat-Anfrage (streamend) und prueft den HTTP-Status. `tools`
/// wird nur mitgesendet, wenn es gesetzt ist (Etappe 2).
pub(crate) async fn send_chat_request(
    http: &Client,
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
    messages: &[ChatMessage],
    tools: Option<&[Value]>,
) -> Result<reqwest::Response, ChatError> {
    let endpoint = chat_url(base_url)?;
    let api_key = api_key.map(str::trim).filter(|key| !key.is_empty());
    // Ohne Werkzeuge kein `tools`-Feld: manche Provider antworten auf
    // `"tools": []` mit HTTP 400.
    let tools = tools.filter(|tools| !tools.is_empty());
    let mut request = http.post(endpoint).json(&ChatRequest {
        model,
        messages,
        stream: true,
        tools,
    });
    if let Some(key) = api_key {
        request = request.bearer_auth(key);
    }

    let response = request
        .send()
        .await
        .map_err(|error| ChatError::Request(error_chain(&error)))?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .map_err(|error| ChatError::ResponseRead(error_chain(&error)))?;
        return Err(ChatError::Http {
            status,
            message: provider_error_message(&body, api_key),
        });
    }
    Ok(response)
}

/// Content-Type der Antwort, kleingeschrieben; leer, wenn keiner gesetzt ist.
pub(crate) fn response_content_type(response: &reqwest::Response) -> String {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase()
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
    let api_key = api_key.map(str::trim).filter(|key| !key.is_empty());
    let response = send_chat_request(http, base_url, api_key, model, messages, None).await?;

    if !response_content_type(&response).starts_with("text/event-stream") {
        let body = response
            .text()
            .await
            .map_err(|error| ChatError::ResponseRead(error_chain(&error)))?;
        let text = parse_chat_response(&body)?;
        on_delta(&text);
        return Ok(text);
    }

    let mut accumulated = String::new();
    let mut finished = false;
    read_sse_stream(
        response,
        |event| {
            if event.trim() == "[DONE]" {
                finished = true;
                return Ok(SseStep::Done);
            }
            if let Some(message) = stream_error_message(event, api_key) {
                return Err(ChatError::Http {
                    status: StatusCode::BAD_GATEWAY,
                    message,
                });
            }
            let response = serde_json::from_str::<ChatStreamResponse>(event)
                .map_err(|error| ChatError::InvalidJson(error.to_string()))?;
            if let Some(choice) = response.choices.into_iter().next() {
                if let Some(content) = choice.delta.content {
                    accumulated.push_str(&content);
                    on_delta(&accumulated);
                }
                if let Some(reason) = choice.finish_reason.as_deref() {
                    if reason == "length" {
                        return Err(ChatError::TruncatedOutput);
                    }
                    if !reason.trim().is_empty() {
                        finished = true;
                    }
                }
            }
            Ok(SseStep::Continue)
        },
        &mut is_cancelled,
    )
    .await?;
    finish_stream(accumulated, finished)
}

pub(crate) fn finish_stream(accumulated: String, completed: bool) -> Result<String, ChatError> {
    if !completed {
        Err(ChatError::IncompleteStream)
    } else if accumulated.trim().is_empty() {
        Err(ChatError::MissingChoice)
    } else {
        Ok(accumulated)
    }
}

pub(crate) fn stream_error_message(body: &str, api_key: Option<&str>) -> Option<String> {
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

/// reqwest zeigt in Display nur die oberste Ebene ("error decoding response
/// body"); die eigentliche Ursache (z. B. ein Timeout) steckt in der
/// source-Kette und gehört mit in die Meldung.
pub(crate) fn error_chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !message.contains(&cause_text) {
            message.push_str(": ");
            message.push_str(&cause_text);
        }
        source = cause.source();
    }
    message
}

pub(crate) fn provider_error_message(body: &str, api_key: Option<&str>) -> String {
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
            tools: None,
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

    /// Reproduziert den Praxisfall: Ein Client-Total-Timeout mitten im
    /// SSE-Stream erscheint bei reqwest nur als "error decoding response
    /// body" - die Meldung muss die Timeout-Ursache mitnennen.
    #[tokio::test]
    async fn stream_abort_by_client_timeout_names_the_cause() {
        use std::io::{BufRead as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                      Transfer-Encoding: chunked\r\n\r\n",
                )
                .unwrap();
            let payload: &[u8] = b"data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n";
            stream
                .write_all(format!("{:x}\r\n", payload.len()).as_bytes())
                .unwrap();
            stream.write_all(payload).unwrap();
            stream.write_all(b"\r\n").unwrap();
            stream.flush().unwrap();
            // Stream offen halten, bis der Client per Timeout abbricht.
            std::thread::sleep(Duration::from_millis(1500));
        });

        let http = Client::builder()
            .timeout(Duration::from_millis(400))
            .build()
            .unwrap();
        let error = chat_stream(
            &http,
            &format!("http://{addr}/v1"),
            None,
            "test-model",
            &[ChatMessage::user("hi")],
            |_| {},
        )
        .await
        .unwrap_err();
        server.join().unwrap();

        let ChatError::ResponseRead(message) = error else {
            panic!("unerwarteter Fehler: {error:?}");
        };
        assert!(
            message.contains("error decoding response body"),
            "{message}"
        );
        assert!(message.to_lowercase().contains("timed out"), "{message}");
    }

    /// Beweist: connect_timeout + read_timeout ohne Gesamt-Timeout lassen
    /// einen Stream durch, dessen Gesamtdauer den read_timeout uebersteigt,
    /// solange die Luecken zwischen Chunks darunter bleiben (wie in lib.rs).
    #[tokio::test]
    async fn stream_completes_when_total_duration_exceeds_read_timeout() {
        use std::io::{BufRead as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let parts = ["Alpha", "Beta", "Gamma", "Delta", "Epsilon"];
        let expected = parts.concat();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                      Transfer-Encoding: chunked\r\n\r\n",
                )
                .unwrap();
            for part in parts {
                let payload =
                    format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{part}\"}}}}]}}\n\n");
                let payload = payload.into_bytes();
                stream
                    .write_all(format!("{:x}\r\n", payload.len()).as_bytes())
                    .unwrap();
                stream.write_all(&payload).unwrap();
                stream.write_all(b"\r\n").unwrap();
                stream.flush().unwrap();
                std::thread::sleep(Duration::from_millis(150));
            }
            let done: &[u8] = b"data: [DONE]\n\n";
            stream
                .write_all(format!("{:x}\r\n", done.len()).as_bytes())
                .unwrap();
            stream.write_all(done).unwrap();
            stream.write_all(b"\r\n").unwrap();
            stream.write_all(b"0\r\n\r\n").unwrap();
            stream.flush().unwrap();
        });

        let http = Client::builder()
            .connect_timeout(Duration::from_millis(400))
            .read_timeout(Duration::from_millis(400))
            .build()
            .unwrap();
        let text = chat_stream(
            &http,
            &format!("http://{addr}/v1"),
            None,
            "test-model",
            &[ChatMessage::user("hi")],
            |_| {},
        )
        .await
        .expect("stream should complete without a total timeout");
        server.join().unwrap();
        assert_eq!(expected, text);
    }

    #[test]
    fn error_chain_appends_sources_without_duplicates() {
        #[derive(Debug)]
        struct Wrapper(std::io::Error);
        impl std::fmt::Display for Wrapper {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "error decoding response body")
            }
        }
        impl std::error::Error for Wrapper {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }

        let wrapped = Wrapper(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "operation timed out",
        ));
        assert_eq!(
            "error decoding response body: operation timed out",
            error_chain(&wrapped)
        );

        let plain = std::io::Error::new(std::io::ErrorKind::Other, "nur eine Ebene");
        assert_eq!("nur eine Ebene", error_chain(&plain));
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

    #[tokio::test]
    async fn stream_without_done_or_finish_reason_fails_with_incomplete_stream() {
        use std::io::{BufRead as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                      Transfer-Encoding: chunked\r\n\r\n",
                )
                .unwrap();
            let chunk1: &[u8] = b"data: {\"choices\":[{\"delta\":{\"content\":\"Teil 1, \"}}]}\n\n";
            stream
                .write_all(format!("{:x}\r\n", chunk1.len()).as_bytes())
                .unwrap();
            stream.write_all(chunk1).unwrap();
            stream.write_all(b"\r\n").unwrap();
            stream.flush().unwrap();

            let chunk2: &[u8] = b"data: {\"choices\":[{\"delta\":{\"content\":\"Teil 2\"}}]}\n\n";
            stream
                .write_all(format!("{:x}\r\n", chunk2.len()).as_bytes())
                .unwrap();
            stream.write_all(chunk2).unwrap();
            stream.write_all(b"\r\n").unwrap();
            stream.flush().unwrap();

            // Stream ohne [DONE] und ohne finish_reason regulär beenden
            stream.write_all(b"0\r\n\r\n").unwrap();
            stream.flush().unwrap();
        });

        let http = Client::builder().build().unwrap();
        let error = chat_stream(
            &http,
            &format!("http://{addr}/v1"),
            None,
            "test-model",
            &[ChatMessage::user("hi")],
            |_| {},
        )
        .await
        .unwrap_err();
        server.join().unwrap();

        assert!(
            matches!(error, ChatError::IncompleteStream),
            "unexpected: {error:?}"
        );
        assert_eq!(
            error.to_string(),
            "Die KI-Antwort endete vorzeitig ohne Abschlusssignal des Providers, das Ergebnis ist unvollständig - bitte erneut versuchen"
        );
    }

    #[tokio::test]
    async fn stream_with_finish_reason_completes_without_done() {
        use std::io::{BufRead as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                      Transfer-Encoding: chunked\r\n\r\n",
                )
                .unwrap();
            let chunk1: &[u8] = b"data: {\"choices\":[{\"delta\":{\"content\":\"Hallo \"}}]}\n\n";
            stream
                .write_all(format!("{:x}\r\n", chunk1.len()).as_bytes())
                .unwrap();
            stream.write_all(chunk1).unwrap();
            stream.write_all(b"\r\n").unwrap();
            stream.flush().unwrap();

            let chunk2: &[u8] = b"data: {\"choices\":[{\"delta\":{\"content\":\"Welt\"},\"finish_reason\":\"stop\"}]}\n\n";
            stream
                .write_all(format!("{:x}\r\n", chunk2.len()).as_bytes())
                .unwrap();
            stream.write_all(chunk2).unwrap();
            stream.write_all(b"\r\n").unwrap();
            stream.flush().unwrap();

            // Stream ohne [DONE] beenden
            stream.write_all(b"0\r\n\r\n").unwrap();
            stream.flush().unwrap();
        });

        let http = Client::builder().build().unwrap();
        let text = chat_stream(
            &http,
            &format!("http://{addr}/v1"),
            None,
            "test-model",
            &[ChatMessage::user("hi")],
            |_| {},
        )
        .await
        .expect("stream with finish_reason should succeed");
        server.join().unwrap();

        assert_eq!(text, "Hallo Welt");
    }

    /// Der extrahierte `read_sse_stream` muss fuer den strengen Pfad weiterhin
    /// `finish_reason: "length"` und ein gesetztes Abbruch-Flag erkennen.
    #[tokio::test]
    async fn stream_with_finish_reason_length_is_truncated() {
        use std::io::{BufRead as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            let event =
                "data: {\"choices\":[{\"delta\":{\"content\":\"Teil\"},\"finish_reason\":\"length\"}]}\n\n";
            let body = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{event}",
                event.len()
            );
            stream.write_all(body.as_bytes()).unwrap();
            stream.flush().unwrap();
        });

        let http = Client::builder().build().unwrap();
        let error = chat_stream(
            &http,
            &format!("http://{addr}/v1"),
            None,
            "test-model",
            &[ChatMessage::user("hi")],
            |_| {},
        )
        .await
        .unwrap_err();
        server.join().unwrap();

        assert!(
            matches!(error, ChatError::TruncatedOutput),
            "unerwartet: {error:?}"
        );
    }

    #[tokio::test]
    async fn cancelled_flag_aborts_the_stream() {
        use std::io::{BufRead as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            let event = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n";
            let body = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{event}",
                event.len()
            );
            stream.write_all(body.as_bytes()).unwrap();
            stream.flush().unwrap();
        });

        let http = Client::builder().build().unwrap();
        let error = chat_stream_cancellable(
            &http,
            &format!("http://{addr}/v1"),
            None,
            "test-model",
            &[ChatMessage::user("hi")],
            |_| {},
            || true,
        )
        .await
        .unwrap_err();
        server.join().unwrap();

        assert!(
            matches!(error, ChatError::Cancelled),
            "unerwartet: {error:?}"
        );
    }

    #[tokio::test]
    async fn stream_with_done_completes_successfully() {
        use std::io::{BufRead as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                      Transfer-Encoding: chunked\r\n\r\n",
                )
                .unwrap();
            let chunk1: &[u8] = b"data: {\"choices\":[{\"delta\":{\"content\":\"Erfolg\"}}]}\n\n";
            stream
                .write_all(format!("{:x}\r\n", chunk1.len()).as_bytes())
                .unwrap();
            stream.write_all(chunk1).unwrap();
            stream.write_all(b"\r\n").unwrap();
            stream.flush().unwrap();

            let done: &[u8] = b"data: [DONE]\n\n";
            stream
                .write_all(format!("{:x}\r\n", done.len()).as_bytes())
                .unwrap();
            stream.write_all(done).unwrap();
            stream.write_all(b"\r\n").unwrap();
            stream.flush().unwrap();
        });

        let http = Client::builder().build().unwrap();
        let text = chat_stream(
            &http,
            &format!("http://{addr}/v1"),
            None,
            "test-model",
            &[ChatMessage::user("hi")],
            |_| {},
        )
        .await
        .expect("stream with [DONE] should succeed");
        server.join().unwrap();

        assert_eq!(text, "Erfolg");
    }

    #[tokio::test]
    async fn stream_with_finish_reason_and_trailing_usage_event_completes() {
        use std::io::{BufRead as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                      Transfer-Encoding: chunked\r\n\r\n",
                )
                .unwrap();
            let chunk1: &[u8] = b"data: {\"choices\":[{\"delta\":{\"content\":\"Nach\"}}]}\n\n";
            stream
                .write_all(format!("{:x}\r\n", chunk1.len()).as_bytes())
                .unwrap();
            stream.write_all(chunk1).unwrap();
            stream.write_all(b"\r\n").unwrap();
            stream.flush().unwrap();

            let chunk2: &[u8] =
                b"data: {\"choices\":[{\"delta\":{\"content\":\"richt\"},\"finish_reason\":\"stop\"}]}\n\n";
            stream
                .write_all(format!("{:x}\r\n", chunk2.len()).as_bytes())
                .unwrap();
            stream.write_all(chunk2).unwrap();
            stream.write_all(b"\r\n").unwrap();
            stream.flush().unwrap();

            // Nachgelagertes usage-Event mit leerem choices
            let chunk3: &[u8] = b"data: {\"choices\":[],\"usage\":{\"total_tokens\":3}}\n\n";
            stream
                .write_all(format!("{:x}\r\n", chunk3.len()).as_bytes())
                .unwrap();
            stream.write_all(chunk3).unwrap();
            stream.write_all(b"\r\n").unwrap();
            stream.flush().unwrap();

            // Verbindungsende ohne [DONE]
            stream.write_all(b"0\r\n\r\n").unwrap();
            stream.flush().unwrap();
        });

        let http = Client::builder().build().unwrap();
        let text = chat_stream(
            &http,
            &format!("http://{addr}/v1"),
            None,
            "test-model",
            &[ChatMessage::user("hi")],
            |_| {},
        )
        .await
        .expect("stream with finish_reason and trailing usage event should succeed");
        server.join().unwrap();

        assert_eq!(text, "Nachricht");
    }
}
