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
mod fetch;
mod html;
mod search;
mod tools;

pub use search::web_search;
pub(crate) use tools::string_argument;
pub use tools::{
    production_runtime, tool_definitions, tool_kind, tool_label, ToolEvent, ToolRuntime,
};

use std::time::Duration;

use tauri::State;

use crate::storage::{AppPaths, AppResult};
use config::WebSearchConfig;

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
/// Feste Meldungen der Schleife, wenn ein Aufruf nicht ausgefuehrt werden kann.
pub const TOOL_LIMIT_MESSAGE: &str = "Tool-Limit pro Runde erreicht";
pub const UNKNOWN_TOOL_MESSAGE: &str = "unbekanntes Tool";
pub const INVALID_ARGUMENTS_MESSAGE: &str = "ungültige Tool-Argumente";

/// Kuerzt auf Unicode-Skalarwerte.
pub(crate) fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        text.chars().take(max).collect()
    }
}

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

    /// Kurze Ursache ohne Praefix. Enthaelt **keinen** vom Zielserver
    /// gesteuerten Text (kein Content-Type, kein Location-Header), damit sich
    /// keine Delimiter in den Modellkontext einschleusen lassen - nur feste
    /// Saetze und hoechstens eine Zahl.
    pub fn short_reason(&self) -> String {
        match self {
            WebError::AddressNotAllowed | WebError::NoAllowedAddress => "Adresse nicht erlaubt",
            WebError::InvalidUrl => "ungültige URL",
            WebError::Timeout => "Zeitüberschreitung",
            WebError::HttpStatus(status) => return format!("HTTP-Status {status}"),
            WebError::UnsupportedContentType(_) => "nicht unterstützter Content-Type",
            WebError::MissingContentType => "Antwort ohne Content-Type",
            WebError::BodyTooLarge => "Antwort zu groß",
            WebError::EmptyText => "kein Text extrahiert",
            WebError::TooManyRedirects => "zu viele Weiterleitungen",
            WebError::SearchRedirect(_) => "Suchinstanz leitet weiter",
            WebError::Response(_) => "Abruf fehlgeschlagen",
        }
        .to_string()
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

#[cfg(test)]
mod tests;
