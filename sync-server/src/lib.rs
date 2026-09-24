//! Sync-Server der YouTube-Summarizer-App. Verbindlich ist `docs/spec-sync.md`
//! (Abschnitte „Server-Regeln“ und „Betrieb des Servers“).

pub mod db;
mod http;
pub mod merge;

pub use http::{app, serve};

use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// Strukturfehler in der Operation mit diesem Index (HTTP 400).
    Invalid(usize, String),
    /// Anfrage ohne auswertbaren Inhalt, z. B. kein JSON (HTTP 400).
    Malformed(String),
    /// Token fehlt, ist unbekannt oder widerrufen (HTTP 401).
    Unauthorized,
    /// `X-Sync-Protocol` fehlt oder passt nicht (HTTP 426).
    Protocol,
    /// Push-Body über `MAX_PUSH_BODY_BYTES` (HTTP 413).
    TooLarge,
    /// `X-Sync-Dataset` passt nicht; enthält die aktuelle Datensatz-ID (HTTP 409).
    DatasetMismatch(String),
    /// `since` liegt über dem Server-Zähler (HTTP 409).
    CursorAhead,
    Db(rusqlite::Error),
    Io(std::io::Error),
    /// Gespeicherte Daten verletzen eine Invariante.
    Corrupt(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Invalid(index, message) => write!(f, "Operation {index}: {message}"),
            Error::Malformed(message) => write!(f, "ungültige Anfrage: {message}"),
            Error::Unauthorized => write!(f, "nicht autorisiert"),
            Error::Protocol => write!(f, "Protokollversion passt nicht"),
            Error::TooLarge => write!(f, "Anfrage zu groß"),
            Error::DatasetMismatch(id) => write!(f, "Datensatz-ID passt nicht (aktuell {id})"),
            Error::CursorAhead => write!(f, "since liegt über dem Server-Zähler"),
            Error::Db(err) => write!(f, "Datenbank: {err}"),
            Error::Io(err) => write!(f, "Datei: {err}"),
            Error::Corrupt(message) => write!(f, "inkonsistente Daten: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(err: rusqlite::Error) -> Self {
        Error::Db(err)
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Corrupt(err.to_string())
    }
}

#[cfg(test)]
mod tests;
