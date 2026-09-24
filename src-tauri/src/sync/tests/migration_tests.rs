//! Migration einer Alt-DB: C1, C19 (Migrationsteil), C21, C22.

use rusqlite::{params, Connection};
use tempfile::TempDir;

use super::{dump, open, strings, text};
use crate::storage::{self, AppPaths};
use crate::sync::ABORTED;

/// Schema vor dem Sync (Stand von `init_db` bis Revision 4.1), mit
/// `UNIQUE(video_id)` und ohne uids.
const LEGACY_SCHEMA: &str = r#"
    CREATE TABLE videos (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        video_id TEXT NOT NULL UNIQUE,
        url TEXT NOT NULL,
        title TEXT NOT NULL,
        thumbnail_url TEXT NOT NULL,
        thumbnail_data BLOB,
        transcript TEXT,
        chapters TEXT,
        summary TEXT,
        summary_provider TEXT,
        summary_model TEXT,
        published_at TEXT,
        description TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        transcript_error TEXT
    );
    CREATE TABLE collections (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL UNIQUE,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE UNIQUE INDEX idx_collections_name_nocase ON collections(name COLLATE NOCASE);
    CREATE TABLE video_collections (
        video_id INTEGER NOT NULL,
        collection_id INTEGER NOT NULL,
        created_at TEXT NOT NULL,
        PRIMARY KEY (video_id, collection_id),
        FOREIGN KEY (video_id) REFERENCES videos(id) ON DELETE CASCADE,
        FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE CASCADE
    );
    CREATE INDEX idx_video_collections_video_id ON video_collections(video_id);
    CREATE INDEX idx_video_collections_collection_id ON video_collections(collection_id);
    CREATE TABLE summaries (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        video_id INTEGER NOT NULL,
        created_at TEXT NOT NULL,
        summary TEXT NOT NULL,
        provider TEXT,
        model TEXT,
        options TEXT,
        FOREIGN KEY (video_id) REFERENCES videos(id) ON DELETE CASCADE
    );
    CREATE INDEX idx_summaries_video_id ON summaries(video_id);
    CREATE TABLE chats (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        video_id INTEGER NOT NULL,
        title TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        context_options TEXT,
        FOREIGN KEY (video_id) REFERENCES videos(id) ON DELETE CASCADE
    );
    CREATE TABLE chat_messages (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        chat_id INTEGER NOT NULL,
        role TEXT NOT NULL,
        content TEXT NOT NULL,
        tool_calls TEXT,
        tool_call_id TEXT,
        provider TEXT,
        model TEXT,
        created_at TEXT NOT NULL,
        FOREIGN KEY (chat_id) REFERENCES chats(id) ON DELETE CASCADE
    );
    CREATE INDEX idx_chat_messages_chat ON chat_messages(chat_id, id);
    CREATE INDEX idx_chats_video ON chats(video_id, updated_at);
"#;

/// Alt-DB: zwei Videos (das dritte gelöscht, Autoincrement-Stand 3), drei
/// Summaries an Video 1 (zwei mit gleichem Zeitstempel), eine Legacy-Summary
/// nur in `videos.summary` an Video 2, zwei Chats mit vier Runden (B und C
/// fallen nach dem Kanonisieren auf dieselbe Millisekunde), zwei Sammlungen,
/// drei Zuordnungen. Zeitstempel im alten Format `to_rfc3339()`.
const LEGACY_DATA: &str = r#"
    INSERT INTO videos (id, video_id, url, title, thumbnail_url, thumbnail_data, transcript,
        summary, summary_provider, summary_model, published_at, description, created_at, updated_at)
    VALUES
        (1, 'vidAAAAAAA1', 'https://youtu.be/vidAAAAAAA1', 'Video eins', 'https://t/1.jpg',
         x'FFD8FF00', '[{"text":"hi","start":0.0,"time":"0:00"}]', 'S3', 'P', 'M', '2026-01-02',
         'Beschreibung', '2026-09-20T10:00:00.123456789+00:00', '2026-09-20T10:06:00.250000000+00:00'),
        (2, 'vidBBBBBBB2', 'https://youtu.be/vidBBBBBBB2', 'Video zwei', 'https://t/2.jpg',
         NULL, NULL, 'Alt', 'P', 'M', NULL, NULL,
         '2026-09-20T12:00:00.5+02:00', '2026-09-21T08:00:00Z'),
        (3, 'vidCCCCCCC3', 'https://youtu.be/vidCCCCCCC3', 'Video drei', 'https://t/3.jpg',
         NULL, NULL, NULL, NULL, NULL, NULL, NULL,
         '2026-09-22T08:00:00Z', '2026-09-22T08:00:00Z');
    DELETE FROM videos WHERE id = 3;

    INSERT INTO summaries (id, video_id, created_at, summary, provider, model, options) VALUES
        (1, 1, '2026-09-20T10:05:00.000000001+00:00', 'S1', 'P', 'M', '{"preset":"standard"}'),
        (2, 1, '2026-09-20T10:06:00.250000000+00:00', 'S2', 'P', 'M', NULL),
        (3, 1, '2026-09-20T10:06:00.250000000+00:00', 'S3', 'P', 'M', NULL);

    INSERT INTO chats (id, video_id, title, created_at, updated_at, context_options) VALUES
        (1, 1, 'Chat eins', '2026-09-20T11:00:00.100000000+00:00',
         '2026-09-20T11:05:00.000400000+00:00', '{"transcript":true,"summaryIds":[1]}'),
        (2, 2, 'Chat zwei', '2026-09-21T09:00:00Z', '2026-09-21T09:00:00Z', NULL);

    INSERT INTO chat_messages (id, chat_id, role, content, tool_calls, tool_call_id, created_at) VALUES
        (1, 1, 'user', 'Frage A', NULL, NULL, '2026-09-20T11:00:00.100000000+00:00'),
        (2, 1, 'assistant', 'Antwort A', NULL, NULL, '2026-09-20T11:00:00.100000000+00:00'),
        (3, 1, 'user', 'Frage B', NULL, NULL, '2026-09-20T11:05:00.000100000+00:00'),
        (4, 1, 'assistant', '', '[{"id":"1"}]', NULL, '2026-09-20T11:05:00.000100000+00:00'),
        (5, 1, 'tool', 'Ergebnis', NULL, '1', '2026-09-20T11:05:00.000100000+00:00'),
        (6, 1, 'user', 'Frage C', NULL, NULL, '2026-09-20T11:05:00.000400000+00:00'),
        (7, 2, 'user', 'Frage D', NULL, NULL, '2026-09-21T09:00:00Z');

    INSERT INTO collections (id, name, created_at, updated_at) VALUES
        (1, 'KI', '2026-09-01T00:00:00+00:00', '2026-09-01T00:00:00+00:00'),
        (2, 'Musik', '2026-09-02T00:00:00+00:00', '2026-09-03T00:00:00+00:00');

    INSERT INTO video_collections (video_id, collection_id, created_at) VALUES
        (1, 1, '2026-09-05T00:00:00+00:00'),
        (2, 1, '2026-09-05T00:00:00+00:00'),
        (1, 2, '2026-09-06T00:00:00+00:00');
"#;

fn legacy_db() -> (TempDir, AppPaths) {
    let temp = TempDir::new().unwrap();
    let paths = AppPaths {
        db_path: temp.path().join("videos.db"),
        config_path: temp.path().join("config.json"),
    };
    let conn = Connection::open(&paths.db_path).unwrap();
    conn.execute_batch(LEGACY_SCHEMA).unwrap();
    conn.execute_batch(LEGACY_DATA).unwrap();
    (temp, paths)
}

/// Alle Nicht-Zeitstempel-Spalten der Alt-Tabellen, nach Schlüssel sortiert:
/// Inhalte, Integer-ids und Referenzen.
fn legacy_content(conn: &Connection) -> Vec<String> {
    let queries = [
        "SELECT id, video_id, url, title, thumbnail_url, thumbnail_data, transcript, chapters, \
         summary, summary_provider, summary_model, published_at, description, transcript_error \
         FROM videos ORDER BY id",
        "SELECT id, video_id, summary, provider, model, options FROM summaries ORDER BY id",
        "SELECT id, video_id, title, context_options FROM chats ORDER BY id",
        "SELECT id, chat_id, role, content, tool_calls, tool_call_id, provider, model \
         FROM chat_messages ORDER BY id",
        "SELECT id, name FROM collections ORDER BY id",
        "SELECT video_id, collection_id FROM video_collections ORDER BY video_id, collection_id",
    ];
    let mut lines = Vec::new();
    for sql in queries {
        let mut stmt = conn.prepare(sql).unwrap();
        let columns = stmt.column_count();
        let rows = stmt
            .query_map([], |row| {
                (0..columns)
                    .map(|index| row.get::<_, rusqlite::types::Value>(index))
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap();
        for row in rows {
            lines.push(format!("{sql}: {:?}", row.unwrap()));
        }
    }
    lines
}

fn videos_sequence(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT seq FROM sqlite_sequence WHERE name = 'videos'",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}

fn round_of(conn: &Connection, message_id: i64) -> (String, i64) {
    conn.query_row(
        "SELECT round_uid, position FROM chat_messages WHERE id = ?1",
        params![message_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

/// Prüft den migrierten Endstand der Alt-DB (gemeinsam für C1 und C19).
fn assert_migrated(paths: &AppPaths) {
    let conn = open(paths);
    // Legacy-Summary von Video 2 ist nachgezogen.
    for (table, expected) in [
        ("videos", 2),
        ("summaries", 4),
        ("chats", 2),
        ("chat_messages", 7),
        ("collections", 2),
        ("video_collections", 3),
    ] {
        assert_eq!(count(&conn, table), expected, "{table}");
    }

    for (table, column) in [
        ("videos", "uid"),
        ("summaries", "uid"),
        ("chats", "uid"),
        ("collections", "uid"),
        ("chat_messages", "round_uid"),
    ] {
        let invalid: i64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM {table} WHERE {column} IS NULL \
                     OR length({column}) != 32 OR {column} GLOB '*[^0-9a-f]*'"
                ),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(invalid, 0, "{table}.{column}");
    }

    // Runden: A = {1, 2}, B = {3, 4, 5}, C = {6} (trotz gleicher kanonischer
    // Millisekunde wie B), D = {7}; position nach id.
    let rounds: Vec<(String, i64)> = (1..=7).map(|id| round_of(&conn, id)).collect();
    let positions: Vec<i64> = rounds.iter().map(|(_, position)| *position).collect();
    assert_eq!(positions, vec![0, 1, 0, 1, 2, 0, 0]);
    assert_eq!(rounds[0].0, rounds[1].0);
    assert_eq!(rounds[2].0, rounds[3].0);
    assert_eq!(rounds[2].0, rounds[4].0);
    let distinct: std::collections::BTreeSet<&String> = rounds.iter().map(|(uid, _)| uid).collect();
    assert_eq!(distinct.len(), 4);

    // Reihenfolgen wie vorher nach id.
    let message_ids: Vec<i64> = storage::get_chat_messages(paths, 1)
        .unwrap()
        .iter()
        .map(|message| message.id)
        .collect();
    assert_eq!(message_ids, vec![1, 2, 3, 4, 5, 6]);
    let summary_ids: Vec<i64> = storage::get_summaries(paths, 1)
        .unwrap()
        .iter()
        .map(|summary| summary.id)
        .collect();
    assert_eq!(summary_ids, vec![3, 2, 1]);

    // Outbox je Zeile: 2 Videos, 4 Summaries, 2 Chats, 4 Runden, 3 Zuordnungen,
    // 2 Sammlungen.
    let mut expected: Vec<(String, String)> = Vec::new();
    for (entity, sql) in [
        ("video", "SELECT uid FROM videos"),
        ("summary", "SELECT uid FROM summaries"),
        ("chat", "SELECT uid FROM chats"),
        ("round", "SELECT DISTINCT round_uid FROM chat_messages"),
        (
            "membership",
            "SELECT v.uid || '/' || c.uid FROM video_collections vc \
             JOIN videos v ON v.id = vc.video_id JOIN collections c ON c.id = vc.collection_id",
        ),
        ("collection", "SELECT uid FROM collections"),
    ] {
        let mut stmt = conn.prepare(sql).unwrap();
        for key in stmt.query_map([], |row| row.get::<_, String>(0)).unwrap() {
            expected.push((entity.to_string(), key.unwrap()));
        }
    }
    expected.sort();
    assert_eq!(expected.len(), 17);
    assert_eq!(super::outbox_keys(&conn), expected);
    let non_canonical: Vec<String> = {
        let mut stmt = conn.prepare("SELECT changed_at FROM sync_outbox").unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        rows.map(Result::unwrap)
            .filter(|value| !sync_proto::is_canonical_time(value))
            .collect()
    };
    assert!(non_canonical.is_empty(), "{non_canonical:?}");
}

#[test]
fn c1_migrates_legacy_db_once() {
    let (_temp, paths) = legacy_db();
    storage::init_db(&paths).unwrap();
    assert_migrated(&paths);

    let conn = open(&paths);
    let schema: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'videos'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!schema.contains("UNIQUE"), "{schema}");
    let video = storage::get_video(&paths, 1).unwrap().unwrap();
    assert_eq!(video.title, "Video eins");
    assert_eq!(video.summary.as_deref(), Some("S3"));
    assert_eq!(
        video.thumbnail.as_deref(),
        Some("data:image/jpeg;base64,/9j/AA==")
    );
    let legacy = storage::get_summaries(&paths, 2).unwrap();
    assert_eq!(legacy.len(), 1);
    assert_eq!(legacy[0].summary, "Alt");
    assert_eq!(
        storage::get_chat(&paths, 1)
            .unwrap()
            .unwrap()
            .context_options
            .summary_ids,
        Some(vec![1])
    );

    // Zweiter Start ändert nichts (auch nicht die Outbox-`seq`).
    let before = dump(&conn);
    drop(conn);
    storage::init_db(&paths).unwrap();
    assert_eq!(dump(&open(&paths)), before);
}

/// C21: Inhalte, Integer-ids, Referenzen und Autoincrement-Stand überstehen den
/// Umbau; ein Abbruch mitten im Umbau (nach `DROP TABLE videos`) rollt zurück.
#[test]
fn c21_rebuild_keeps_rows_ids_references_and_sequence() {
    let (_temp, paths) = legacy_db();
    let mut conn = open(&paths);
    let content = legacy_content(&conn);
    let full = dump(&conn);
    assert_eq!(videos_sequence(&conn), 3);

    // Zwischenstände 2/3/4 liegen im Umbau: nach dem Kopieren, nach DROP,
    // nach RENAME.
    for checkpoint in [2, 3, 4] {
        let error = storage::migrate(&mut conn, Some(checkpoint)).unwrap_err();
        assert_eq!(error, format!("{ABORTED} {checkpoint}"));
        assert_eq!(dump(&conn), full, "Abbruch nach {checkpoint}");
    }

    storage::migrate(&mut conn, None).unwrap();
    let after = legacy_content(&conn);
    // Einzige neue Zeile: die nachgezogene Legacy-Summary (id 4).
    let new_rows: Vec<&String> = after
        .iter()
        .filter(|line| !content.contains(line))
        .collect();
    assert_eq!(new_rows.len(), 1, "{new_rows:?}");
    assert!(new_rows[0].contains("Integer(4), Integer(2), Text(\"Alt\")"));
    assert_eq!(after.len(), content.len() + 1);
    assert_eq!(videos_sequence(&conn), 3);
    let violations: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
    let foreign_keys: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(foreign_keys, 1);
    drop(conn);

    let next = storage::insert_video(&paths, super::sample_video("vidDDDDDDD4"), false).unwrap();
    assert_eq!(next.id, 4);
}

/// C19 (Migrationsteil): Abbruch an jedem Zwischenstand rollt vollständig
/// zurück; die Wiederholung führt zum selben Endstand und ist idempotent.
#[test]
fn c19_migration_abort_at_every_step_rolls_back() {
    let mut checkpoint = 0;
    loop {
        let (_temp, paths) = legacy_db();
        let mut conn = open(&paths);
        let before = dump(&conn);
        match storage::migrate(&mut conn, Some(checkpoint)) {
            Err(error) => {
                assert_eq!(error, format!("{ABORTED} {checkpoint}"));
                assert_eq!(dump(&conn), before, "Abbruch nach {checkpoint}");
                storage::migrate(&mut conn, None).unwrap();
                assert_migrated(&paths);
                let migrated = dump(&conn);
                storage::migrate(&mut conn, None).unwrap();
                assert_eq!(dump(&conn), migrated);
                checkpoint += 1;
            }
            Ok(()) => break,
        }
    }
    // Basis, Spalten, 3 im Umbau, Umbau, Indizes, uids, Runden, Zeitstempel,
    // Trigger, Legacy-Summaries, Seeding.
    assert_eq!(checkpoint, 13);
}

/// C22: Altbestände werden einheitlich `…mmmZ` (SQLite rundet auf die
/// Millisekunde, rechnet Offsets nach UTC um, lässt Kanonisches stehen und
/// Unlesbares unverändert); Runden sind vorher nach Originalzeit gebildet.
#[test]
fn c22_canonicalizes_legacy_timestamps() {
    let (_temp, paths) = legacy_db();
    {
        let conn = open(&paths);
        conn.execute_batch(
            "INSERT INTO chat_messages (id, chat_id, role, content, created_at) VALUES
                 (8, 2, 'user', 'kanonisch', '2026-09-21T09:30:00.123Z'),
                 (9, 2, 'user', 'gerundet', '2026-09-21T09:40:00.1239+00:00');
             UPDATE collections SET updated_at = 'unlesbar' WHERE id = 2;",
        )
        .unwrap();
    }
    storage::init_db(&paths).unwrap();
    let conn = open(&paths);
    let message_time = |id: i64| {
        text(
            &conn,
            "SELECT created_at FROM chat_messages WHERE id = ?1",
            id,
        )
    };
    assert_eq!(message_time(1), "2026-09-20T11:00:00.100Z");
    // Nanosekunden: B (.000100) und C (.000400) werden beide .000Z …
    assert_eq!(message_time(3), "2026-09-20T11:05:00.000Z");
    assert_eq!(message_time(6), "2026-09-20T11:05:00.000Z");
    // … bleiben aber getrennte Runden.
    assert_ne!(round_of(&conn, 3).0, round_of(&conn, 6).0);
    assert_eq!(message_time(7), "2026-09-21T09:00:00.000Z");
    assert_eq!(message_time(8), "2026-09-21T09:30:00.123Z");
    assert_eq!(message_time(9), "2026-09-21T09:40:00.124Z");

    let video_times = strings(
        &conn,
        "SELECT created_at || ' ' || updated_at FROM videos WHERE id >= ?1 ORDER BY id",
        1,
    );
    assert_eq!(
        video_times,
        vec![
            "2026-09-20T10:00:00.123Z 2026-09-20T10:06:00.250Z",
            // +02:00 nach UTC.
            "2026-09-20T10:00:00.500Z 2026-09-21T08:00:00.000Z",
        ]
    );
    assert_eq!(
        text(&conn, "SELECT published_at FROM videos WHERE id = ?1", 1),
        "2026-01-02"
    );
    assert_eq!(
        text(&conn, "SELECT created_at FROM summaries WHERE id = ?1", 1),
        "2026-09-20T10:05:00.000Z"
    );
    assert_eq!(
        text(
            &conn,
            "SELECT created_at || ' ' || updated_at FROM chats WHERE id = ?1",
            1
        ),
        "2026-09-20T11:00:00.100Z 2026-09-20T11:05:00.000Z"
    );
    assert_eq!(
        text(
            &conn,
            "SELECT created_at FROM video_collections WHERE collection_id = ?1 LIMIT 1",
            2
        ),
        "2026-09-06T00:00:00.000Z"
    );
    assert_eq!(
        text(
            &conn,
            "SELECT created_at || ' ' || updated_at FROM collections WHERE id = ?1",
            2
        ),
        "2026-09-02T00:00:00.000Z unlesbar"
    );
    // Die Outbox bleibt auch für Unlesbares kanonisch.
    let changed_at: String = conn
        .query_row(
            "SELECT o.changed_at FROM sync_outbox o JOIN collections c ON c.uid = o.key \
             WHERE o.entity = 'collection' AND c.id = 2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(sync_proto::is_canonical_time(&changed_at), "{changed_at}");
    // Chat-Nachrichten in Originalreihenfolge.
    let ids: Vec<i64> = storage::get_chat_messages(&paths, 2)
        .unwrap()
        .iter()
        .map(|message| message.id)
        .collect();
    assert_eq!(ids, vec![7, 8, 9]);
}

/// Schon vorhandene verwaiste Zeilen blockieren die Migration nicht, bleiben
/// unverändert und werden nicht in die Outbox aufgenommen.
#[test]
fn legacy_orphans_survive_migration_without_outbox_entries() {
    let (_temp, paths) = legacy_db();
    {
        let conn = Connection::open(&paths.db_path).unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = OFF;
             INSERT INTO summaries (id, video_id, created_at, summary) VALUES (10, 99, '2026-09-01T00:00:00Z', 'Waise');
             INSERT INTO chats (id, video_id, title, created_at, updated_at) VALUES (10, 99, 'Waise', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z');
             INSERT INTO chat_messages (id, chat_id, role, content, created_at) VALUES (20, 10, 'user', 'Waise', '2026-09-01T00:00:00Z');
             INSERT INTO video_collections (video_id, collection_id, created_at) VALUES (1, 99, '2026-09-01T00:00:00Z');",
        )
        .unwrap();
    }

    storage::init_db(&paths).unwrap();

    let conn = open(&paths);
    let orphans: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(orphans, 3);
    assert_eq!(count(&conn, "summaries"), 5);
    assert_eq!(count(&conn, "chat_messages"), 8);
    let orphan_keys = [
        text(&conn, "SELECT uid FROM summaries WHERE id = ?1", 10),
        text(&conn, "SELECT uid FROM chats WHERE id = ?1", 10),
        text(
            &conn,
            "SELECT round_uid FROM chat_messages WHERE id = ?1",
            20,
        ),
    ];
    let keys: Vec<String> = super::outbox_keys(&conn)
        .into_iter()
        .map(|(_, key)| key)
        .collect();
    assert_eq!(keys.len(), 17);
    for key in &orphan_keys {
        assert!(!keys.contains(key), "{key}");
    }
    assert!(!keys.iter().any(|key| key.ends_with("/99")));
}
