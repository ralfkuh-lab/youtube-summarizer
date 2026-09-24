//! Transaktionen, Dublettenprüfung, Sortierungen und Zuordnungs-Diff: C18,
//! C23 und die Sortier- und Diff-Regeln aus „Neue Spalten und Änderungen“.

use std::sync::{mpsc, Arc, Barrier};
use std::thread;
use std::time::Duration;

use rusqlite::{params, Connection};

use super::{clear_outbox, open, outbox_keys, sample_video, strings, temp_paths, video_uid};
use crate::models::NewChatMessage;
use crate::storage::{self, VIDEO_EXISTS_ERROR};

/// C18: Ein Schreiber (wie Apply) hält die Sperre, während `append_chat_turn`
/// beginnt. Mit `IMMEDIATE` wartet der Aufruf statt nach dem Lesen mit
/// `BUSY_SNAPSHOT` abzubrechen.
#[test]
fn c18_append_chat_turn_waits_for_concurrent_writer() {
    let (_temp, paths) = temp_paths();
    let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), false).unwrap();
    let (chat, _) = storage::append_chat_turn(
        &paths,
        video.id,
        None,
        "Chat",
        vec![NewChatMessage::user("Erste")],
        None,
    )
    .unwrap();

    let (locked, wait_locked) = mpsc::channel();
    let db_path = paths.db_path.clone();
    let writer = thread::spawn(move || {
        let mut conn = Connection::open(db_path).unwrap();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        tx.execute(
            "UPDATE chats SET title = 'Vom Server' WHERE id = ?1",
            params![chat.id],
        )
        .unwrap();
        locked.send(()).unwrap();
        thread::sleep(Duration::from_millis(300));
        tx.commit().unwrap();
    });
    wait_locked.recv().unwrap();

    let (_, records) = storage::append_chat_turn(
        &paths,
        video.id,
        Some(chat.id),
        "Chat",
        vec![
            NewChatMessage::user("Zweite"),
            NewChatMessage::assistant("Antwort"),
        ],
        None,
    )
    .unwrap();
    writer.join().unwrap();

    assert_eq!(records.len(), 2);
    let contents: Vec<String> = storage::get_chat_messages(&paths, chat.id)
        .unwrap()
        .into_iter()
        .map(|message| message.content)
        .collect();
    assert_eq!(contents, vec!["Erste", "Zweite", "Antwort"]);
    assert_eq!(
        storage::get_chat(&paths, chat.id).unwrap().unwrap().title,
        "Vom Server"
    );
}

/// C23: Zwei gleichzeitige Einfügungen derselben YouTube-ID ergeben genau eine
/// Zeile, auch ohne `UNIQUE(video_id)`.
#[test]
fn c23_concurrent_insert_video_keeps_one_row() {
    let (_temp, paths) = temp_paths();
    for round in 0..20 {
        let video_id = format!("vid{round:08}");
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let paths = paths.clone();
                let barrier = barrier.clone();
                let video_id = video_id.clone();
                thread::spawn(move || {
                    barrier.wait();
                    storage::insert_video(&paths, sample_video(&video_id), false)
                })
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let error = results.into_iter().find_map(Result::err).unwrap();
        assert_eq!(error, VIDEO_EXISTS_ERROR);
        let count: i64 = open(&paths)
            .query_row(
                "SELECT COUNT(*) FROM videos WHERE video_id = ?1",
                params![video_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "{video_id}");
    }
}

/// „Neueste“ bei gleichem `created_at` entscheidet die uid (absteigend) — in
/// `get_summaries`, `delete_summary` und `update_summary` gleich.
#[test]
fn newest_summary_breaks_ties_by_uid() {
    let (_temp, paths) = temp_paths();
    let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), false).unwrap();
    for text in ["A", "B", "C"] {
        storage::update_summary(&paths, video.id, text, None, None, None).unwrap();
    }
    let conn = open(&paths);
    conn.execute_batch(
        "UPDATE summaries SET created_at = '2026-09-24T10:00:00.000Z';
         UPDATE summaries SET uid = '00000000000000000000000000000001' WHERE summary = 'C';
         UPDATE summaries SET uid = 'ffffffffffffffffffffffffffffffff' WHERE summary = 'A';
         UPDATE summaries SET uid = '80000000000000000000000000000000' WHERE summary = 'B';",
    )
    .unwrap();
    let order: Vec<String> = storage::get_summaries(&paths, video.id)
        .unwrap()
        .into_iter()
        .map(|summary| summary.summary)
        .collect();
    assert_eq!(order, vec!["A", "B", "C"]);

    let c = storage::get_summaries(&paths, video.id).unwrap()[2].id;
    storage::delete_summary(&paths, c).unwrap();
    let current = storage::get_video(&paths, video.id).unwrap().unwrap();
    assert_eq!(current.summary.as_deref(), Some("A"));
}

/// Chat-Nachrichten: `created_at, round_uid, position` — Runden gleicher Zeit
/// nach uid, innerhalb der Runde nach Position (nicht nach id).
#[test]
fn chat_messages_sort_by_time_round_and_position() {
    let (_temp, paths) = temp_paths();
    let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), false).unwrap();
    let (chat, _) = storage::append_chat_turn(
        &paths,
        video.id,
        None,
        "Chat",
        vec![
            NewChatMessage::user("R1 a"),
            NewChatMessage::assistant("R1 b"),
        ],
        None,
    )
    .unwrap();
    storage::append_chat_turn(
        &paths,
        video.id,
        Some(chat.id),
        "Chat",
        vec![
            NewChatMessage::user("R2 a"),
            NewChatMessage::assistant("R2 b"),
        ],
        None,
    )
    .unwrap();
    let conn = open(&paths);
    conn.execute_batch(
        "UPDATE chat_messages SET created_at = '2026-09-24T10:00:00.000Z';
         UPDATE chat_messages SET round_uid = 'b0000000000000000000000000000000' WHERE content LIKE 'R1%';
         UPDATE chat_messages SET round_uid = 'a0000000000000000000000000000000' WHERE content LIKE 'R2%';
         UPDATE chat_messages SET position = 1 - position WHERE content LIKE 'R2%';",
    )
    .unwrap();
    let contents: Vec<String> = storage::get_chat_messages(&paths, chat.id)
        .unwrap()
        .into_iter()
        .map(|message| message.content)
        .collect();
    assert_eq!(contents, vec!["R2 b", "R2 a", "R1 a", "R1 b"]);
}

/// Neue Schreibpfade schreiben kanonische Zeitstempel.
#[test]
fn write_paths_store_canonical_timestamps() {
    let (_temp, paths) = temp_paths();
    let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), false).unwrap();
    storage::update_summary(&paths, video.id, "S", None, None, None).unwrap();
    let collection = storage::create_collection(&paths, "KI").unwrap();
    storage::set_video_collections(&paths, video.id, vec![collection.id]).unwrap();
    storage::append_chat_turn(
        &paths,
        video.id,
        None,
        "Chat",
        vec![NewChatMessage::user("Frage")],
        None,
    )
    .unwrap();
    let conn = open(&paths);
    for sql in [
        "SELECT created_at FROM videos UNION ALL SELECT updated_at FROM videos",
        "SELECT created_at FROM summaries",
        "SELECT created_at FROM chats UNION ALL SELECT updated_at FROM chats",
        "SELECT created_at FROM chat_messages",
        "SELECT created_at FROM collections UNION ALL SELECT updated_at FROM collections",
        "SELECT created_at FROM video_collections",
        "SELECT changed_at FROM sync_outbox",
    ] {
        let values = strings(&conn, &format!("{sql} LIMIT ?1"), 100);
        assert!(!values.is_empty(), "{sql}");
        assert!(
            values
                .iter()
                .all(|value| sync_proto::is_canonical_time(value)),
            "{sql}: {values:?}"
        );
    }
}

/// `set_video_collections` schreibt nur echte Differenzen.
#[test]
fn set_video_collections_writes_only_differences() {
    let (_temp, paths) = temp_paths();
    let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), false).unwrap();
    let [a, b, c] = ["A", "B", "C"].map(|name| storage::create_collection(&paths, name).unwrap());
    storage::set_video_collections(&paths, video.id, vec![a.id, b.id]).unwrap();
    let conn = open(&paths);
    let created_b: String = conn
        .query_row(
            "SELECT created_at FROM video_collections WHERE collection_id = ?1",
            params![b.id],
            |row| row.get(0),
        )
        .unwrap();
    clear_outbox(&conn);

    let updated = storage::set_video_collections(&paths, video.id, vec![c.id, b.id, c.id]).unwrap();

    assert_eq!(updated.collection_ids, vec![b.id, c.id]);
    let uid = video_uid(&conn, video.id);
    let collection_uid = |id: i64| {
        conn.query_row(
            "SELECT uid FROM collections WHERE id = ?1",
            params![id],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
    };
    let mut expected = vec![
        (
            "membership".to_string(),
            format!("{uid}/{}", collection_uid(a.id)),
        ),
        (
            "membership".to_string(),
            format!("{uid}/{}", collection_uid(c.id)),
        ),
    ];
    expected.sort();
    assert_eq!(outbox_keys(&conn), expected);
    let still: String = conn
        .query_row(
            "SELECT created_at FROM video_collections WHERE collection_id = ?1",
            params![b.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(still, created_b);
}
