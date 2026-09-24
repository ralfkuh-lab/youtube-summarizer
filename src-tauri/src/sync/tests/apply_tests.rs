//! Staging und Apply: C11–C17, C19 (Apply-Teil), C20 und das Zurückstellen.

use rusqlite::{params, Connection};
use sync_proto::{GoneReason, Op, State};

use super::{
    apply_states, as_states, chat_state, collection_state, collection_uid, dump, gone, inbox_count,
    membership_state, open, outbox, outbox_keys, push_all, round_state, sample_video, stage,
    strings, summary_state, temp_paths, text, uid, video_state, video_uid, T0,
};
use crate::models::NewChatMessage;
use crate::storage::{self, AppPaths};
use crate::sync::{apply, pull, snapshot, ABORTED};

/// Video anlegen und pushen: geteilt, veröffentlicht, Outbox leer.
fn shared_video(paths: &AppPaths, youtube_id: &str) -> (i64, String) {
    let video = storage::insert_video(paths, sample_video(youtube_id), false).unwrap();
    let mut conn = open(paths);
    push_all(&mut conn);
    (video.id, video_uid(&conn, video.id))
}

fn count(conn: &Connection, sql: &str, value: &str) -> i64 {
    conn.query_row(sql, params![value], |row| row.get(0))
        .unwrap()
}

fn titles(paths: &AppPaths) -> Vec<String> {
    let mut titles: Vec<String> = storage::get_videos(paths)
        .unwrap()
        .into_iter()
        .map(|video| video.title)
        .collect();
    titles.sort();
    titles
}

fn collection_names(paths: &AppPaths) -> Vec<String> {
    storage::get_collections(paths)
        .unwrap()
        .into_iter()
        .map(|collection| collection.name)
        .collect()
}

fn private_and_published(conn: &Connection, id: i64) -> (bool, bool) {
    conn.query_row(
        "SELECT local_only, published FROM videos WHERE id = ?1",
        params![id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

/// Staging legt Seiten ab und setzt den Cursor; Apply verdichtet je Schlüssel
/// auf die höchste `seq` und leert die Inbox.
#[test]
fn staging_sets_cursor_and_apply_compacts_per_key() {
    let (_temp, paths) = temp_paths();
    let mut conn = open(&paths);
    let v = uid(1);
    stage(&mut conn, vec![video_state(&v, "vidAAAAAAA1", "alt")]);
    assert_eq!(pull::cursor(&conn).unwrap(), 1);
    stage(
        &mut conn,
        vec![
            collection_state(&uid(2), "KI"),
            video_state(&v, "vidAAAAAAA1", "neu"),
        ],
    );
    assert_eq!(pull::cursor(&conn).unwrap(), 3);
    assert_eq!(inbox_count(&conn), 3);

    let applied = apply::apply(&mut conn).unwrap();

    assert_eq!(titles(&paths), vec!["neu"]);
    assert_eq!(collection_names(&paths), vec!["KI"]);
    assert!(applied.collections);
    assert_eq!(applied.video_ids.len(), 1);
    assert_eq!(inbox_count(&conn), 0);
    // Trigger schweigen während Apply, `applying` ist danach entfernt.
    assert_eq!(outbox(&conn), Vec::new());
    assert_eq!(crate::sync::state_get(&conn, "applying").unwrap(), None);
    // Eingehende (kanonische) Zeitstempel bleiben unverändert.
    assert_eq!(
        text(&conn, "SELECT created_at FROM videos WHERE uid = ?1", &v),
        T0
    );
}

/// C11: Lebender Zustand bei ausstehendem Eintrag ohne lokale Zeile
/// (Löschwunsch) → nichts eingefügt, Eintrag bleibt.
#[test]
fn c11_live_state_never_undoes_pending_delete() {
    let (_temp, paths) = temp_paths();
    let (video, v) = shared_video(&paths, "vidAAAAAAA1");
    let (other, o) = shared_video(&paths, "vidBBBBBBB2");
    storage::update_summary(&paths, video, "S", None, None, None).unwrap();
    let mut conn = open(&paths);
    push_all(&mut conn);
    let summary = storage::get_summaries(&paths, video).unwrap()[0].clone();
    let s = text(&conn, "SELECT uid FROM summaries WHERE id = ?1", summary.id);
    storage::delete_summary(&paths, summary.id).unwrap();
    storage::delete_video(&paths, other).unwrap();
    let before = outbox(&conn);
    assert_eq!(before.len(), 2);

    apply_states(
        &mut conn,
        vec![
            video_state(&o, "vidBBBBBBB2", "Testvideo"),
            summary_state(&s, &v, T0, "S"),
        ],
    );

    assert!(storage::get_summaries(&paths, video).unwrap().is_empty());
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM videos WHERE uid = ?1", &o),
        0
    );
    assert_eq!(outbox(&conn), before);
}

/// Sichtbarer Zustand eines Videos: Summaries (neueste zuerst) mit aktueller
/// Summary, Chats mit Nachrichten in Reihenfolge.
fn visible(paths: &AppPaths, video: i64) -> Vec<String> {
    let mut state: Vec<String> = storage::get_summaries(paths, video)
        .unwrap()
        .into_iter()
        .map(|summary| format!("summary {} {}", summary.id, summary.summary))
        .collect();
    let current = storage::get_video(paths, video).unwrap().unwrap();
    state.push(format!("current {:?}", current.summary));
    for chat in storage::list_chats(paths, video).unwrap() {
        for message in storage::get_chat_messages(paths, chat.id).unwrap() {
            state.push(format!(
                "chat {} {} {}",
                chat.id, message.id, message.content
            ));
        }
    }
    state
}

fn subtree_uids(conn: &Connection, video: i64) -> Vec<String> {
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

/// C12: `videoGone withdrawn` → privat, Teilbaum neu identifiziert, Inhalte
/// bleiben, Owner-Outbox leer; dasselbe Echo erneut wirkt nicht.
#[test]
fn c12_withdrawn_privatizes_and_repeated_echo_is_harmless() {
    let (_temp, paths) = temp_paths();
    let (video, v) = shared_video(&paths, "vidAAAAAAA1");
    for text in ["S1", "S2", "S3"] {
        storage::update_summary(&paths, video, text, None, None, None).unwrap();
    }
    let (chat, _) = storage::append_chat_turn(
        &paths,
        video,
        None,
        "Chat",
        vec![NewChatMessage::user("R1"), NewChatMessage::assistant("A1")],
        None,
    )
    .unwrap();
    storage::append_chat_turn(
        &paths,
        video,
        Some(chat.id),
        "Chat",
        vec![NewChatMessage::user("R2")],
        None,
    )
    .unwrap();
    let mut conn = open(&paths);
    conn.execute_batch(
        "UPDATE summaries SET created_at = '2026-09-24T10:00:00.000Z';
         UPDATE chat_messages SET created_at = '2026-09-24T11:00:00.000Z';",
    )
    .unwrap();
    storage::refresh_latest_summary(&conn, video, T0).unwrap();
    push_all(&mut conn);
    // Eine noch nicht gesendete lokale Änderung des Teilbaums.
    storage::update_summary(&paths, video, "S4", None, None, None).unwrap();
    conn.execute(
        "UPDATE summaries SET created_at = '2026-09-24T10:00:00.000Z' WHERE summary = 'S4'",
        [],
    )
    .unwrap();
    storage::refresh_latest_summary(&conn, video, T0).unwrap();
    assert_eq!(outbox(&conn).len(), 1);
    let before = visible(&paths, video);
    let old_uids = subtree_uids(&conn, video);

    apply_states(&mut conn, vec![gone(&v, GoneReason::Withdrawn, None)]);

    assert_eq!(private_and_published(&conn, video), (true, false));
    assert_eq!(visible(&paths, video), before);
    let new_uids = subtree_uids(&conn, video);
    assert_eq!(new_uids.len(), old_uids.len());
    assert!(
        new_uids.iter().all(|uid| !old_uids.contains(uid)),
        "{new_uids:?}"
    );
    assert_eq!(outbox(&conn), Vec::new());

    let after = dump(&conn);
    apply_states(&mut conn, vec![gone(&v, GoneReason::Withdrawn, None)]);
    // Nur der Cursor rückt weiter.
    let changed: Vec<String> = dump(&conn)
        .into_iter()
        .filter(|line| !after.contains(line))
        .collect();
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert!(changed[0].contains("cursor"), "{changed:?}");
}

/// C13: `merged` ohne und mit Zielzeile: Outbox-Einträge der alten uid
/// umgeschrieben (`changed_at` erhalten, bei Kollision der jüngere), keine
/// Dubletten, lokale Zuordnungen bleiben; Alias vor Grabstein im selben
/// Batch.
#[test]
fn c13_merged_video_without_target_renames_and_rewrites_outbox() {
    let (_temp, paths) = temp_paths();
    let collection = storage::create_collection(&paths, "KI").unwrap();
    let (video, x) = shared_video(&paths, "vidAAAAAAA1");
    storage::update_summary(&paths, video, "S", None, None, None).unwrap();
    storage::set_video_collections(&paths, video, vec![collection.id]).unwrap();
    let mut conn = open(&paths);
    conn.execute(
        "UPDATE sync_outbox SET changed_at = '2026-01-01T00:00:00.000Z'",
        [],
    )
    .unwrap();
    let c = text(
        &conn,
        "SELECT uid FROM collections WHERE id = ?1",
        collection.id,
    );
    let m = uid(0xabc);

    apply_states(&mut conn, vec![gone(&x, GoneReason::Merged, Some(&m))]);

    assert_eq!(video_uid(&conn, video), m);
    let summary_uid = text(
        &conn,
        "SELECT uid FROM summaries WHERE video_id = ?1",
        video,
    );
    let mut expected = vec![
        (
            "membership".to_string(),
            format!("{m}/{c}"),
            Some(m.clone()),
            Some(c.clone()),
            None,
        ),
        (
            "summary".to_string(),
            summary_uid,
            Some(m.clone()),
            None,
            None,
        ),
    ];
    expected.sort();
    let mut entries = outbox(&conn);
    entries.sort();
    assert_eq!(entries, expected);
    let times = strings(&conn, "SELECT changed_at FROM sync_outbox WHERE ?1", "1");
    assert!(times.iter().all(|time| time == "2026-01-01T00:00:00.000Z"));
    assert_eq!(
        storage::get_video(&paths, video)
            .unwrap()
            .unwrap()
            .collection_ids,
        vec![collection.id]
    );

    // Alias und Grabstein der Wurzel im selben Batch: kein lebender Rest.
    let (other, y) = shared_video(&paths, "vidBBBBBBB2");
    let root = uid(0xdef);
    apply_states(
        &mut conn,
        vec![
            gone(&root, GoneReason::Deleted, None),
            gone(&y, GoneReason::Merged, Some(&root)),
        ],
    );
    assert!(storage::get_video(&paths, other).unwrap().is_none());
}

#[test]
fn c13_merged_video_with_target_moves_children_and_merges_entries() {
    let (_temp, paths) = temp_paths();
    let [c1, c2] = ["Eins", "Zwei"].map(|name| storage::create_collection(&paths, name).unwrap());
    let (x_id, x) = shared_video(&paths, "vidAAAAAAA1");
    let (m_id, m) = shared_video(&paths, "vidBBBBBBB2");
    storage::update_summary(&paths, x_id, "SX", None, None, None).unwrap();
    storage::append_chat_turn(
        &paths,
        x_id,
        None,
        "Chat X",
        vec![NewChatMessage::user("RX")],
        None,
    )
    .unwrap();
    storage::set_video_collections(&paths, x_id, vec![c1.id, c2.id]).unwrap();
    storage::set_video_collections(&paths, m_id, vec![c1.id]).unwrap();
    storage::update_transcript(&paths, x_id, "[]", None, None).unwrap();
    storage::update_transcript(&paths, m_id, "[]", None, None).unwrap();
    let mut conn = open(&paths);
    let c1_uid = text(&conn, "SELECT uid FROM collections WHERE id = ?1", c1.id);
    // Video: M jünger; Zuordnung c1: X jünger.
    conn.execute_batch(&format!(
        "UPDATE sync_outbox SET changed_at = '2026-01-01T00:00:00.000Z';
         UPDATE sync_outbox SET changed_at = '2026-02-01T00:00:00.000Z'
             WHERE (entity = 'video' AND key = '{m}') OR key = '{x}/{c1_uid}';"
    ))
    .unwrap();
    let m_video_seq: i64 = conn
        .query_row(
            "SELECT seq FROM sync_outbox WHERE entity = 'video' AND key = ?1",
            params![m],
            |row| row.get(0),
        )
        .unwrap();

    apply_states(&mut conn, vec![gone(&x, GoneReason::Merged, Some(&m))]);

    assert!(storage::get_video(&paths, x_id).unwrap().is_none());
    let merged = storage::get_video(&paths, m_id).unwrap().unwrap();
    assert_eq!(merged.collection_ids, vec![c1.id, c2.id]);
    assert_eq!(merged.summary.as_deref(), Some("SX"));
    assert_eq!(storage::get_summaries(&paths, m_id).unwrap().len(), 1);
    assert_eq!(storage::list_chats(&paths, m_id).unwrap().len(), 1);
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM sync_outbox WHERE key LIKE '%' || ?1 || '%' OR owner = ?1",
            &x
        ),
        0
    );
    let (video_seq, video_time): (i64, String) = conn
        .query_row(
            "SELECT seq, changed_at FROM sync_outbox WHERE entity = 'video' AND key = ?1",
            params![m],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (video_seq, video_time.as_str()),
        (m_video_seq, "2026-02-01T00:00:00.000Z")
    );
    let membership_time = text(
        &conn,
        "SELECT changed_at FROM sync_outbox WHERE key = ?1",
        &format!("{m}/{c1_uid}"),
    );
    assert_eq!(membership_time, "2026-02-01T00:00:00.000Z");
    let owners = strings(
        &conn,
        "SELECT DISTINCT owner FROM sync_outbox WHERE entity != ?1",
        "x",
    );
    assert_eq!(owners, vec![m.clone()]);
}

#[test]
fn c13_merged_collection_with_and_without_target() {
    let (_temp, paths) = temp_paths();
    let (v1, v1_uid) = shared_video(&paths, "vidAAAAAAA1");
    let (v2, _) = shared_video(&paths, "vidBBBBBBB2");
    let x = storage::create_collection(&paths, "X").unwrap();
    let m = storage::create_collection(&paths, "M").unwrap();
    let lone = storage::create_collection(&paths, "Allein").unwrap();
    storage::set_video_collections(&paths, v1, vec![x.id, m.id, lone.id]).unwrap();
    storage::set_video_collections(&paths, v2, vec![x.id]).unwrap();
    let mut conn = open(&paths);
    let (x_uid, m_uid, lone_uid) = (
        collection_uid(&conn, x.id),
        collection_uid(&conn, m.id),
        collection_uid(&conn, lone.id),
    );
    let root = uid(0x77);

    apply_states(
        &mut conn,
        vec![
            State::CollectionGone {
                uid: x_uid.clone(),
                reason: GoneReason::Merged,
                merged_into: Some(m_uid.clone()),
            },
            State::CollectionGone {
                uid: lone_uid.clone(),
                reason: GoneReason::Merged,
                merged_into: Some(root.clone()),
            },
        ],
    );

    // Mit Ziel: Zuordnungen umgehängt, doppelte zusammengefasst.
    assert_eq!(collection_names(&paths), vec!["Allein", "M"]);
    assert_eq!(
        storage::get_video(&paths, v1)
            .unwrap()
            .unwrap()
            .collection_ids,
        vec![m.id, lone.id]
    );
    assert_eq!(
        storage::get_video(&paths, v2)
            .unwrap()
            .unwrap()
            .collection_ids,
        vec![m.id]
    );
    // Ohne Ziel: uid umbenannt, Einträge umgeschrieben.
    assert_eq!(collection_uid(&conn, lone.id), root);
    let keys = outbox_keys(&conn);
    assert!(keys.contains(&("collection".to_string(), root.clone())));
    assert!(keys.contains(&("membership".to_string(), format!("{v1_uid}/{root}"))));
    assert!(!keys
        .iter()
        .any(|(_, key)| key.contains(&x_uid) || key.contains(&lone_uid)));
    let parents = strings(
        &conn,
        "SELECT DISTINCT parent FROM sync_outbox WHERE entity = ?1 ORDER BY parent",
        "membership",
    );
    let mut expected = vec![m_uid, root];
    expected.sort();
    assert_eq!(parents, expected);
}

/// C13a: `merged` X → M, M lokal mit ausstehendem Löschwunsch (Video
/// `deleted`, Video `withdrawn`, Sammlung); der X-Upsert war quittiert.
#[test]
fn c13a_merge_into_pending_delete_follows_the_wish() {
    for tomb in ["deleted", "withdrawn"] {
        let (_temp, paths) = temp_paths();
        let (m_id, m) = shared_video(&paths, "vidAAAAAAA1");
        let (x_id, x) = shared_video(&paths, "vidBBBBBBB2");
        if tomb == "deleted" {
            storage::delete_video(&paths, m_id).unwrap();
        } else {
            storage::video_set_local_only(&paths, m_id, true).unwrap();
        }
        let mut conn = open(&paths);
        let wish = outbox(&conn);
        let wish_seq: i64 = conn
            .query_row("SELECT seq FROM sync_outbox", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            wish,
            vec![(
                "video".to_string(),
                m.clone(),
                Some(m.clone()),
                None,
                Some(tomb.to_string())
            )]
        );

        apply_states(&mut conn, vec![gone(&x, GoneReason::Merged, Some(&m))]);

        if tomb == "deleted" {
            assert!(storage::get_video(&paths, x_id).unwrap().is_none());
        } else {
            assert_eq!(private_and_published(&conn, x_id), (true, false));
            assert_ne!(video_uid(&conn, x_id), x);
            assert_ne!(video_uid(&conn, x_id), m);
        }
        assert_eq!(outbox(&conn), wish);
        let seq: i64 = conn
            .query_row("SELECT seq FROM sync_outbox", [], |row| row.get(0))
            .unwrap();
        assert_eq!(seq, wish_seq);
        let next = snapshot::snapshot(&mut conn).unwrap();
        let reason = if tomb == "deleted" {
            GoneReason::Deleted
        } else {
            GoneReason::Withdrawn
        };
        assert_eq!(next.upserts, Vec::new());
        assert_eq!(next.deletes.len(), 1);
        assert_eq!(
            next.deletes[0].op,
            Op::VideoGone {
                uid: m.clone(),
                reason
            }
        );
    }

    let (_temp, paths) = temp_paths();
    let m = storage::create_collection(&paths, "M").unwrap();
    let x = storage::create_collection(&paths, "X").unwrap();
    let mut conn = open(&paths);
    push_all(&mut conn);
    let m_uid = text(&conn, "SELECT uid FROM collections WHERE id = ?1", m.id);
    let x_uid = text(&conn, "SELECT uid FROM collections WHERE id = ?1", x.id);
    storage::delete_collection(&paths, m.id).unwrap();
    let wish = outbox(&conn);

    apply_states(
        &mut conn,
        vec![State::CollectionGone {
            uid: x_uid,
            reason: GoneReason::Merged,
            merged_into: Some(m_uid.clone()),
        }],
    );

    assert_eq!(collection_names(&paths), Vec::<String>::new());
    assert_eq!(outbox(&conn), wish);
    let next = snapshot::snapshot(&mut conn).unwrap();
    assert_eq!(next.deletes.len(), 1);
    assert_eq!(next.deletes[0].op, Op::CollectionDelete { uid: m_uid });
    assert_eq!(next.upserts, Vec::new());
}

/// C14: Fremde Runde mit gleichem `created_at` → gleiche Reihenfolge auf
/// beiden Datenbanken.
#[test]
fn c14_foreign_round_with_same_time_sorts_identically() {
    let (_temp_a, a) = temp_paths();
    let (_temp_b, b) = temp_paths();
    let video_a = storage::insert_video(&a, sample_video("vidAAAAAAA1"), false).unwrap();
    let (chat_a, _) = storage::append_chat_turn(
        &a,
        video_a.id,
        None,
        "Chat",
        vec![NewChatMessage::user("A1"), NewChatMessage::assistant("A2")],
        None,
    )
    .unwrap();
    let mut conn_a = open(&a);
    let mut conn_b = open(&b);
    let sent = push_all(&mut conn_a);
    apply_states(&mut conn_b, as_states(&sent));
    let video_b = storage::get_videos(&b).unwrap()[0].id;
    let chat_b = storage::list_chats(&b, video_b).unwrap()[0].id;
    let time = text(
        &conn_a,
        "SELECT created_at FROM chat_messages WHERE chat_id = ?1 LIMIT 1",
        chat_a.id,
    );
    storage::append_chat_turn(
        &b,
        video_b,
        Some(chat_b),
        "Chat",
        vec![NewChatMessage::user("B1"), NewChatMessage::assistant("B2")],
        None,
    )
    .unwrap();
    conn_b
        .execute("UPDATE chat_messages SET created_at = ?1", params![time])
        .unwrap();
    let sent = push_all(&mut conn_b);
    apply_states(&mut conn_a, as_states(&sent));

    let contents = |paths: &AppPaths, chat: i64| -> Vec<String> {
        storage::get_chat_messages(paths, chat)
            .unwrap()
            .into_iter()
            .map(|message| message.content)
            .collect()
    };
    let on_a = contents(&a, chat_a.id);
    assert_eq!(on_a.len(), 4);
    assert_eq!(on_a, contents(&b, chat_b));
}

/// C15: Zwei Summaries gleicher Zeit in entgegengesetzter Importreihenfolge
/// → gleiche „neueste“.
#[test]
fn c15_same_time_summaries_pick_same_newest_in_any_order() {
    let v = uid(1);
    let (low, high) = (uid(0x10), uid(0x20));
    let mut results = Vec::new();
    for order in [[&low, &high], [&high, &low]] {
        let (_temp, paths) = temp_paths();
        let mut conn = open(&paths);
        apply_states(&mut conn, vec![video_state(&v, "vidAAAAAAA1", "V")]);
        for summary in order {
            apply_states(&mut conn, vec![summary_state(summary, &v, T0, summary)]);
        }
        let id = storage::get_videos(&paths).unwrap()[0].id;
        let newest = storage::get_summaries(&paths, id).unwrap()[0]
            .summary
            .clone();
        let current = storage::get_video(&paths, id).unwrap().unwrap().summary;
        results.push((newest, current));
    }
    assert_eq!(results[0], results[1]);
    assert_eq!(results[0].0, high);
    assert_eq!(results[0].1.as_deref(), Some(high.as_str()));
}

/// C16: `summaryUids` `null`, `[]`, bekannt, erst später ankommend.
#[test]
fn c16_summary_uids_map_and_resolve_late_arrivals() {
    let (_temp, paths) = temp_paths();
    let mut conn = open(&paths);
    let v = uid(1);
    let (known, late) = (uid(0x51), uid(0x52));
    apply_states(
        &mut conn,
        vec![
            video_state(&v, "vidAAAAAAA1", "V"),
            summary_state(&known, &v, T0, "bekannt"),
            chat_state(&uid(0xc1), &v, None),
            chat_state(&uid(0xc2), &v, Some(Vec::new())),
            chat_state(&uid(0xc3), &v, Some(vec![known.clone()])),
            chat_state(&uid(0xc4), &v, Some(vec![known.clone(), late.clone()])),
        ],
    );
    let video = storage::get_videos(&paths).unwrap()[0].id;
    let known_id = storage::get_summaries(&paths, video).unwrap()[0].id;
    let options = |conn: &Connection, chat: u32| -> serde_json::Value {
        serde_json::from_str(&text(
            conn,
            "SELECT context_options FROM chats WHERE uid = ?1",
            uid(chat),
        ))
        .unwrap()
    };
    assert_eq!(options(&conn, 0xc1)["summaryIds"], serde_json::Value::Null);
    assert_eq!(options(&conn, 0xc2)["summaryIds"], serde_json::json!([]));
    assert_eq!(
        options(&conn, 0xc3)["summaryIds"],
        serde_json::json!([known_id])
    );
    assert_eq!(
        options(&conn, 0xc4)["summaryIds"],
        serde_json::json!([known_id])
    );
    assert_eq!(
        options(&conn, 0xc4)["pendingSummaryUids"],
        serde_json::json!([late])
    );

    // Beim Push gehen lokale ids und ausstehende uids zusammen hinaus.
    let chat_c4 = uid(0xc4);
    crate::sync::outbox::put(&conn, "chat", &chat_c4, Some(&v), None, None).unwrap();
    let chat_op = snapshot::snapshot(&mut conn)
        .unwrap()
        .upserts
        .into_iter()
        .find_map(|pending| match pending.op {
            Op::Chat { data, .. } => Some(data),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        chat_op.context_options.summary_uids,
        Some(vec![known.clone(), late.clone()])
    );
    conn.execute("DELETE FROM sync_outbox", []).unwrap();

    apply_states(&mut conn, vec![summary_state(&late, &v, T0, "spät")]);
    let late_id: i64 = conn
        .query_row(
            "SELECT id FROM summaries WHERE uid = ?1",
            params![late],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        options(&conn, 0xc4)["summaryIds"],
        serde_json::json!([known_id, late_id])
    );
    assert_eq!(options(&conn, 0xc4).get("pendingSummaryUids"), None);
    assert_eq!(options(&conn, 0xc1)["summaryIds"], serde_json::Value::Null);
    assert_eq!(options(&conn, 0xc2)["summaryIds"], serde_json::json!([]));
}

/// C17: Zwei Sammlungen tauschen im Batch die Namen; eine lokal ausstehende
/// gleichnamige Sammlung → kein Constraint-Fehler, Kollision zurückgestellt.
#[test]
fn c17_collection_name_swap_and_pending_collision() {
    let (_temp, paths) = temp_paths();
    let a = storage::create_collection(&paths, "Alpha").unwrap();
    let b = storage::create_collection(&paths, "Beta").unwrap();
    let (video, v) = shared_video(&paths, "vidAAAAAAA1");
    storage::create_collection(&paths, "Gamma").unwrap();
    let mut conn = open(&paths);
    let (a_uid, b_uid) = (collection_uid(&conn, a.id), collection_uid(&conn, b.id));
    let y = uid(0x99);

    apply_states(
        &mut conn,
        vec![
            collection_state(&a_uid, "Beta"),
            collection_state(&b_uid, "alpha"),
            collection_state(&y, "gamma"),
            membership_state(&v, &y, true),
        ],
    );

    assert_eq!(collection_names(&paths), vec!["alpha", "Beta", "Gamma"]);
    assert_eq!(
        text(&conn, "SELECT name FROM collections WHERE uid = ?1", &a_uid),
        "Beta"
    );
    // „gamma“ und ihre Zuordnung warten in der Inbox.
    assert_eq!(inbox_count(&conn), 2);
    assert!(storage::get_video(&paths, video)
        .unwrap()
        .unwrap()
        .collection_ids
        .is_empty());
}

/// Zurückstellen: Server-Sammlung „KI“ mit Zuordnung trifft auf lokal
/// ausstehende „KI“. Sie wartet über Neustarts und wird eingefügt, sobald die
/// lokale umbenannt (und quittiert) oder gelöscht ist; `merged` löst sie auf.
#[test]
fn deferred_collection_waits_until_conflict_resolves() {
    for variant in ["umbenannt", "gelöscht", "verschmolzen"] {
        let (_temp, paths) = temp_paths();
        let (video, v) = shared_video(&paths, "vidAAAAAAA1");
        let local = storage::create_collection(&paths, "KI").unwrap();
        let y = uid(0x99);
        {
            let mut conn = open(&paths);
            apply_states(
                &mut conn,
                vec![collection_state(&y, "KI"), membership_state(&v, &y, true)],
            );
            assert_eq!(inbox_count(&conn), 2, "{variant}");
        }
        // Neustart: neue Verbindung, Apply ohne neue Seiten.
        let mut conn = open(&paths);
        apply::apply(&mut conn).unwrap();
        assert_eq!(inbox_count(&conn), 2, "{variant}");
        assert_eq!(collection_names(&paths), vec!["KI"]);

        match variant {
            "umbenannt" => {
                // Kein Namenskonflikt mehr, auch wenn die Umbenennung noch
                // aussteht.
                storage::update_collection(&paths, local.id, "Forschung").unwrap();
                apply::apply(&mut conn).unwrap();
            }
            "gelöscht" => {
                storage::delete_collection(&paths, local.id).unwrap();
                apply::apply(&mut conn).unwrap();
            }
            _ => {
                let local_uid = collection_uid(&conn, local.id);
                apply_states(
                    &mut conn,
                    vec![
                        State::CollectionGone {
                            uid: local_uid,
                            reason: GoneReason::Merged,
                            merged_into: Some(y.clone()),
                        },
                        collection_state(&y, "KI"),
                    ],
                );
            }
        }

        assert_eq!(inbox_count(&conn), 0, "{variant}");
        assert_eq!(
            text(&conn, "SELECT name FROM collections WHERE uid = ?1", &y),
            "KI",
            "{variant}"
        );
        let y_id: i64 = conn
            .query_row(
                "SELECT id FROM collections WHERE uid = ?1",
                params![y],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            storage::get_video(&paths, video)
                .unwrap()
                .unwrap()
                .collection_ids,
            vec![y_id],
            "{variant}"
        );
    }
}

/// C20: `video` mit der YouTube-ID einer privaten Kopie → zweite Zeile, die
/// private bleibt unverändert.
#[test]
fn c20_shared_video_next_to_private_copy() {
    let (_temp, paths) = temp_paths();
    let private = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), true).unwrap();
    let mut conn = open(&paths);
    let row = |conn: &Connection| {
        strings(
            conn,
            "SELECT uid || local_only || published || title || updated_at FROM videos WHERE id = ?1",
            private.id,
        )
    };
    let before = row(&conn);

    apply_states(
        &mut conn,
        vec![video_state(&uid(1), "vidAAAAAAA1", "Geteilt")],
    );

    assert_eq!(row(&conn), before);
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM videos WHERE video_id = ?1",
            "vidAAAAAAA1"
        ),
        2
    );
    let shared = strings(
        &conn,
        "SELECT local_only || published FROM videos WHERE uid = ?1",
        &uid(1),
    );
    assert_eq!(shared, vec!["01"]);
}

/// Inbox für C19: Grabstein, lebende Zustände aller Arten und eine
/// zurückgestellte Sammlung.
fn c19_fixture() -> (tempfile::TempDir, AppPaths, i64) {
    let (temp, paths) = temp_paths();
    let (old, old_uid) = shared_video(&paths, "vidAAAAAAA1");
    storage::update_summary(&paths, old, "Alt", None, None, None).unwrap();
    storage::create_collection(&paths, "Lokal").unwrap();
    let mut conn = open(&paths);
    let v = uid(1);
    let (s, c, r, k, y) = (uid(2), uid(3), uid(4), uid(5), uid(6));
    stage(
        &mut conn,
        vec![
            gone(&old_uid, GoneReason::Withdrawn, None),
            video_state(&v, "vidBBBBBBB2", "Neu"),
            summary_state(&s, &v, T0, "Summary"),
            chat_state(&c, &v, Some(vec![s.clone()])),
            round_state(&r, &c, &v, "Runde"),
            collection_state(&k, "KI"),
            membership_state(&v, &k, true),
            collection_state(&y, "lokal"),
        ],
    );
    (temp, paths, old)
}

fn assert_c19_applied(paths: &AppPaths, old: i64) {
    let conn = open(paths);
    assert_eq!(private_and_published(&conn, old), (true, false));
    assert_eq!(titles(paths), vec!["Neu", "Testvideo"]);
    let new = storage::get_videos(paths)
        .unwrap()
        .into_iter()
        .find(|video| video.title == "Neu")
        .unwrap();
    let summary = storage::get_summaries(paths, new.id).unwrap();
    assert_eq!(summary.len(), 1);
    let detail = storage::get_video(paths, new.id).unwrap().unwrap();
    assert_eq!(detail.summary.as_deref(), Some("Summary"));
    let chat = &storage::list_chats(paths, new.id).unwrap()[0];
    assert_eq!(chat.context_options.summary_ids, Some(vec![summary[0].id]));
    assert_eq!(
        storage::get_chat_messages(paths, chat.id).unwrap()[0].content,
        "Runde"
    );
    assert_eq!(collection_names(paths), vec!["KI", "Lokal"]);
    assert_eq!(new.collection_ids.len(), 1);
    assert_eq!(inbox_count(&conn), 1);
}

/// C19 (Apply-Teil): Abbruch an jedem Zwischenstand rollt zurück; die
/// Wiederholung führt zum selben Endstand und ist idempotent.
#[test]
fn c19_apply_abort_at_every_step_rolls_back() {
    let mut checkpoint = 0;
    loop {
        let (_temp, paths, old) = c19_fixture();
        let mut conn = open(&paths);
        let before = dump(&conn);
        match apply::apply_until(&mut conn, Some(checkpoint)) {
            Err(error) => {
                assert_eq!(error, format!("{ABORTED} {checkpoint}"));
                assert_eq!(dump(&conn), before, "Abbruch nach {checkpoint}");
                apply::apply(&mut conn).unwrap();
                assert_c19_applied(&paths, old);
                let applied = dump(&conn);
                apply::apply(&mut conn).unwrap();
                assert_eq!(dump(&conn), applied);
                checkpoint += 1;
            }
            Ok(_) => break,
        }
    }
    // Grabstein, Sammlungen, 5 lebende Zustände, Nacharbeit, Abschluss.
    assert_eq!(checkpoint, 9);
}
