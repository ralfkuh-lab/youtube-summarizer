//! SearXNG-Suche: Endpunkt-Normalisierung, Trefferliste und Client (ohne
//! System-Proxy, ohne automatische Weiterleitungen).

use reqwest::redirect::Policy;
use reqwest::{Client, Url};
use serde_json::Value;

use super::fetch::{map_request_error, read_body_limited};
use super::{
    truncate_chars, WebError, MAX_BODY_BYTES, MAX_SEARCH_RESULTS, MAX_SNIPPET_CHARS,
    REQUEST_TIMEOUT, USER_AGENT,
};

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
