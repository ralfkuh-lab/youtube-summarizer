//! Websuche (SearXNG) und Seitenabruf fuer das Tool-Calling.
//!
//! Beide Wege nutzen einen **eigenen** HTTP-Client: der Provider-Schluessel darf
//! nie an Tool-Ziele gehen, und `fetch_page` prueft Adressen und Redirects
//! selbst (kein Cookie-Store, keine Auth-Header, keine automatischen
//! Weiterleitungen).
//!
//! Etappe 2a: die Aufrufer in der Chat-Schleife folgen in Etappe 2b; bis dahin
//! sind Teile dieses Moduls nur ueber die Tests erreichbar.
#![allow(dead_code)]

mod address;
pub mod config;
mod html;

use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tauri::State;

use futures_util::StreamExt;
use reqwest::redirect::Policy;
use reqwest::{Client, Url};
use serde_json::{json, Value};

use crate::storage::{AppPaths, AppResult};
use address::{check_fetch_url, FilteredResolver};
use config::WebSearchConfig;

pub use address::is_blocked_ip;
pub use html::html_to_text;

/// Zeitlimit fuer Suchanfragen und Seitenabrufe.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Hoechstens so viele Weiterleitungen werden verfolgt.
const MAX_REDIRECTS: usize = 5;
/// Groesste akzeptierte Antwortgroesse (Bytes).
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Gesamtbudget eines `fetch_page`-Aufrufs (inkl. Weiterleitungen und Body).
const FETCH_BUDGET: Duration = Duration::from_secs(30);
const MAX_SEARCH_RESULTS: usize = 8;
const MAX_SNIPPET_CHARS: usize = 300;
const MAX_PAGE_CHARS: usize = 12_000;
const MAX_TOOL_LABEL_CHARS: usize = 120;
const USER_AGENT: &str = "Mozilla/5.0 YouTubeSummarizer/0.1";

pub const WEB_SEARCH_TOOL: &str = "web_search";
pub const FETCH_PAGE_TOOL: &str = "fetch_page";

/// Fehler der Web-Tools. Die Texte sind kurz; die Chat-Schleife bildet sie auf
/// die festen Meldungen an das Modell ab (`model_message`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebError {
    /// Adresse oder Host ist gesperrt (Loopback, privat, nicht oeffentlich).
    AddressNotAllowed,
    /// URL unbrauchbar (Schema, Zugangsdaten, fehlender Host).
    InvalidUrl,
    /// DNS lieferte ausschliesslich gesperrte Adressen.
    NoAllowedAddress,
    Timeout,
    HttpStatus(u16),
    UnsupportedContentType(String),
    BodyTooLarge,
    TooManyRedirects,
    /// Antwort ohne Content-Type (K9).
    MissingContentType,
    /// Nach der Extraktion blieb kein Text uebrig (K10).
    EmptyText,
    /// Die SearXNG-Instanz leitet weiter (K4).
    SearchRedirect(String),
    /// Sonstiger Fehler beim Lesen der Antwort.
    Response(String),
}

impl WebError {
    /// Text, den das Modell als Tool-Ergebnis sieht.
    pub fn model_message(&self) -> String {
        format!("Fehler: {}", self.short_reason())
    }

    /// Kurze Ursache ohne Praefix (die Schleife setzt `Fehler: ` davor).
    pub fn short_reason(&self) -> String {
        match self {
            WebError::AddressNotAllowed | WebError::NoAllowedAddress => {
                "Adresse nicht erlaubt".to_string()
            }
            // Eine unbrauchbare URL ist ein Argumentfehler des Modells.
            WebError::InvalidUrl => INVALID_ARGUMENTS_MESSAGE.to_string(),
            other => other.to_string(),
        }
    }
}

impl std::fmt::Display for WebError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebError::AddressNotAllowed => write!(formatter, "Adresse nicht erlaubt"),
            WebError::InvalidUrl => write!(formatter, "ungültige URL"),
            WebError::NoAllowedAddress => write!(formatter, "keine erlaubte Adresse"),
            WebError::Timeout => write!(formatter, "Zeitüberschreitung"),
            WebError::HttpStatus(status) => write!(formatter, "HTTP-Status {status}"),
            WebError::UnsupportedContentType(value) => write!(formatter, "Content-Type {value}"),
            WebError::BodyTooLarge => write!(formatter, "Antwort zu groß"),
            WebError::TooManyRedirects => write!(formatter, "zu viele Weiterleitungen"),
            WebError::MissingContentType => write!(formatter, "Antwort ohne Content-Type"),
            WebError::EmptyText => write!(formatter, "kein Text extrahiert"),
            WebError::SearchRedirect(location) => write!(
                formatter,
                "SearXNG leitet weiter nach {location} – bitte die endgültige URL eintragen"
            ),
            WebError::Response(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for WebError {}

// ------------------------------------------------------------ Seitenabruf --

/// Laedt eine oeffentliche Seite und liefert den extrahierten Text (hoechstens
/// 12 000 Unicode-Skalarwerte).
pub async fn fetch_page(url: &str) -> Result<String, WebError> {
    fetch_page_with(url, Arc::new(is_blocked_ip), FETCH_BUDGET).await
}

/// Interne Fassung mit injizierbarem Adresspraedikat: die Produktion uebergibt
/// fest `is_blocked_ip`, nur Tests duerfen den lokalen Testserver zulassen.
async fn fetch_page_with(
    url: &str,
    blocked: Arc<dyn Fn(IpAddr) -> bool + Send + Sync>,
    budget: Duration,
) -> Result<String, WebError> {
    match tokio::time::timeout(budget, fetch_page_inner(url, blocked)).await {
        Ok(result) => result,
        Err(_) => Err(WebError::Timeout),
    }
}

async fn fetch_page_inner(
    url: &str,
    blocked: Arc<dyn Fn(IpAddr) -> bool + Send + Sync>,
) -> Result<String, WebError> {
    let client = Client::builder()
        .redirect(Policy::none())
        // Kein Proxy: HTTP_PROXY/ALL_PROXY wuerden den Zielhost ungeprueft an
        // einen fremden Proxy schicken (SSRF-Schutz wird damit umgangen).
        .no_proxy()
        // Kein Cookie-Store, keine Auth-Header; eigener Resolver mit Filter.
        .dns_resolver(Arc::new(FilteredResolver {
            blocked: blocked.clone(),
        }))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| WebError::Response(error.to_string()))?;

    let mut current = Url::parse(url.trim()).map_err(|_| WebError::InvalidUrl)?;
    for hop in 0..=MAX_REDIRECTS {
        check_fetch_url(&current, blocked.as_ref())?;
        let response = client
            .get(current.clone())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(map_request_error)?;
        let status = response.status();

        if status.is_redirection() {
            if hop == MAX_REDIRECTS {
                return Err(WebError::TooManyRedirects);
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(WebError::HttpStatus(status.as_u16()))?;
            // Relative Ziele werden gegen die aktuelle URL aufgeloest und im
            // naechsten Durchlauf erneut geprueft.
            current = current.join(location).map_err(|_| WebError::InvalidUrl)?;
            continue;
        }

        if !status.is_success() {
            return Err(WebError::HttpStatus(status.as_u16()));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .ok_or(WebError::MissingContentType)?;
        let media_type = media_type_of(content_type);
        if !is_allowed_media_type(&media_type) {
            return Err(WebError::UnsupportedContentType(media_type));
        }
        let body = read_body_limited(response, MAX_BODY_BYTES).await?;
        let raw = String::from_utf8_lossy(&body).to_string();
        // Die Extraktion laeuft im Blocking-Pool: sonst koennte das
        // Gesamtbudget nicht greifen, waehrend sie rechnet.
        let text = if media_type == "text/plain" {
            raw
        } else {
            tokio::task::spawn_blocking(move || html_to_text(&raw))
                .await
                .map_err(|error| WebError::Response(error.to_string()))?
        };
        if text.trim().is_empty() {
            return Err(WebError::EmptyText);
        }
        return Ok(truncate_chars(&text, MAX_PAGE_CHARS));
    }
    Err(WebError::TooManyRedirects)
}

fn map_request_error(error: reqwest::Error) -> WebError {
    // Fehler des eigenen Resolvers (gefilterte Adresse) stecken in der
    // Quellkette; sie werden unveraendert weitergegeben.
    if let Some(web_error) = find_web_error(&error) {
        return web_error;
    }
    if error.is_timeout() {
        WebError::Timeout
    } else {
        WebError::Response(crate::ai::client::error_chain(&error))
    }
}

fn find_web_error(error: &(dyn std::error::Error + 'static)) -> Option<WebError> {
    let mut current = Some(error);
    while let Some(cause) = current {
        if let Some(web_error) = cause.downcast_ref::<WebError>() {
            return Some(web_error.clone());
        }
        current = cause.source();
    }
    None
}

/// Media-Type ohne Parameter, kleingeschrieben.
fn media_type_of(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn is_allowed_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "text/html" | "application/xhtml+xml" | "text/plain"
    )
}

/// Liest den Body und bricht ab, sobald das Limit ueberschritten wird.
async fn read_body_limited(response: reqwest::Response, limit: usize) -> Result<Vec<u8>, WebError> {
    let mut stream = response.bytes_stream();
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(map_request_error)?;
        if body.len() + chunk.len() > limit {
            return Err(WebError::BodyTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        text.chars().take(max).collect()
    }
}

// --------------------------------------------------------------- Websuche --

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub content: String,
}

/// Endpunkt der SearXNG-Instanz: trimmen, abschliessendes `/` und `/search`
/// entfernen, dann `/search` anhaengen.
pub fn search_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim();
    let without_slash = trimmed.trim_end_matches('/');
    let without_search = without_slash
        .strip_suffix("/search")
        .unwrap_or(without_slash);
    format!("{without_search}/search")
}

/// Fragt die SearXNG-Instanz und liefert hoechstens 8 Treffer.
pub async fn web_search(base_url: &str, query: &str) -> Result<Vec<SearchResult>, WebError> {
    // Die SearXNG-URL ist Benutzerkonfiguration und darf auf Loopback zeigen.
    let mut url = Url::parse(&search_endpoint(base_url)).map_err(|_| WebError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(WebError::InvalidUrl);
    }
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("format", "json");

    let client = search_client(&url)?;
    let request_url = url.clone();
    let response = client.get(url).send().await.map_err(map_request_error)?;
    let status = response.status();
    if status.is_redirection() {
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                request_url
                    .join(value)
                    .map(|url| url.to_string())
                    .unwrap_or_else(|_| value.to_string())
            });
        return Err(match location {
            Some(location) => WebError::SearchRedirect(location),
            None => WebError::HttpStatus(status.as_u16()),
        });
    }
    if !status.is_success() {
        return Err(WebError::HttpStatus(status.as_u16()));
    }
    let body = read_body_limited(response, MAX_BODY_BYTES).await?;
    parse_search_results(&String::from_utf8_lossy(&body))
}

/// Client fuer die Suche: keine automatischen Weiterleitungen (die koennen auf
/// eine andere Maschine zeigen) und **immer** ohne System-Proxy - sonst ginge
/// die Suchanfrage samt Suchbegriff an den Proxy, sobald die konfigurierte
/// Instanz nicht als `localhost`/IP-Literal erkannt wird (z. B. `localhost.`,
/// /etc/hosts-Alias oder LAN-Name).
fn search_client(_url: &Url) -> Result<Client, WebError> {
    Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| WebError::Response(error.to_string()))
}

fn parse_search_results(body: &str) -> Result<Vec<SearchResult>, WebError> {
    let value: Value =
        serde_json::from_str(body).map_err(|error| WebError::Response(error.to_string()))?;
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(results
        .into_iter()
        .take(MAX_SEARCH_RESULTS)
        .map(|item| SearchResult {
            title: text_field(&item, "title"),
            url: text_field(&item, "url"),
            content: truncate_chars(&text_field(&item, "content"), MAX_SNIPPET_CHARS),
        })
        .collect())
}

fn text_field(item: &Value, key: &str) -> String {
    item.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

// ---------------------------------------------------------- Werkzeug-Lauf --

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
                            .map_err(|error| error.short_reason())?;
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

// --------------------------------------------------------------- Commands --

#[tauri::command]
pub fn web_search_config_get(paths: State<'_, AppPaths>) -> AppResult<WebSearchConfig> {
    Ok(config::load(&paths))
}

#[tauri::command]
pub fn web_search_config_set(
    paths: State<'_, AppPaths>,
    config: WebSearchConfig,
) -> AppResult<WebSearchConfig> {
    config::save(&paths, &config)
}

/// Eine Testsuche gegen die angegebene URL; liefert die Anzahl der Treffer.
#[tauri::command]
pub async fn web_search_test(url: String) -> AppResult<usize> {
    let endpoint = config::normalize_url(&url)?;
    if endpoint.is_empty() {
        return Err("Ungültige SearXNG-URL".to_string());
    }
    let results = web_search(&endpoint, "test")
        .await
        .map_err(test_error_text)?;
    Ok(results.len())
}

fn test_error_text(error: WebError) -> String {
    match error {
        WebError::HttpStatus(403) => {
            "SearXNG lehnt JSON ab – in settings.yml unter search.formats „json“ erlauben"
                .to_string()
        }
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------- Schemas --

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

#[cfg(test)]
mod tests;
