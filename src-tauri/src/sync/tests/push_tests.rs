//! Snapshot, Quittung und Neu abgleichen: C9, C10 und die Regeln aus „Push“.

use rusqlite::params;
use sync_proto::{GoneReason, Op, OpResult, OpStatus};

use super::{
    inbox_count, open, outbox, outbox_keys, push_all, sample_video, stage, temp_paths, text, uid,
    video_state, video_uid,
};
use crate::models::NewChatMessage;
use crate::storage;
use crate::sync::{outbox as sync_outbox, pull, snapshot, state_get};

fn result(status: OpStatus) -> OpResult {
    OpResult {
        status,
        reason: None,
        missing: None,
    }
}

fn published(conn: &rusqlite::Connection, id: i64) -> bool {
    conn.query_row(
        "SELECT published FROM videos WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )
    .unwrap()
}

/// C9: Snapshot, danach eine Änderung in der UI, dann Quittung → der neue
/// Eintrag bleibt; der Snapshot setzt `published`.
#[test]
fn c9_acknowledge_keeps_entry_changed_after_snapshot() {
    let (_temp, paths) = temp_paths();
    let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), false).unwrap();
    let mut conn = open(&paths);
    assert!(!published(&conn, video.id));

    let snapshot = snapshot::snapshot(&mut conn).unwrap();
    assert!(published(&conn, video.id));
    assert_eq!(snapshot.upserts.len(), 1);
    let sent_seq = snapshot.upserts[0].seq;

    storage::update_transcript(&paths, video.id, "[]", None, None).unwrap();
    snapshot::acknowledge(&mut conn, &snapshot.upserts, &[result(OpStatus::Ok)]).unwrap();

    let uid = video_uid(&conn, video.id);
    assert_eq!(outbox_keys(&conn), vec![("video".to_string(), uid)]);
    let seq: i64 = conn
        .query_row("SELECT seq FROM sync_outbox", [], |row| row.get(0))
        .unwrap();
    assert!(seq > sent_seq);
}

/// C10: Video nach dem Snapshot, vor dem Senden gelöscht → `deleted`-Eintrag.
#[test]
fn c10_delete_after_snapshot_writes_tombstone() {
    let (_temp, paths) = temp_paths();
    let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), false).unwrap();
    let mut conn = open(&paths);
    let uid = video_uid(&conn, video.id);
    snapshot::snapshot(&mut conn).unwrap();

    storage::delete_video(&paths, video.id).unwrap();

    assert_eq!(
        outbox(&conn),
        vec![(
            "video".to_string(),
            uid.clone(),
            Some(uid),
            None,
            Some("deleted".to_string())
        )]
    );
}

/// Beide Phasen aus einem Snapshot in Spec-Reihenfolge; `summaryUids` aus
/// lokalen ids und `pendingSummaryUids`; private Upserts werden verworfen;
/// `retry` wird nicht quittiert.
#[test]
fn snapshot_builds_both_phases_and_maps_context() {
    let (_temp, paths) = temp_paths();
    let collection = storage::create_collection(&paths, "KI").unwrap();
    let doomed_collection = storage::create_collection(&paths, "Weg").unwrap();
    let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), false).unwrap();
    let doomed = storage::insert_video(&paths, sample_video("vidBBBBBBB2"), false).unwrap();
    let private = storage::insert_video(&paths, sample_video("vidCCCCCCC3"), true).unwrap();
    storage::update_summary(&paths, video.id, "Bleibt", None, None, None).unwrap();
    storage::update_summary(&paths, video.id, "Weg", None, None, None).unwrap();
    let (chat, _) = storage::append_chat_turn(
        &paths,
        video.id,
        None,
        "Chat",
        vec![
            NewChatMessage::user("Frage"),
            NewChatMessage::assistant("Antwort"),
        ],
        None,
    )
    .unwrap();
    let (doomed_chat, _) = storage::append_chat_turn(
        &paths,
        video.id,
        None,
        "Weg",
        vec![NewChatMessage::user("x")],
        None,
    )
    .unwrap();
    storage::set_video_collections(&paths, video.id, vec![collection.id]).unwrap();
    let mut conn = open(&paths);
    let summaries = storage::get_summaries(&paths, video.id).unwrap();
    let (kept, removed) = (&summaries[1], &summaries[0]);
    let kept_uid = text(&conn, "SELECT uid FROM summaries WHERE id = ?1", kept.id);
    let pending_uid = uid(7);
    conn.execute(
        "UPDATE chats SET context_options = ?1 WHERE id = ?2",
        params![
            serde_json::json!({"transcript": false, "summaryIds": [kept.id, 999],
                               "pendingSummaryUids": [pending_uid]})
            .to_string(),
            chat.id
        ],
    )
    .unwrap();
    conn.execute(
        "UPDATE videos SET published = 1 WHERE id = ?1",
        params![doomed.id],
    )
    .unwrap();
    let doomed_uid = video_uid(&conn, doomed.id);
    storage::delete_summary(&paths, removed.id).unwrap();
    storage::delete_chat(&paths, doomed_chat.id).unwrap();
    storage::delete_collection(&paths, doomed_collection.id).unwrap();
    storage::delete_video(&paths, doomed.id).unwrap();
    // Zweite Sicherung: ein (fehlerhafter) Eintrag für das private Video.
    let private_uid = video_uid(&conn, private.id);
    sync_outbox::put(&conn, "video", &private_uid, Some(&private_uid), None, None).unwrap();

    let snapshot = snapshot::snapshot(&mut conn).unwrap();

    let kinds = |ops: &[snapshot::Pending]| -> Vec<&'static str> {
        ops.iter()
            .map(|pending| match &pending.op {
                Op::VideoGone { .. } => "videoGone",
                Op::CollectionDelete { .. } => "collectionDelete",
                Op::ChatDelete { .. } => "chatDelete",
                Op::SummaryDelete { .. } => "summaryDelete",
                Op::Collection { .. } => "collection",
                Op::Video { .. } => "video",
                Op::Summary(_) => "summary",
                Op::Chat { .. } => "chat",
                Op::Round(_) => "round",
                Op::Membership { .. } => "membership",
            })
            .collect()
    };
    assert_eq!(
        kinds(&snapshot.deletes),
        vec![
            "videoGone",
            "collectionDelete",
            "chatDelete",
            "summaryDelete"
        ]
    );
    assert_eq!(
        kinds(&snapshot.upserts),
        vec![
            "collection",
            "video",
            "summary",
            "chat",
            "round",
            "membership"
        ]
    );
    let chat_op = snapshot
        .upserts
        .iter()
        .find_map(|pending| match &pending.op {
            Op::Chat { data, .. } => Some(data.clone()),
            _ => None,
        })
        .unwrap();
    assert!(!chat_op.context_options.transcript);
    assert_eq!(
        chat_op.context_options.summary_uids,
        Some(vec![kept_uid, pending_uid])
    );
    let round = snapshot
        .upserts
        .iter()
        .find_map(|pending| match &pending.op {
            Op::Round(data) => Some(data.clone()),
            _ => None,
        })
        .unwrap();
    let contents: Vec<&str> = round.messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(contents, vec!["Frage", "Antwort"]);
    assert_eq!(
        snapshot.deletes[0].op,
        Op::VideoGone {
            uid: doomed_uid,
            reason: GoneReason::Deleted
        }
    );
    // Der private Eintrag ist verworfen und entfernt.
    assert!(!outbox_keys(&conn).contains(&("video".to_string(), private_uid)));

    // retry bleibt, ok/rejected werden quittiert.
    let sent: Vec<_> = snapshot.deletes.clone();
    let results = vec![
        result(OpStatus::Ok),
        result(OpStatus::Rejected),
        result(OpStatus::Retry),
        result(OpStatus::Ok),
    ];
    snapshot::acknowledge(&mut conn, &sent, &results).unwrap();
    let remaining: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT seq FROM sync_outbox").unwrap();
        let rows = stmt.query_map([], |row| row.get(0)).unwrap();
        rows.map(Result::unwrap).collect()
    };
    assert!(remaining.contains(&sent[2].seq));
    for index in [0, 1, 3] {
        assert!(!remaining.contains(&sent[index].seq));
    }
    let error = snapshot::acknowledge(&mut conn, &sent, &results[..1]).unwrap_err();
    assert!(error.contains("passt nicht"), "{error}");
}

/// Neu abgleichen: Inbox leer, Cursor 0, Einträge für alle lebenden Zeilen,
/// Löschwünsche bleiben, `published` unverändert, neue Datensatz-ID.
#[test]
fn rebaseline_reseeds_and_keeps_delete_wishes() {
    let (_temp, paths) = temp_paths();
    let collection = storage::create_collection(&paths, "KI").unwrap();
    let video = storage::insert_video(&paths, sample_video("vidAAAAAAA1"), false).unwrap();
    let gone = storage::insert_video(&paths, sample_video("vidBBBBBBB2"), false).unwrap();
    storage::insert_video(&paths, sample_video("vidCCCCCCC3"), true).unwrap();
    storage::set_video_collections(&paths, video.id, vec![collection.id]).unwrap();
    let mut conn = open(&paths);
    push_all(&mut conn);
    storage::delete_video(&paths, gone.id).unwrap();
    let wish = outbox(&conn);
    assert_eq!(wish.len(), 1);
    stage(
        &mut conn,
        vec![video_state(&uid(9), "vidZZZZZZZ9", "Fremd")],
    );
    assert_eq!(inbox_count(&conn), 1);

    pull::rebaseline(&mut conn, "neuer-datensatz").unwrap();

    assert_eq!(inbox_count(&conn), 0);
    assert_eq!(pull::cursor(&conn).unwrap(), 0);
    assert_eq!(
        state_get(&conn, "dataset_id").unwrap().as_deref(),
        Some("neuer-datensatz")
    );
    let uid = video_uid(&conn, video.id);
    let collection_uid = text(
        &conn,
        "SELECT uid FROM collections WHERE id = ?1",
        collection.id,
    );
    let mut expected = vec![
        ("video".to_string(), uid.clone()),
        ("collection".to_string(), collection_uid.clone()),
        ("membership".to_string(), format!("{uid}/{collection_uid}")),
        (wish[0].0.clone(), wish[0].1.clone()),
    ];
    expected.sort();
    assert_eq!(outbox_keys(&conn), expected);
    assert!(outbox(&conn).contains(&wish[0]));
    assert!(published(&conn, video.id));
}
