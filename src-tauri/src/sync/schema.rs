//! Sync-Tabellen und Outbox-Trigger (docs/spec-sync.md, „Sync-Tabellen“ und
//! „Trigger“).

use rusqlite::Connection;

use super::{db_error, state_get, state_set};
use crate::storage::AppResult;

/// Version der Trigger in `sync_state.schema`. Bei jeder Änderung an
/// [`triggers_sql`] erhöhen: die Migration löscht dann alle `sync_*`-Trigger
/// und legt sie neu an.
const TRIGGER_SCHEMA: &str = "1";

const TABLES: &str = r#"
CREATE TABLE IF NOT EXISTS sync_outbox (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    entity     TEXT NOT NULL,
    key        TEXT NOT NULL,
    owner      TEXT,
    parent     TEXT,
    tomb       TEXT,
    changed_at TEXT NOT NULL,
    unsendable TEXT,
    UNIQUE(entity, key)
);
CREATE INDEX IF NOT EXISTS idx_sync_outbox_owner ON sync_outbox(owner);
CREATE INDEX IF NOT EXISTS idx_sync_outbox_parent ON sync_outbox(parent);
CREATE TABLE IF NOT EXISTS sync_state (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE IF NOT EXISTS sync_inbox (seq INTEGER PRIMARY KEY, entity TEXT, key TEXT, payload TEXT);
"#;

/// Legt die Sync-Tabellen an und bringt die Trigger auf [`TRIGGER_SCHEMA`].
pub(crate) fn install(conn: &Connection) -> AppResult<()> {
    conn.execute_batch(TABLES).map_err(db_error)?;
    if state_get(conn, "schema")?.as_deref() == Some(TRIGGER_SCHEMA) {
        return Ok(());
    }
    let names = {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger' AND name LIKE 'sync!_%' ESCAPE '!'")
            .map_err(db_error)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?
    };
    for name in names {
        conn.execute_batch(&format!("DROP TRIGGER \"{name}\";"))
            .map_err(db_error)?;
    }
    conn.execute_batch(&triggers_sql()).map_err(db_error)?;
    state_set(conn, "schema", TRIGGER_SCHEMA)
}

/// Outbox-Schreiben = DELETE des alten Eintrags + INSERT (neue `seq`). Kein
/// `INSERT OR REPLACE`: im Trigger würde die Konfliktbehandlung des äußeren
/// Statements gelten.
fn triggers_sql() -> String {
    let active = "NOT EXISTS (SELECT 1 FROM sync_state WHERE key = 'applying' AND value = '1')";
    let now = format!("strftime('{}', 'now')", super::CANONICAL_TIME_FORMAT);
    let mut sql = String::new();

    for (table, column) in [
        ("videos", "uid"),
        ("summaries", "uid"),
        ("chats", "uid"),
        ("collections", "uid"),
        ("chat_messages", "round_uid"),
    ] {
        sql.push_str(&format!(
            "CREATE TRIGGER sync_{table}_uid BEFORE INSERT ON {table} WHEN NEW.{column} IS NULL
             BEGIN SELECT RAISE(ABORT, '{table}.{column} fehlt'); END;\n"
        ));
    }

    let put_video = format!(
        "DELETE FROM sync_outbox WHERE entity = 'video' AND key = NEW.uid;
         INSERT INTO sync_outbox (entity, key, owner, changed_at)
             VALUES ('video', NEW.uid, NEW.uid, {now});"
    );
    sql.push_str(&format!(
        r#"
CREATE TRIGGER sync_videos_insert AFTER INSERT ON videos
WHEN NEW.local_only = 0 AND {active}
BEGIN {put_video} END;

CREATE TRIGGER sync_videos_update
AFTER UPDATE OF url, title, thumbnail_url, thumbnail_data, transcript, chapters,
    published_at, description, transcript_error ON videos
WHEN NEW.local_only = 0 AND {active}
BEGIN {put_video} END;

CREATE TRIGGER sync_videos_delete BEFORE DELETE ON videos
WHEN {active}
BEGIN
    DELETE FROM sync_outbox WHERE owner = OLD.uid;
    INSERT INTO sync_outbox (entity, key, owner, tomb, changed_at)
        SELECT 'video', OLD.uid, OLD.uid, 'deleted', {now} WHERE OLD.published = 1;
END;
"#
    ));

    // Kinder mit Video-Owner: Die Trigger schweigen, wenn das Video privat ist
    // oder (bei Kaskaden) schon fehlt.
    for (name, table, entity, row, timing) in [
        (
            "sync_summaries_insert",
            "summaries",
            "summary",
            "NEW",
            "AFTER INSERT",
        ),
        (
            "sync_summaries_delete",
            "summaries",
            "summary",
            "OLD",
            "AFTER DELETE",
        ),
        ("sync_chats_insert", "chats", "chat", "NEW", "AFTER INSERT"),
        ("sync_chats_update", "chats", "chat", "NEW", "AFTER UPDATE"),
        ("sync_chats_delete", "chats", "chat", "OLD", "BEFORE DELETE"),
    ] {
        let clear_rounds = if timing == "BEFORE DELETE" {
            format!("DELETE FROM sync_outbox WHERE entity = 'round' AND parent = {row}.uid;")
        } else {
            String::new()
        };
        sql.push_str(&format!(
            r#"
CREATE TRIGGER {name} {timing} ON {table}
WHEN {active} AND EXISTS (SELECT 1 FROM videos WHERE id = {row}.video_id AND local_only = 0)
BEGIN
    {clear_rounds}
    DELETE FROM sync_outbox WHERE entity = '{entity}' AND key = {row}.uid;
    INSERT INTO sync_outbox (entity, key, owner, changed_at)
        SELECT '{entity}', {row}.uid, uid, {now} FROM videos WHERE id = {row}.video_id;
END;
"#
        ));
    }

    sql.push_str(&format!(
        r#"
CREATE TRIGGER sync_chat_messages_insert AFTER INSERT ON chat_messages
WHEN {active} AND EXISTS (
    SELECT 1 FROM chats c JOIN videos v ON v.id = c.video_id
    WHERE c.id = NEW.chat_id AND v.local_only = 0)
BEGIN
    DELETE FROM sync_outbox WHERE entity = 'round' AND key = NEW.round_uid;
    INSERT INTO sync_outbox (entity, key, owner, parent, changed_at)
        SELECT 'round', NEW.round_uid, v.uid, c.uid, {now}
        FROM chats c JOIN videos v ON v.id = c.video_id WHERE c.id = NEW.chat_id;
END;
"#
    ));

    for (name, row, timing) in [
        ("sync_collections_insert", "NEW", "AFTER INSERT"),
        ("sync_collections_update", "NEW", "AFTER UPDATE"),
        ("sync_collections_delete", "OLD", "BEFORE DELETE"),
    ] {
        let clear_memberships = if timing == "BEFORE DELETE" {
            format!("DELETE FROM sync_outbox WHERE entity = 'membership' AND parent = {row}.uid;")
        } else {
            String::new()
        };
        sql.push_str(&format!(
            r#"
CREATE TRIGGER {name} {timing} ON collections
WHEN {active}
BEGIN
    {clear_memberships}
    DELETE FROM sync_outbox WHERE entity = 'collection' AND key = {row}.uid;
    INSERT INTO sync_outbox (entity, key, changed_at) VALUES ('collection', {row}.uid, {now});
END;
"#
        ));
    }

    for (name, row, timing) in [
        ("sync_memberships_insert", "NEW", "AFTER INSERT"),
        ("sync_memberships_delete", "OLD", "AFTER DELETE"),
    ] {
        sql.push_str(&format!(
            r#"
CREATE TRIGGER {name} {timing} ON video_collections
WHEN {active}
    AND EXISTS (SELECT 1 FROM videos WHERE id = {row}.video_id AND local_only = 0)
    AND EXISTS (SELECT 1 FROM collections WHERE id = {row}.collection_id)
BEGIN
    DELETE FROM sync_outbox WHERE entity = 'membership' AND key = (
        SELECT v.uid || '/' || c.uid FROM videos v, collections c
        WHERE v.id = {row}.video_id AND c.id = {row}.collection_id);
    INSERT INTO sync_outbox (entity, key, owner, parent, changed_at)
        SELECT 'membership', v.uid || '/' || c.uid, v.uid, c.uid, {now}
        FROM videos v, collections c
        WHERE v.id = {row}.video_id AND c.id = {row}.collection_id;
END;
"#
        ));
    }
    sql
}
