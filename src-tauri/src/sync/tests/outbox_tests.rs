//! Trigger, Outbox und Privatisierung: C2–C8.

use rusqlite::params;

use super::{
    clear_outbox, open, outbox, outbox_keys, sample_video, strings, temp_paths, text, video_uid,
};
use crate::models::NewChatMessage;
use crate::storage;

fn key(entity: &str, key: &str) -> (String, String) {
    (entity.to_string(), key.to_string())
}

fn chat_turn(paths: &storage::AppPaths, video: i64, chat: Option<i64>, text: &str) -> i64 {
    storage::append_chat_turn(
        paths,
        video,
        chat,
        "Chat",
        vec![
            NewChatMessage::user(text),
            NewChatMessage::assistant("Antwort"),
        ],
        None,
    )
    .unwrap()
    .0
    .id
}

#[test]
fn c2_every_insert_path_sets_uid_and_missing_uid_aborts() {
    let (_temp, paths) = temp_paths();
    let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), false).unwrap();
    storage::update_summary(&paths, video.id, "S", None, None, None).unwrap();
    let chat = chat_turn(&paths, video.id, None, "Frage");
    storage::create_collection(&paths, "KI").unwrap();
    // backfill_legacy_summaries: Video mit `videos.summary`, aber ohne Historie.
    let legacy = storage::insert_video(&paths, sample_video("vidBBBBBBB2"), false).unwrap();
    open(&paths)
        .execute(
            "UPDATE videos SET summary = 'Alt' WHERE id = ?1",
            params![legacy.id],
        )
        .unwrap();
    storage::init_db(&paths).unwrap();
    assert_eq!(storage::get_summaries(&paths, legacy.id).unwrap().len(), 1);

    let conn = open(&paths);
    for (table, column) in [
        ("videos", "uid"),
        ("summaries", "uid"),
        ("chats", "uid"),
        ("collections", "uid"),
        ("chat_messages", "round_uid"),
    ] {
        let values = strings(&conn, &format!("SELECT {column} FROM {table} WHERE ?1"), 1);
        assert!(!values.is_empty(), "{table}");
        assert!(
            values.iter().all(|value| sync_proto::is_uid(value)),
            "{table}: {values:?}"
        );
    }
    let positions = strings(
        &conn,
        "SELECT CAST(position AS TEXT) FROM chat_messages WHERE chat_id = ?1 ORDER BY id",
        chat,
    );
    assert_eq!(positions, vec!["0", "1"]);

    for sql in [
        "INSERT INTO videos (video_id, url, title, thumbnail_url, created_at, updated_at) \
         VALUES ('vidCCCCCCC3', 'u', 't', 't', 'x', 'x')",
        "INSERT INTO summaries (video_id, created_at, summary) VALUES (1, 'x', 's')",
        "INSERT INTO chats (video_id, title, created_at, updated_at) VALUES (1, 't', 'x', 'x')",
        "INSERT INTO collections (name, created_at, updated_at) VALUES ('Neu', 'x', 'x')",
        "INSERT INTO chat_messages (chat_id, role, content, created_at) VALUES (1, 'user', 'c', 'x')",
    ] {
        let error = conn.execute(sql, []).unwrap_err().to_string();
        assert!(error.contains("fehlt"), "{sql}: {error}");
    }
}

#[test]
fn c3_private_video_writes_no_outbox_entries() {
    let (_temp, paths) = temp_paths();
    let collection = storage::create_collection(&paths, "KI").unwrap();
    let conn = open(&paths);
    clear_outbox(&conn);

    let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), true).unwrap();
    storage::update_summary(&paths, video.id, "S", None, None, None).unwrap();
    let chat = chat_turn(&paths, video.id, None, "Frage");
    chat_turn(&paths, video.id, Some(chat), "Noch eine");
    storage::set_video_collections(&paths, video.id, vec![collection.id]).unwrap();
    storage::update_transcript(&paths, video.id, "[]", None, None).unwrap();

    assert_eq!(outbox(&conn), Vec::new());
}

/// Veröffentlichtes Video mit drei Summaries und drei Runden je gleichem
/// Zeitstempel (Reihenfolge nur über uids) und einem Chat mit
/// `pendingSummaryUids`.
fn published_video_with_ties(paths: &storage::AppPaths) -> (i64, i64) {
    let video = storage::insert_video(paths, sample_video("vidAAAAAAA1"), false).unwrap();
    for text in ["S1", "S2", "S3", "S4"] {
        storage::update_summary(paths, video.id, text, None, None, None).unwrap();
    }
    let chat = chat_turn(paths, video.id, None, "R1");
    chat_turn(paths, video.id, Some(chat), "R2");
    chat_turn(paths, video.id, Some(chat), "R3");
    let conn = open(paths);
    conn.execute_batch(
        "UPDATE summaries SET created_at = '2026-09-24T10:00:00.000Z';
         UPDATE chat_messages SET created_at = '2026-09-24T11:00:00.000Z';
         UPDATE videos SET published = 1;",
    )
    .unwrap();
    storage::refresh_latest_summary(&conn, video.id, "2026-09-24T12:00:00.000Z").unwrap();
    let pending = text(
        &conn,
        "SELECT uid FROM summaries WHERE summary = 'S2' AND video_id = ?1",
        video.id,
    );
    conn.execute(
        "UPDATE chats SET context_options = ?1 WHERE id = ?2",
        params![
            serde_json::json!({
                "transcript": true,
                "summaryIds": null,
                "pendingSummaryUids": [pending, "f".repeat(32)],
            })
            .to_string(),
            chat
        ],
    )
    .unwrap();
    (video.id, chat)
}

/// Sichtbarer Zustand: Summaries (neueste zuerst), aktuelle Summary,
/// Nachrichten in Chat-Reihenfolge samt Rundenaufteilung.
fn visible_state(paths: &storage::AppPaths, video: i64, chat: i64) -> Vec<String> {
    let mut state: Vec<String> = storage::get_summaries(paths, video)
        .unwrap()
        .into_iter()
        .map(|summary| format!("summary {} {}", summary.id, summary.summary))
        .collect();
    let current = storage::get_video(paths, video).unwrap().unwrap();
    state.push(format!("current {:?} {:?}", current.summary, current.title));
    let conn = open(paths);
    let mut previous_round = String::new();
    let mut round = 0;
    for message in storage::get_chat_messages(paths, chat).unwrap() {
        let uid = text(
            &conn,
            "SELECT round_uid FROM chat_messages WHERE id = ?1",
            message.id,
        );
        if uid != previous_round {
            round += 1;
            previous_round = uid;
        }
        state.push(format!(
            "round {round} message {} {}",
            message.id, message.content
        ));
    }
    state
}

fn subtree_uids(conn: &rusqlite::Connection, video: i64) -> Vec<String> {
    let mut uids = strings(conn, "SELECT uid FROM videos WHERE id = ?1", video);
    uids.extend(strings(
        conn,
        "SELECT uid FROM summaries WHERE video_id = ?1",
        video,
    ));
    uids.extend(strings(
        conn,
        "SELECT uid FROM chats WHERE video_id = ?1",
        video,
    ));
    uids.extend(strings(
        conn,
        "SELECT DISTINCT m.round_uid FROM chat_messages m JOIN chats c ON c.id = m.chat_id \
         WHERE c.video_id = ?1",
        video,
    ));
    uids
}

#[test]
fn c4_delete_summary_then_privatize_published_video() {
    let (_temp, paths) = temp_paths();
    let (video, chat) = published_video_with_ties(&paths);
    let conn = open(&paths);
    let old_uid = video_uid(&conn, video);
    let old_uids = subtree_uids(&conn, video);
    let (removed_id, removed_uid): (i64, String) = conn
        .query_row(
            "SELECT id, uid FROM summaries WHERE summary = 'S1' AND video_id = ?1",
            params![video],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    storage::delete_summary(&paths, removed_id).unwrap();
    assert!(outbox_keys(&conn).contains(&key("summary", &removed_uid)));
    let before = visible_state(&paths, video, chat);
    let pending_old = text(
        &conn,
        "SELECT uid FROM summaries WHERE summary = 'S2' AND video_id = ?1",
        video,
    );

    storage::video_set_local_only(&paths, video, true).unwrap();

    // Summary-Eintrag weg, genau ein `withdrawn` mit alter uid.
    assert_eq!(
        outbox(&conn),
        vec![(
            "video".to_string(),
            old_uid.clone(),
            Some(old_uid.clone()),
            None,
            Some("withdrawn".to_string())
        )]
    );
    let flags: (bool, bool) = conn
        .query_row(
            "SELECT local_only, published FROM videos WHERE id = ?1",
            params![video],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(flags, (true, false));
    // Neue uids im ganzen Teilbaum, Inhalte und Reihenfolgen unverändert.
    let new_uids = subtree_uids(&conn, video);
    assert_eq!(new_uids.len(), old_uids.len() - 1);
    assert!(
        new_uids.iter().all(|uid| !old_uids.contains(uid)),
        "{new_uids:?}"
    );
    assert_eq!(visible_state(&paths, video, chat), before);
    // pendingSummaryUids folgen der Zuordnung alt → neu; Fremdes bleibt.
    let pending_new = text(
        &conn,
        "SELECT uid FROM summaries WHERE summary = 'S2' AND video_id = ?1",
        video,
    );
    assert_ne!(pending_new, pending_old);
    let options: serde_json::Value = serde_json::from_str(&text(
        &conn,
        "SELECT context_options FROM chats WHERE id = ?1",
        chat,
    ))
    .unwrap();
    assert_eq!(
        options["pendingSummaryUids"],
        serde_json::json!([pending_new, "f".repeat(32)])
    );
}

#[test]
fn c4_privatize_unpublished_video_keeps_uids_and_leaves_no_entry() {
    let (_temp, paths) = temp_paths();
    let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), false).unwrap();
    storage::update_summary(&paths, video.id, "S", None, None, None).unwrap();
    chat_turn(&paths, video.id, None, "Frage");
    let conn = open(&paths);
    let before = subtree_uids(&conn, video.id);

    storage::video_set_local_only(&paths, video.id, true).unwrap();

    assert_eq!(outbox(&conn), Vec::new());
    assert_eq!(subtree_uids(&conn, video.id), before);
}

#[test]
fn c5_sharing_private_video_writes_entries_for_video_and_children() {
    let (_temp, paths) = temp_paths();
    let collection = storage::create_collection(&paths, "KI").unwrap();
    let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), true).unwrap();
    storage::update_summary(&paths, video.id, "S", None, None, None).unwrap();
    let chat = chat_turn(&paths, video.id, None, "Frage");
    storage::set_video_collections(&paths, video.id, vec![collection.id]).unwrap();
    let conn = open(&paths);
    clear_outbox(&conn);

    storage::video_set_local_only(&paths, video.id, false).unwrap();

    let uid = video_uid(&conn, video.id);
    let collection_uid = text(
        &conn,
        "SELECT uid FROM collections WHERE id = ?1",
        collection.id,
    );
    let mut expected = vec![
        key("video", &uid),
        key(
            "summary",
            &text(
                &conn,
                "SELECT uid FROM summaries WHERE video_id = ?1",
                video.id,
            ),
        ),
        key(
            "chat",
            &text(&conn, "SELECT uid FROM chats WHERE id = ?1", chat),
        ),
        key(
            "round",
            &text(
                &conn,
                "SELECT round_uid FROM chat_messages WHERE chat_id = ?1",
                chat,
            ),
        ),
        key("membership", &format!("{uid}/{collection_uid}")),
    ];
    expected.sort();
    assert_eq!(outbox_keys(&conn), expected);
    assert!(outbox(&conn)
        .iter()
        .all(|entry| entry.2.as_deref() == Some(uid.as_str())));
}

#[test]
fn c6_deleting_video_writes_tombstone_only_when_published() {
    for published in [true, false] {
        let (_temp, paths) = temp_paths();
        let collection = storage::create_collection(&paths, "KI").unwrap();
        let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), false).unwrap();
        storage::update_summary(&paths, video.id, "S", None, None, None).unwrap();
        chat_turn(&paths, video.id, None, "Frage");
        storage::set_video_collections(&paths, video.id, vec![collection.id]).unwrap();
        let conn = open(&paths);
        conn.execute(
            "UPDATE videos SET published = ?1 WHERE id = ?2",
            params![published, video.id],
        )
        .unwrap();
        let uid = video_uid(&conn, video.id);

        storage::delete_video(&paths, video.id).unwrap();

        let entries: Vec<_> = outbox(&conn)
            .into_iter()
            .filter(|entry| entry.0 != "collection")
            .collect();
        let expected = if published {
            vec![(
                "video".to_string(),
                uid.clone(),
                Some(uid.clone()),
                None,
                Some("deleted".to_string()),
            )]
        } else {
            Vec::new()
        };
        assert_eq!(entries, expected, "published = {published}");
    }
}

#[test]
fn c7_deleting_chat_drops_open_round_entry() {
    let (_temp, paths) = temp_paths();
    let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), false).unwrap();
    let chat = chat_turn(&paths, video.id, None, "Frage");
    let conn = open(&paths);
    let chat_uid = text(&conn, "SELECT uid FROM chats WHERE id = ?1", chat);
    let round = text(
        &conn,
        "SELECT round_uid FROM chat_messages WHERE chat_id = ?1",
        chat,
    );
    assert!(outbox_keys(&conn).contains(&key("round", &round)));

    storage::delete_chat(&paths, chat).unwrap();

    let keys = outbox_keys(&conn);
    assert!(
        !keys.iter().any(|(entity, _)| entity == "round"),
        "{keys:?}"
    );
    assert!(keys.contains(&key("chat", &chat_uid)));
    let entry = outbox(&conn)
        .into_iter()
        .find(|entry| entry.1 == chat_uid)
        .unwrap();
    assert_eq!(entry.2, Some(video_uid(&conn, video.id)));
}

#[test]
fn c8_deleting_collection_drops_open_membership_entries() {
    let (_temp, paths) = temp_paths();
    let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), false).unwrap();
    let collection = storage::create_collection(&paths, "KI").unwrap();
    let conn = open(&paths);
    let collection_uid = text(
        &conn,
        "SELECT uid FROM collections WHERE id = ?1",
        collection.id,
    );
    clear_outbox(&conn);
    storage::set_video_collections(&paths, video.id, vec![collection.id]).unwrap();
    assert_eq!(outbox(&conn).len(), 1);

    storage::delete_collection(&paths, collection.id).unwrap();

    assert_eq!(outbox_keys(&conn), vec![key("collection", &collection_uid)]);
}
