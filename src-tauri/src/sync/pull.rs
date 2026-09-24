//! Pull-Seite ohne HTTP: Staging der Seiten in `sync_inbox`
//! (docs/spec-sync.md, „Pull (Staging)“) und „Neu abgleichen“.

use rusqlite::{params, Connection, TransactionBehavior};
use sync_proto::PullResponse;

use super::{db_error, outbox, state_get, state_set};
use crate::storage::AppResult;

pub(crate) fn cursor(conn: &Connection) -> AppResult<i64> {
    Ok(state_get(conn, "cursor")?
        .and_then(|value| value.parse().ok())
        .unwrap_or(0))
}

/// Legt eine Pull-Seite in einer Schreibtransaktion in `sync_inbox` ab und
/// setzt `cursor = next`. Angewandt wird erst nach der letzten Seite.
pub(crate) fn stage_page(conn: &mut Connection, page: &PullResponse) -> AppResult<()> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    for server_state in &page.states {
        let (entity, key) = server_state.state.entity_key();
        let payload = serde_json::to_string(&server_state.state)
            .map_err(|err| format!("Sync-Zustand konnte nicht gespeichert werden: {err}"))?;
        tx.execute(
            "INSERT OR REPLACE INTO sync_inbox (seq, entity, key, payload) VALUES (?1, ?2, ?3, ?4)",
            params![server_state.seq, entity, key, payload],
        )
        .map_err(db_error)?;
    }
    state_set(&tx, "cursor", &page.next.to_string())?;
    tx.commit().map_err(db_error)
}

/// „Neu abgleichen“ nach einem Datensatzwechsel, eine Transaktion: Inbox
/// leeren, `cursor = 0`, Outbox-Einträge für alle Zeilen nicht privater
/// Videos und alle Sammlungen (bestehende Löschwünsche bleiben), neue
/// Datensatz-ID. `published` bleibt unverändert.
pub(crate) fn rebaseline(conn: &mut Connection, dataset_id: &str) -> AppResult<()> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    tx.execute("DELETE FROM sync_inbox", []).map_err(db_error)?;
    state_set(&tx, "cursor", "0")?;
    outbox::seed(&tx, None)?;
    state_set(&tx, "dataset_id", dataset_id)?;
    tx.commit().map_err(db_error)
}
