//! Outbox-Einträge, die nicht von Triggern stammen: Grabsteine beim
//! Privatschalten und das Seeding (Migration, Freigeben, Neu abgleichen).

use rusqlite::{params, Connection};

use super::{db_error, state_get, state_set, CANONICAL_TIME_FORMAT};
use crate::storage::AppResult;

/// Schreibt einen Eintrag: alten Eintrag gleichen Schlüssels löschen, neu
/// einfügen (neue `seq`, `unsendable` zurückgesetzt).
pub(crate) fn put(
    conn: &Connection,
    entity: &str,
    key: &str,
    owner: Option<&str>,
    parent: Option<&str>,
    tomb: Option<&str>,
) -> AppResult<()> {
    conn.execute(
        "DELETE FROM sync_outbox WHERE entity = ?1 AND key = ?2",
        params![entity, key],
    )
    .map_err(db_error)?;
    conn.execute(
        &format!(
            "INSERT INTO sync_outbox (entity, key, owner, parent, tomb, changed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, strftime('{CANONICAL_TIME_FORMAT}', 'now'))"
        ),
        params![entity, key, owner, parent, tomb],
    )
    .map_err(db_error)?;
    Ok(())
}

/// Einmaliges Seeding nach der ersten Migration (`sync_state.seeded`).
pub(crate) fn seed_once(conn: &Connection) -> AppResult<()> {
    if state_get(conn, "seeded")?.is_some() {
        return Ok(());
    }
    seed(conn, None)?;
    state_set(conn, "seeded", "1")
}

/// Legt Einträge für alle Zeilen nicht privater Videos und alle Sammlungen an
/// (`video_id = None`) bzw. für ein Video mit allen Kindern. Ausstehende
/// Löschwünsche (Einträge ohne Zeile) bleiben unberührt. `changed_at` ist der
/// letzte Änderungszeitpunkt der Zeile, damit LWW-Felder ihr wahres Alter
/// behalten.
pub(crate) fn seed(conn: &Connection, video_id: Option<i64>) -> AppResult<()> {
    const LIVE: &str = "v.local_only = 0 AND (?1 IS NULL OR v.id = ?1)";
    let sources = [
        (
            "video",
            format!("SELECT v.uid, v.uid, NULL, v.updated_at FROM videos v WHERE {LIVE}"),
        ),
        (
            "summary",
            format!(
                "SELECT s.uid, v.uid, NULL, s.created_at \
                 FROM summaries s JOIN videos v ON v.id = s.video_id WHERE {LIVE}"
            ),
        ),
        (
            "chat",
            format!(
                "SELECT c.uid, v.uid, NULL, c.updated_at \
                 FROM chats c JOIN videos v ON v.id = c.video_id WHERE {LIVE}"
            ),
        ),
        (
            "round",
            format!(
                "SELECT m.round_uid, v.uid, c.uid, MIN(m.created_at) \
                 FROM chat_messages m JOIN chats c ON c.id = m.chat_id \
                 JOIN videos v ON v.id = c.video_id WHERE {LIVE} GROUP BY m.round_uid"
            ),
        ),
        (
            "membership",
            format!(
                "SELECT v.uid || '/' || k.uid, v.uid, k.uid, vc.created_at \
                 FROM video_collections vc JOIN videos v ON v.id = vc.video_id \
                 JOIN collections k ON k.id = vc.collection_id WHERE {LIVE}"
            ),
        ),
        (
            "collection",
            "SELECT uid, NULL, NULL, updated_at FROM collections WHERE ?1 IS NULL".to_string(),
        ),
    ];
    for (entity, source) in sources {
        let rows = format!("WITH rows(key, owner, parent, changed_at) AS ({source})");
        conn.execute(
            &format!(
                "{rows} DELETE FROM sync_outbox \
                 WHERE entity = ?2 AND key IN (SELECT key FROM rows)"
            ),
            params![video_id, entity],
        )
        .map_err(db_error)?;
        conn.execute(
            &format!(
                "{rows} INSERT INTO sync_outbox (entity, key, owner, parent, changed_at) \
                 SELECT ?2, key, owner, parent, COALESCE( \
                     strftime('{CANONICAL_TIME_FORMAT}', changed_at), \
                     strftime('{CANONICAL_TIME_FORMAT}', 'now')) \
                 FROM rows"
            ),
            params![video_id, entity],
        )
        .map_err(db_error)?;
    }
    Ok(())
}
