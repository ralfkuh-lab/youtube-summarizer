//! Referenzfaelle T1-T10 aus docs/spec-video-chat.md, Etappe 2a.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::{chat_stream_with_tools_cancellable, ChatTurn, ToolCall};
use crate::ai::client::{ChatError, ChatMessage};
use crate::websearch;

/// Lokaler Provider fuer die Stream-Tests (Muster der Client-Tests).
struct TestServer {
    addr: SocketAddr,
    requests: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<String>>>,
}

impl TestServer {
    fn start<F>(handler: F) -> Self
    where
        F: Fn(&mut TcpStream) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let bodies: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let counter = requests.clone();
        let recorded = bodies.clone();
        let handler = Arc::new(handler);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                counter.fetch_add(1, Ordering::SeqCst);
                let handler = handler.clone();
                let recorded = recorded.clone();
                std::thread::spawn(move || {
                    let body = read_request(&mut stream);
                    recorded.lock().unwrap().push(body);
                    handler(&mut stream);
                });
            }
        });
        Self {
            addr,
            requests,
            bodies,
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}/v1", self.addr)
    }

    fn bodies(&self) -> Vec<String> {
        self.bodies.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        let _ = reader.read_exact(&mut body);
    }
    String::from_utf8_lossy(&body).to_string()
}

fn respond_sse(stream: &mut TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
    stream.flush().unwrap();
}

fn respond_json(stream: &mut TcpStream, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
    stream.flush().unwrap();
}

/// Fuehrt einen Tool-Stream gegen einen Server mit festem Body aus.
async fn turn_from(body: &'static str, content_type: &'static str) -> Result<ChatTurn, ChatError> {
    let server = TestServer::start(move |stream| {
        if content_type == "text/event-stream" {
            respond_sse(stream, body);
        } else {
            respond_json(stream, body);
        }
    });
    let http = reqwest::Client::new();
    chat_stream_with_tools_cancellable(
        &http,
        &server.base_url(),
        None,
        "test-model",
        &[ChatMessage::user("hi")],
        &websearch::tool_definitions(),
        |_| {},
        || false,
    )
    .await
}

fn call(id: &str, name: &str, arguments: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: arguments.to_string(),
    }
}

const T1_STREAM: &str = r#"data: {"choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"web_search","arguments":""}}]}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"query\":"}}]}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"rust sse\"}"}}]}}]}

data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}

data: [DONE]
"#;

#[tokio::test]
async fn t1_reference_stream_builds_one_call() {
    let server = TestServer::start(|stream| respond_sse(stream, T1_STREAM));
    let http = reqwest::Client::new();
    let turn = chat_stream_with_tools_cancellable(
        &http,
        &server.base_url(),
        None,
        "test-model",
        &[ChatMessage::user("hi")],
        &websearch::tool_definitions(),
        |_| {},
        || false,
    )
    .await
    .unwrap();

    assert_eq!(turn.content, "");
    assert_eq!(
        turn.tool_calls,
        vec![call("call_1", "web_search", r#"{"query":"rust sse"}"#)]
    );

    // Die Anfrage enthaelt die Tool-Schemas und die Nachrichten.
    let body: Value = serde_json::from_str(&server.bodies()[0]).unwrap();
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["stream"], true);
    assert_eq!(body["messages"][0]["role"], "user");
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0]["function"]["name"], "web_search");
    assert_eq!(tools[1]["function"]["name"], "fetch_page");
}

#[tokio::test]
async fn t2_finish_reason_stop_yields_the_same_result() {
    let stream = T1_STREAM.replace(
        "\"finish_reason\":\"tool_calls\"",
        "\"finish_reason\":\"stop\"",
    );
    let turn = turn_from(Box::leak(stream.into_boxed_str()), "text/event-stream")
        .await
        .unwrap();

    assert_eq!(turn.content, "");
    assert_eq!(
        turn.tool_calls,
        vec![call("call_1", "web_search", r#"{"query":"rust sse"}"#)]
    );
}

#[tokio::test]
async fn t3_fragments_without_index_land_in_one_call() {
    let body = r#"data: {"choices":[{"delta":{"tool_calls":[{"id":"call_9","function":{"name":"web_search","arguments":"{\"q\":"}}]}}]}

data: {"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"\"x\"}"}}]}}]}

data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}

data: [DONE]
"#;
    let turn = turn_from(body, "text/event-stream").await.unwrap();

    assert_eq!(
        turn.tool_calls.len(),
        1,
        "ohne index gehoeren alle Fragmente zu Index 0"
    );
    assert_eq!(
        turn.tool_calls[0],
        call("call_9", "web_search", r#"{"q":"x"}"#)
    );
}

#[tokio::test]
async fn t4_interleaved_calls_keep_their_arguments_apart() {
    let body = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"web_search","arguments":"{\"query\":"}}]}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_b","function":{"name":"fetch_page","arguments":"{\"url\":"}}]}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"a\"}"}}]}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"\"b\"}"}}]}}]}

data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}

data: [DONE]
"#;
    let turn = turn_from(body, "text/event-stream").await.unwrap();

    assert_eq!(
        turn.tool_calls,
        vec![
            call("call_a", "web_search", r#"{"query":"a"}"#),
            call("call_b", "fetch_page", r#"{"url":"b"}"#),
        ]
    );
}

#[tokio::test]
async fn t5_json_fallback_accepts_object_arguments() {
    let body = r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"web_search","arguments":{"query":"x"}}}]},"finish_reason":"tool_calls"}]}"#;
    let turn = turn_from(body, "application/json").await.unwrap();

    assert_eq!(turn.content, "");
    assert_eq!(
        turn.tool_calls,
        vec![call("call_1", "web_search", r#"{"query":"x"}"#)]
    );
}

#[tokio::test]
async fn t6_tools_only_without_text_is_not_missing_choice() {
    let body = r#"data: {"choices":[{"delta":{"content":null,"tool_calls":[{"index":0,"id":"call_1","function":{"name":"web_search","arguments":"{}"}}]}}]}

data: [DONE]

"#;
    let turn = turn_from(body, "text/event-stream").await.unwrap();

    assert_eq!(turn.content, "");
    assert_eq!(turn.tool_calls.len(), 1);
}

#[tokio::test]
async fn t7_text_and_tool_call_are_both_returned() {
    let body = r#"data: {"choices":[{"delta":{"content":"Hi"}}]}

data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"web_search","arguments":"{}"}}]}}]}

data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}

data: [DONE]
"#;
    let turn = turn_from(body, "text/event-stream").await.unwrap();

    assert_eq!(turn.content, "Hi");
    assert_eq!(turn.tool_calls, vec![call("call_1", "web_search", "{}")]);
}

#[tokio::test]
async fn t8_without_text_and_tools_missing_choice() {
    let body = r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}

data: [DONE]
"#;
    let error = turn_from(body, "text/event-stream").await.unwrap_err();

    assert!(
        matches!(error, ChatError::MissingChoice),
        "unerwartet: {error:?}"
    );
}

#[test]
fn t9_assistant_with_tool_calls_serializes_null_content() {
    let message = ChatMessage::assistant("").with_tool_calls(json!([{
        "id": "call_1",
        "type": "function",
        "function": {"name": "web_search", "arguments": "{}"},
    }]));
    let value = serde_json::to_value(&message).unwrap();

    assert_eq!(value["role"], "assistant");
    assert!(value.as_object().unwrap().contains_key("content"));
    assert!(
        value["content"].is_null(),
        "content muss null sein: {value}"
    );
    assert_eq!(value["tool_calls"][0]["id"], "call_1");
    assert!(
        value.get("tool_call_id").is_none(),
        "tool_call_id wird weggelassen"
    );

    // Auch eine aus der DB gelesene Assistant-Nachricht (content = "") wird null.
    let from_db = ChatMessage {
        role: "assistant".to_string(),
        content: Some(String::new()),
        tool_calls: Some(json!([])),
        tool_call_id: None,
    };
    assert!(serde_json::to_value(&from_db).unwrap()["content"].is_null());

    // Text bleibt ein String.
    let with_text = ChatMessage::assistant("Antwort");
    assert_eq!(
        serde_json::to_value(&with_text).unwrap()["content"],
        "Antwort"
    );
}

#[tokio::test]
async fn t10_missing_id_falls_back_to_call_index() {
    let body = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"web_search","arguments":"{}"}},{"index":1,"function":{"name":"fetch_page","arguments":"{}"}}]}}]}

data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}

data: [DONE]
"#;
    let turn = turn_from(body, "text/event-stream").await.unwrap();

    assert_eq!(turn.tool_calls[0].id, "call_0");
    assert_eq!(turn.tool_calls[1].id, "call_1");
}
