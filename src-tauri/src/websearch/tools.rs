//! Werkzeug-Laufzeit fuer die Chat-Schleife: Executor, Labels, Events und die
//! Function-Schemas.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{json, Value};
use url::Url;

use super::fetch::fetch_page;
use super::search::{web_search, SearchResult};
use super::{truncate_chars, WebError, FETCH_PAGE_TOOL, MAX_TOOL_LABEL_CHARS, WEB_SEARCH_TOOL};

/// Ergebnis eines Werkzeug-Aufrufs an die Chat-Schleife: `Ok(text)` wird
/// verpackt an das Modell gegeben, `Err(kurze Ursache)` als `Fehler: …`.
pub type ToolFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;
pub type ToolExecutor = Arc<dyn Fn(String, String) -> ToolFuture + Send + Sync>;

/// Laufzeit der Webtools fuer eine Chat-Runde.
#[derive(Clone)]
pub struct ToolRuntime {
    pub execute: ToolExecutor,
}

impl std::fmt::Debug for ToolRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ToolRuntime")
    }
}

/// Aktivitaet eines Werkzeug-Aufrufs (Event `ai:chat_tool`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolEvent {
    pub kind: &'static str,
    pub label: String,
    pub status: &'static str,
}

/// Feste Meldungen der Schleife, wenn ein Aufruf nicht ausgefuehrt werden kann.
pub const TOOL_LIMIT_MESSAGE: &str = "Tool-Limit pro Runde erreicht";
pub const UNKNOWN_TOOL_MESSAGE: &str = "unbekanntes Tool";
pub const INVALID_ARGUMENTS_MESSAGE: &str = "ungültige Tool-Argumente";

/// Laufzeit mit den echten Webtools (SearXNG + Seitenabruf).
pub fn production_runtime(searxng_url: String) -> ToolRuntime {
    ToolRuntime {
        execute: Arc::new(move |name: String, arguments: String| {
            let searxng_url = searxng_url.clone();
            Box::pin(async move {
                match name.as_str() {
                    WEB_SEARCH_TOOL => {
                        let query = string_argument(&arguments, "query")?;
                        let results = web_search(&searxng_url, &query)
                            .await
                            .map_err(search_error_reason)?;
                        Ok(search_results_text(&results))
                    }
                    FETCH_PAGE_TOOL => {
                        let url = string_argument(&arguments, "url")?;
                        fetch_page(&url).await.map_err(|error| error.short_reason())
                    }
                    _ => Err(UNKNOWN_TOOL_MESSAGE.to_string()),
                }
            })
        }),
    }
}

/// Fehler der Suche: Transportprobleme heissen fuer das Modell anders als beim
/// Seitenabruf.
fn search_error_reason(error: WebError) -> String {
    match error {
        WebError::Response(_) => "Suche fehlgeschlagen".to_string(),
        other => other.short_reason(),
    }
}

pub(crate) fn string_argument(arguments: &str, key: &str) -> Result<String, String> {
    let value: Value =
        serde_json::from_str(arguments).map_err(|_| INVALID_ARGUMENTS_MESSAGE.to_string())?;
    match value.get(key).and_then(Value::as_str) {
        Some(text) if !text.trim().is_empty() => Ok(text.to_string()),
        _ => Err(INVALID_ARGUMENTS_MESSAGE.to_string()),
    }
}

/// Kurzes Label fuer die Aktivitaetszeile (`kind == "search"` -> Suchanfrage,
/// sonst `host/pfad`), auf 120 Unicode-Skalarwerte gekuerzt.
pub fn tool_label(name: &str, arguments: &str) -> String {
    let value = serde_json::from_str::<Value>(arguments).unwrap_or(Value::Null);
    let raw = match name {
        WEB_SEARCH_TOOL => value
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        FETCH_PAGE_TOOL => value
            .get("url")
            .and_then(Value::as_str)
            .map(fetch_label)
            .unwrap_or_default(),
        other => other.to_string(),
    };
    truncate_chars(&raw, MAX_TOOL_LABEL_CHARS)
}

pub fn tool_kind(name: &str) -> &'static str {
    if name == FETCH_PAGE_TOOL {
        "fetch"
    } else {
        "search"
    }
}

fn fetch_label(url: &str) -> String {
    match Url::parse(url) {
        Ok(parsed) => {
            let host = parsed.host_str().unwrap_or_default();
            match parsed.path() {
                "" | "/" => host.to_string(),
                path => format!("{host}{path}"),
            }
        }
        Err(_) => url.to_string(),
    }
}

/// Kompakter Text der Trefferliste fuer das Modell.
pub fn search_results_text(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "Keine Treffer.".to_string();
    }
    results
        .iter()
        .enumerate()
        .map(|(index, result)| {
            format!(
                "{}. {}\n{}\n{}",
                index + 1,
                result.title,
                result.url,
                result.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Function-Schemas beider Werkzeuge, wie sie an den Provider gehen.
pub fn tool_definitions() -> Vec<Value> {
    vec![web_search_tool(), fetch_page_tool()]
}

pub fn web_search_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": WEB_SEARCH_TOOL,
            "description": "Search the web. Returns titles, URLs and snippets.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"}
                },
                "required": ["query"]
            }
        }
    })
}

pub fn fetch_page_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": FETCH_PAGE_TOOL,
            "description": "Fetch a public web page and return its text content.",
            "parameters": {
                "type": "object",
                "properties": {
                    "url": {"type": "string"}
                },
                "required": ["url"]
            }
        }
    })
}
