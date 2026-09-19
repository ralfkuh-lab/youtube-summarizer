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
mod html;

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::redirect::Policy;
use reqwest::{Client, Url};
use serde_json::{json, Value};

use address::{check_fetch_url, FilteredResolver};

pub use address::is_blocked_ip;
pub use html::html_to_text;

/// Zeitlimit fuer Suchanfragen und Seitenabrufe.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Hoechstens so viele Weiterleitungen werden verfolgt.
const MAX_REDIRECTS: usize = 5;
/// Groesste akzeptierte Antwortgroesse (Bytes).
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_SEARCH_RESULTS: usize = 8;
const MAX_SNIPPET_CHARS: usize = 300;
const MAX_PAGE_CHARS: usize = 12_000;
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
    /// Sonstiger Fehler beim Lesen der Antwort.
    Response(String),
}

impl WebError {
    /// Text, den das Modell als Tool-Ergebnis sieht.
    pub fn model_message(&self) -> String {
        match self {
            WebError::AddressNotAllowed | WebError::NoAllowedAddress => {
                "Fehler: Adresse nicht erlaubt".to_string()
            }
            other => format!("Fehler: {other}"),
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
            WebError::Response(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for WebError {}

// ------------------------------------------------------------ Seitenabruf --

/// Laedt eine oeffentliche Seite und liefert den extrahierten Text (hoechstens
/// 12 000 Unicode-Skalarwerte).
pub async fn fetch_page(url: &str) -> Result<String, WebError> {
    fetch_page_with(url, Arc::new(is_blocked_ip)).await
}

/// Interne Fassung mit injizierbarem Adresspraedikat: die Produktion uebergibt
/// fest `is_blocked_ip`, nur Tests duerfen den lokalen Testserver zulassen.
async fn fetch_page_with(
    url: &str,
    blocked: Arc<dyn Fn(IpAddr) -> bool + Send + Sync>,
) -> Result<String, WebError> {
    let client = Client::builder()
        .redirect(Policy::none())
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
                .ok_or(WebError::InvalidUrl)?;
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
            .unwrap_or_default()
            .to_string();
        let media_type = media_type_of(&content_type);
        if !is_allowed_media_type(&media_type) {
            return Err(WebError::UnsupportedContentType(media_type));
        }
        let body = read_body_limited(response, MAX_BODY_BYTES).await?;
        let raw = String::from_utf8_lossy(&body).to_string();
        let text = if media_type == "text/plain" {
            raw
        } else {
            html_to_text(&raw)
        };
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

    let client = Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| WebError::Response(error.to_string()))?;
    let response = client.get(url).send().await.map_err(map_request_error)?;
    let status = response.status();
    if !status.is_success() {
        return Err(WebError::HttpStatus(status.as_u16()));
    }
    let body = read_body_limited(response, MAX_BODY_BYTES).await?;
    parse_search_results(&String::from_utf8_lossy(&body))
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
