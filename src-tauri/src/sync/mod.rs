//! Client-Seite des Syncs (docs/spec-sync.md, „Client-Datenmodell“ und
//! „Client-Sync-Engine“). Alles hier arbeitet synchron auf einer
//! `rusqlite::Connection`; HTTP, Befehle und UI folgen in späteren Etappen.

pub(crate) mod apply;
pub(crate) mod client;
pub(crate) mod commands;
pub(crate) mod config;
pub(crate) mod engine;
pub(crate) mod outbox;
pub(crate) mod privatize;
pub(crate) mod pull;
pub(crate) mod schema;
pub(crate) mod snapshot;

use rusqlite::{params, Connection, OptionalExtension};

use crate::storage::AppResult;

/// SQL-Ausdruck für einen kanonischen UTC-Zeitpunkt (`…mmmZ`).
pub(crate) const CANONICAL_TIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%fZ";

pub(crate) const ABORTED: &str = "Abgebrochen nach Schritt";

/// Zählt Zwischenstände einer Transaktion (Migration, Apply); Tests brechen
/// mit `abort_after` nach dem n-ten ab (Referenzfälle C19, C21).
pub(crate) struct Checkpoints {
    next: usize,
    abort_after: Option<usize>,
}

impl Checkpoints {
    pub(crate) fn new(abort_after: Option<usize>) -> Self {
        Self {
            next: 0,
            abort_after,
        }
    }

    pub(crate) fn pass(&mut self) -> AppResult<()> {
        let index = self.next;
        self.next += 1;
        if self.abort_after == Some(index) {
            return Err(format!("{ABORTED} {index}"));
        }
        Ok(())
    }
}

pub(crate) fn db_error(err: rusqlite::Error) -> String {
    format!("Sync-Datenbankfehler: {err}")
}

/// `n` neue zufällige uids (32 Hex-Zeichen), aufsteigend sortiert: Wer sie in
/// bestehender Reihenfolge vergibt, erhält diese Reihenfolge.
pub(crate) fn new_uids(conn: &Connection, n: usize) -> AppResult<Vec<String>> {
    if n == 0 {
        return Ok(Vec::new());
    }
    let hex: String = conn
        .query_row(
            "SELECT lower(hex(randomblob(?1)))",
            params![(n * 16) as i64],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    let mut uids: Vec<String> = (0..n)
        .map(|index| hex[index * 32..(index + 1) * 32].to_string())
        .collect();
    uids.sort();
    Ok(uids)
}

pub(crate) fn new_uid(conn: &Connection) -> AppResult<String> {
    Ok(new_uids(conn, 1)?.remove(0))
}

pub(crate) fn state_get(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    conn.query_row(
        "SELECT value FROM sync_state WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(db_error)
}

pub(crate) fn state_set(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO sync_state (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(db_error)?;
    Ok(())
}

#[cfg(test)]
mod tests;
