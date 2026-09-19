//! Seitenabruf: Adresspruefung, manuelle Redirects, Groessen-/Typgrenzen und
//! HTML-Extraktion.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::redirect::Policy;
use reqwest::{Client, Url};

use super::address::{check_fetch_url, is_blocked_ip, FilteredResolver};
use super::html::html_to_text;
use super::{
    truncate_chars, WebError, FETCH_BUDGET, MAX_BODY_BYTES, MAX_PAGE_CHARS, MAX_REDIRECTS,
    REQUEST_TIMEOUT, USER_AGENT,
};

/// Laedt eine oeffentliche Seite und liefert den extrahierten Text (hoechstens
/// 12 000 Unicode-Skalarwerte).
pub async fn fetch_page(url: &str) -> Result<String, WebError> {
    fetch_page_with(url, Arc::new(is_blocked_ip), FETCH_BUDGET).await
}

/// Interne Fassung mit injizierbarem Adresspraedikat: die Produktion uebergibt
/// fest `is_blocked_ip`, nur Tests duerfen den lokalen Testserver zulassen.
pub(crate) async fn fetch_page_with(
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

pub(crate) fn map_request_error(error: reqwest::Error) -> WebError {
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
pub(crate) fn media_type_of(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

pub(crate) fn is_allowed_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "text/html" | "application/xhtml+xml" | "text/plain"
    )
}

/// Liest den Body und bricht ab, sobald das Limit ueberschritten wird.
pub(crate) async fn read_body_limited(
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, WebError> {
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
