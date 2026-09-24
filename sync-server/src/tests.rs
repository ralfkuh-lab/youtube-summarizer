//! Referenzfälle S1–S23 aus `docs/spec-sync.md` ohne HTTP (S18/S20 folgen
//! mit dem HTTP-Teil).

use crate::db::{self, pull_page};
use crate::merge::push;
use crate::Error;
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use sync_proto::*;

const T1: &str = "2026-09-24T10:00:00.000Z";
const T15: &str = "2026-09-24T10:30:00.000Z";
const T2: &str = "2026-09-24T11:00:00.000Z";
const T3: &str = "2026-09-24T12:00:00.000Z";
const YT: &str = "dQw4w9WgXcQ";
const DEV_A: i64 = 1;
const DEV_B: i64 = 2;

fn uid(n: u32) -> String {
    format!("{n:032x}")
}

fn db() -> Connection {
    db::open(Path::new(":memory:")).unwrap()
}

fn run(conn: &mut Connection, dev: i64, ops: Vec<Op>) -> Vec<OpResult> {
    let dataset = db::dataset_id(conn).unwrap();
    push(conn, dev, &dataset, &ops).unwrap()
}

fn run_ok(conn: &mut Connection, dev: i64, ops: Vec<Op>) {
    for (index, result) in run(conn, dev, ops).into_iter().enumerate() {
        assert_eq!(result.status, OpStatus::Ok, "Operation {index}: {result:?}");
    }
}

fn pull_all(conn: &mut Connection, since: i64) -> Vec<ServerState> {
    let dataset = db::dataset_id(conn).unwrap();
    let page = pull_page(conn, &dataset, since, usize::MAX, usize::MAX).unwrap();
    assert!(!page.more);
    page.states
}

/// Aktueller Zustand eines Schlüssels mit seiner `seq`.
fn state(conn: &mut Connection, entity: &str, key: &str) -> Option<(i64, State)> {
    pull_all(conn, 0)
        .into_iter()
        .find(|item| item.state.entity_key() == (entity, key.to_owned()))
        .map(|item| (item.seq, item.state))
}

fn seq(conn: &mut Connection, entity: &str, key: &str) -> i64 {
    state(conn, entity, key).expect("Zustand fehlt").0
}

fn video_data(conn: &mut Connection, uid: &str) -> VideoData {
    match state(conn, "video", uid) {
        Some((_, State::Video(data))) => data,
        other => panic!("kein lebendes Video {uid}: {other:?}"),
    }
}

fn live_videos(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM videos", [], |row| row.get(0))
        .unwrap()
}

fn vdata(uid: &str, title: &str) -> VideoData {
    VideoData {
        uid: uid.into(),
        youtube_id: YT.into(),
        url: format!("https://www.youtube.com/watch?v={YT}"),
        title: title.into(),
        thumbnail_url: "https://i.ytimg.com/x.jpg".into(),
        thumbnail_data: None,
        transcript: None,
        chapters: None,
        published_at: None,
        description: None,
        transcript_error: None,
        created_at: T1.into(),
    }
}

fn video(data: VideoData, at: &str) -> Op {
    Op::Video {
        data,
        changed_at: at.into(),
    }
}

fn gone(uid: &str, reason: GoneReason) -> Op {
    Op::VideoGone {
        uid: uid.into(),
        reason,
    }
}

fn summary(uid: &str, video_uid: &str) -> Op {
    Op::Summary(SummaryData {
        uid: uid.into(),
        video_uid: video_uid.into(),
        created_at: T1.into(),
        summary: format!("Zusammenfassung {uid}"),
        provider: None,
        model: None,
        options: None,
    })
}

fn chat_with(uid: &str, video_uid: &str, title: &str, summary_uids: Option<Vec<String>>) -> Op {
    Op::Chat {
        data: ChatData {
            uid: uid.into(),
            video_uid: video_uid.into(),
            title: title.into(),
            created_at: T1.into(),
            updated_at: T1.into(),
            context_options: ContextOptions {
                transcript: true,
                summary_uids,
            },
        },
        changed_at: T1.into(),
    }
}

fn chat(uid: &str, video_uid: &str) -> Op {
    chat_with(uid, video_uid, "Chat", None)
}

fn round(uid: &str, chat_uid: &str, video_uid: &str) -> Op {
    Op::Round(RoundData {
        uid: uid.into(),
        chat_uid: chat_uid.into(),
        video_uid: video_uid.into(),
        created_at: T1.into(),
        messages: vec![RoundMessage {
            role: "user".into(),
            content: "Frage".into(),
            tool_calls: None,
            tool_call_id: None,
            provider: None,
            model: None,
        }],
    })
}

fn collection(uid: &str, name: &str, at: &str) -> Op {
    Op::Collection {
        data: CollectionData {
            uid: uid.into(),
            name: name.into(),
            created_at: T1.into(),
        },
        changed_at: at.into(),
    }
}

fn membership(video_uid: &str, collection_uid: &str, present: bool, at: &str) -> Op {
    Op::Membership {
        video_uid: video_uid.into(),
        collection_uid: collection_uid.into(),
        present,
        changed_at: at.into(),
    }
}

fn assert_rejected(result: &OpResult) {
    assert_eq!(result.status, OpStatus::Rejected, "{result:?}");
}

#[test]
fn s01_new_video_lives_and_is_pulled() {
    let mut conn = db();
    let v = uid(1);
    run_ok(&mut conn, DEV_A, vec![video(vdata(&v, "Titel"), T1)]);
    let states = pull_all(&mut conn, 0);
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].state, State::Video(vdata(&v, "Titel")));
}

#[test]
fn s02_older_title_loses_and_video_is_echoed() {
    let mut conn = db();
    let v = uid(1);
    run_ok(&mut conn, DEV_A, vec![video(vdata(&v, "neu"), T2)]);
    let before = seq(&mut conn, "video", &v);
    run_ok(&mut conn, DEV_B, vec![video(vdata(&v, "alt"), T1)]);
    assert_eq!(video_data(&mut conn, &v).title, "neu");
    assert!(seq(&mut conn, "video", &v) > before);
}

#[test]
fn s03_newer_video_without_transcript_keeps_it() {
    let mut conn = db();
    let v = uid(1);
    let with = VideoData {
        transcript: Some("[1]".into()),
        ..vdata(&v, "alt")
    };
    run_ok(&mut conn, DEV_A, vec![video(with, T1)]);
    run_ok(&mut conn, DEV_B, vec![video(vdata(&v, "neu"), T2)]);
    let data = video_data(&mut conn, &v);
    assert_eq!(data.transcript.as_deref(), Some("[1]"));
    assert_eq!(data.title, "neu");
}

#[test]
fn s04_older_transcript_fills_empty_field_and_title_stays() {
    let mut conn = db();
    let v = uid(1);
    run_ok(
        &mut conn,
        DEV_A,
        vec![video(
            VideoData {
                transcript_error: Some("keine Untertitel".into()),
                ..vdata(&v, "neu")
            },
            T2,
        )],
    );
    let older = VideoData {
        transcript: Some("[1]".into()),
        description: Some("Beschreibung".into()),
        ..vdata(&v, "alt")
    };
    run_ok(&mut conn, DEV_B, vec![video(older, T1)]);
    let data = video_data(&mut conn, &v);
    assert_eq!(data.transcript.as_deref(), Some("[1]"));
    assert_eq!(data.description.as_deref(), Some("Beschreibung"));
    assert_eq!(data.title, "neu");
    assert_eq!(data.transcript_error, None);
}

#[test]
fn fill_fields_use_lww_between_two_values_and_keep_winner_stamp() {
    let mut conn = db();
    let v = uid(1);
    let with = |text: &str| VideoData {
        description: Some(text.into()),
        ..vdata(&v, "T")
    };
    run_ok(&mut conn, DEV_A, vec![video(vdata(&v, "T"), T3)]);
    // Leeres Feld wird aus einem älteren Datensatz gefüllt (Stempel T1) …
    run_ok(&mut conn, DEV_B, vec![video(with("alt"), T1)]);
    // … und von einem jüngeren anderen Wert (T2) nach LWW ersetzt,
    run_ok(&mut conn, DEV_B, vec![video(with("mittel"), T2)]);
    assert_eq!(
        video_data(&mut conn, &v).description.as_deref(),
        Some("mittel")
    );
    // nicht aber von einem älteren.
    run_ok(&mut conn, DEV_A, vec![video(with("uralt"), T15)]);
    assert_eq!(
        video_data(&mut conn, &v).description.as_deref(),
        Some("mittel")
    );
}

#[test]
fn transcript_error_follows_lww_until_a_transcript_exists() {
    let mut conn = db();
    let v = uid(1);
    let error = |text: &str| VideoData {
        transcript_error: Some(text.into()),
        ..vdata(&v, "T")
    };
    run_ok(&mut conn, DEV_A, vec![video(error("alt"), T1)]);
    run_ok(&mut conn, DEV_A, vec![video(error("neu"), T2)]);
    assert_eq!(
        video_data(&mut conn, &v).transcript_error.as_deref(),
        Some("neu")
    );
}

#[test]
fn s05_summary_after_delete_is_rejected_and_gone_is_echoed() {
    let mut conn = db();
    let (v, u) = (uid(1), uid(2));
    run_ok(
        &mut conn,
        DEV_A,
        vec![video(vdata(&v, "T"), T1), summary(&u, &v)],
    );
    run_ok(
        &mut conn,
        DEV_A,
        vec![Op::SummaryDelete {
            uid: u.clone(),
            video_uid: v.clone(),
        }],
    );
    let before = seq(&mut conn, "summary", &u);
    let results = run(&mut conn, DEV_B, vec![summary(&u, &v)]);
    assert_rejected(&results[0]);
    let (after, state) = state(&mut conn, "summary", &u).unwrap();
    assert_eq!(state, State::SummaryGone { uid: u.clone() });
    assert!(after > before);
}

#[test]
fn s06_video_and_child_with_old_uid_after_delete_are_rejected() {
    let mut conn = db();
    let (v, s) = (uid(1), uid(2));
    run_ok(&mut conn, DEV_A, vec![video(vdata(&v, "T"), T1)]);
    run_ok(&mut conn, DEV_A, vec![gone(&v, GoneReason::Deleted)]);
    let before = seq(&mut conn, "video", &v);
    let results = run(
        &mut conn,
        DEV_B,
        vec![video(vdata(&v, "T"), T2), summary(&s, &v)],
    );
    assert_rejected(&results[0]);
    assert_rejected(&results[1]);
    let (after, state) = state(&mut conn, "video", &v).unwrap();
    assert!(matches!(
        state,
        State::VideoGone {
            reason: GoneReason::Deleted,
            ..
        }
    ));
    assert!(after > before);
    assert!(state_of(&mut conn, "summary", &s).is_none());
}

fn state_of(conn: &mut Connection, entity: &str, key: &str) -> Option<State> {
    state(conn, entity, key).map(|(_, state)| state)
}

#[test]
fn s07_new_uid_with_same_youtube_id_after_delete_lives() {
    let mut conn = db();
    let (old, new) = (uid(1), uid(2));
    run_ok(&mut conn, DEV_A, vec![video(vdata(&old, "T"), T1)]);
    run_ok(&mut conn, DEV_A, vec![gone(&old, GoneReason::Deleted)]);
    run_ok(&mut conn, DEV_A, vec![video(vdata(&new, "neu"), T2)]);
    assert_eq!(video_data(&mut conn, &new).title, "neu");
    assert_eq!(live_videos(&conn), 1);
}

#[test]
fn s08_same_youtube_id_on_two_devices_merges_into_one_root() {
    let mut conn = db();
    let (a, b) = (uid(1), uid(2));
    let (sa, sb, cb, rb) = (uid(11), uid(12), uid(13), uid(14));
    run_ok(
        &mut conn,
        DEV_A,
        vec![video(vdata(&a, "A"), T1), summary(&sa, &a)],
    );
    let child_before = seq(&mut conn, "summary", &sa);
    run_ok(
        &mut conn,
        DEV_B,
        vec![
            video(vdata(&b, "B"), T2),
            summary(&sb, &b),
            chat(&cb, &b),
            round(&rb, &cb, &b),
        ],
    );
    assert_eq!(live_videos(&conn), 1);
    assert_eq!(
        state_of(&mut conn, "video", &b),
        Some(State::VideoGone {
            uid: b.clone(),
            reason: GoneReason::Merged,
            merged_into: Some(a.clone()),
        })
    );
    assert_eq!(video_data(&mut conn, &a).title, "B");
    for (entity, key) in [
        ("summary", &sa),
        ("summary", &sb),
        ("chat", &cb),
        ("round", &rb),
    ] {
        let owner = match state_of(&mut conn, entity, key) {
            Some(State::Summary(data)) => data.video_uid,
            Some(State::Chat(data)) => data.video_uid,
            Some(State::Round(data)) => data.video_uid,
            other => panic!("{entity}/{key}: {other:?}"),
        };
        assert_eq!(owner, a, "{entity}/{key} hängt nicht an der Wurzel");
    }
    // Teilbaum der Wurzel wurde beim Verschmelzen geechot.
    assert!(seq(&mut conn, "summary", &sa) > child_before);
}

#[test]
fn s09_withdrawn_removes_children_and_rejects_later_children() {
    let mut conn = db();
    let (v, s, c, r, k) = (uid(1), uid(2), uid(3), uid(4), uid(5));
    run_ok(
        &mut conn,
        DEV_A,
        vec![
            collection(&k, "KI", T1),
            video(vdata(&v, "T"), T1),
            summary(&s, &v),
            chat(&c, &v),
            round(&r, &c, &v),
            membership(&v, &k, true, T1),
        ],
    );
    run_ok(&mut conn, DEV_A, vec![gone(&v, GoneReason::Withdrawn)]);
    assert!(matches!(
        state_of(&mut conn, "video", &v),
        Some(State::VideoGone {
            reason: GoneReason::Withdrawn,
            ..
        })
    ));
    for (entity, key) in [
        ("summary", s.clone()),
        ("chat", c.clone()),
        ("round", r.clone()),
        ("membership", membership_key(&v, &k)),
    ] {
        assert!(state_of(&mut conn, entity, &key).is_none(), "{entity}");
    }
    let results = run(
        &mut conn,
        DEV_B,
        vec![
            summary(&uid(6), &v),
            chat(&uid(7), &v),
            round(&uid(8), &c, &v),
            membership(&v, &k, true, T2),
        ],
    );
    results.iter().for_each(assert_rejected);
}

#[test]
fn s10_gone_for_unknown_uid_blocks_later_video() {
    let mut conn = db();
    let v = uid(1);
    run_ok(&mut conn, DEV_A, vec![gone(&v, GoneReason::Deleted)]);
    let results = run(&mut conn, DEV_B, vec![video(vdata(&v, "spät"), T2)]);
    assert_rejected(&results[0]);
    assert_eq!(live_videos(&conn), 0);
    assert!(matches!(
        state_of(&mut conn, "video", &v),
        Some(State::VideoGone {
            reason: GoneReason::Deleted,
            ..
        })
    ));
}

#[test]
fn s11_withdrawn_then_new_uid_with_new_and_old_child_uids() {
    let mut conn = db();
    let (old, new, s_old, s_new) = (uid(1), uid(2), uid(3), uid(4));
    run_ok(
        &mut conn,
        DEV_A,
        vec![video(vdata(&old, "T"), T1), summary(&s_old, &old)],
    );
    run_ok(&mut conn, DEV_A, vec![gone(&old, GoneReason::Withdrawn)]);
    run_ok(
        &mut conn,
        DEV_A,
        vec![
            video(vdata(&new, "T"), T2),
            summary(&s_new, &new),
            summary(&s_old, &new),
        ],
    );
    assert_eq!(live_videos(&conn), 1);
    for s in [&s_new, &s_old] {
        match state_of(&mut conn, "summary", s) {
            Some(State::Summary(data)) => assert_eq!(data.video_uid, new),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn s12_new_collection_with_same_name_key_merges() {
    let mut conn = db();
    let (v, y, x) = (uid(1), uid(2), uid(3));
    run_ok(
        &mut conn,
        DEV_A,
        vec![video(vdata(&v, "T"), T1), collection(&y, "KI", T1)],
    );
    run_ok(
        &mut conn,
        DEV_B,
        vec![collection(&x, "ki", T2), membership(&v, &x, true, T2)],
    );
    assert_eq!(
        state_of(&mut conn, "collection", &x),
        Some(State::CollectionGone {
            uid: x.clone(),
            reason: GoneReason::Merged,
            merged_into: Some(y.clone()),
        })
    );
    assert_eq!(
        state_of(&mut conn, "membership", &membership_key(&v, &y)),
        Some(State::Membership {
            video_uid: v.clone(),
            collection_uid: y.clone(),
            present: true,
        })
    );
    assert!(matches!(
        state_of(&mut conn, "collection", &y),
        Some(State::Collection(CollectionData { name, .. })) if name == "KI"
    ));
}

#[test]
fn s13_stale_rename_to_existing_name_does_not_merge() {
    let mut conn = db();
    let (x, y) = (uid(1), uid(2));
    run_ok(
        &mut conn,
        DEV_A,
        vec![collection(&y, "KI", T1), collection(&x, "Forschung", T3)],
    );
    run_ok(&mut conn, DEV_B, vec![collection(&x, "KI", T2)]);
    assert!(matches!(
        state_of(&mut conn, "collection", &x),
        Some(State::Collection(CollectionData { name, .. })) if name == "Forschung"
    ));
    assert!(matches!(
        state_of(&mut conn, "collection", &y),
        Some(State::Collection(_))
    ));
}

#[test]
fn winning_rename_merges_memberships_and_echoes_root_with_all_memberships() {
    let mut conn = db();
    let (v1, v2, v3, x, y) = (uid(1), uid(2), uid(3), uid(4), uid(5));
    let other = |n| VideoData {
        youtube_id: format!("abcdefghij{n}"),
        ..vdata(&uid(n), "T")
    };
    run_ok(
        &mut conn,
        DEV_A,
        vec![
            video(vdata(&v1, "T"), T1),
            video(other(2), T1),
            video(other(3), T1),
            collection(&y, "KI", T1),
            collection(&x, "Forschung", T1),
            // Paar (v1): gleicher Zeitstempel, present gewinnt.
            membership(&v1, &y, false, T2),
            membership(&v1, &x, true, T2),
            // Paar (v2): jüngerer Wert gewinnt.
            membership(&v2, &y, true, T1),
            membership(&v2, &x, false, T2),
            // v3 nur an der Wurzel: wird trotzdem geechot.
            membership(&v3, &y, true, T1),
        ],
    );
    let v3_before = seq(&mut conn, "membership", &membership_key(&v3, &y));
    run_ok(&mut conn, DEV_B, vec![collection(&x, "ki", T3)]);
    let present =
        |conn: &mut Connection, v: &str| match state_of(conn, "membership", &membership_key(v, &y))
        {
            Some(State::Membership { present, .. }) => present,
            other => panic!("{other:?}"),
        };
    assert!(present(&mut conn, &v1));
    assert!(!present(&mut conn, &v2));
    assert!(state_of(&mut conn, "membership", &membership_key(&v1, &x)).is_none());
    assert!(seq(&mut conn, "membership", &membership_key(&v3, &y)) > v3_before);
    assert!(matches!(
        state_of(&mut conn, "collection", &y),
        Some(State::Collection(CollectionData { name, .. })) if name == "KI"
    ));
}

#[test]
fn s14_alias_chains_are_flattened() {
    let mut conn = db();
    let (v, x, y, z) = (uid(1), uid(2), uid(3), uid(4));
    run_ok(
        &mut conn,
        DEV_A,
        vec![
            video(vdata(&v, "T"), T1),
            collection(&x, "a", T1),
            collection(&y, "b", T1),
            collection(&z, "c", T1),
        ],
    );
    run_ok(&mut conn, DEV_A, vec![collection(&x, "b", T2)]); // X → Y
    run_ok(&mut conn, DEV_A, vec![collection(&y, "c", T2)]); // Y → Z
    run_ok(&mut conn, DEV_B, vec![membership(&v, &x, true, T3)]);
    assert!(matches!(
        state_of(&mut conn, "membership", &membership_key(&v, &z)),
        Some(State::Membership { present: true, .. })
    ));
    assert!(matches!(
        state_of(&mut conn, "collection", &x),
        Some(State::CollectionGone { merged_into: Some(root), .. }) if root == z
    ));
}

#[test]
fn s15_membership_uses_lww_and_is_echoed() {
    let mut conn = db();
    let (v, k) = (uid(1), uid(2));
    let key = membership_key(&v, &k);
    run_ok(
        &mut conn,
        DEV_A,
        vec![
            video(vdata(&v, "T"), T1),
            collection(&k, "KI", T1),
            membership(&v, &k, true, T1),
        ],
    );
    run_ok(&mut conn, DEV_A, vec![membership(&v, &k, false, T2)]);
    let before = seq(&mut conn, "membership", &key);
    run_ok(&mut conn, DEV_B, vec![membership(&v, &k, true, T15)]);
    let (after, state) = state(&mut conn, "membership", &key).unwrap();
    assert!(matches!(state, State::Membership { present: false, .. }));
    assert!(after > before);
}

#[test]
fn lww_tie_breaks_by_higher_device_id() {
    let mut conn = db();
    let v = uid(1);
    run_ok(&mut conn, DEV_B, vec![video(vdata(&v, "B"), T1)]);
    run_ok(&mut conn, DEV_A, vec![video(vdata(&v, "A"), T1)]);
    assert_eq!(video_data(&mut conn, &v).title, "B");
}

#[test]
fn s16_round_for_gone_chat_is_rejected_with_chat_gone_echo() {
    let mut conn = db();
    let (v, c) = (uid(1), uid(2));
    run_ok(
        &mut conn,
        DEV_A,
        vec![video(vdata(&v, "T"), T1), chat(&c, &v)],
    );
    run_ok(
        &mut conn,
        DEV_A,
        vec![Op::ChatDelete {
            uid: c.clone(),
            video_uid: v.clone(),
        }],
    );
    let before = seq(&mut conn, "chat", &c);
    let results = run(&mut conn, DEV_B, vec![round(&uid(3), &c, &v)]);
    assert_rejected(&results[0]);
    let (after, state) = state(&mut conn, "chat", &c).unwrap();
    assert_eq!(state, State::ChatGone { uid: c.clone() });
    assert!(after > before);
    assert!(state_of(&mut conn, "round", &uid(3)).is_none());
}

#[test]
fn s17_unknown_parents_answer_retry_with_missing() {
    let mut conn = db();
    let (v, unknown_video, unknown_chat) = (uid(1), uid(2), uid(3));
    run_ok(&mut conn, DEV_A, vec![video(vdata(&v, "T"), T1)]);
    let states_before = pull_all(&mut conn, 0).len();
    let results = run(
        &mut conn,
        DEV_A,
        vec![
            summary(&uid(4), &unknown_video),
            round(&uid(5), &unknown_chat, &v),
        ],
    );
    assert_eq!(results[0].status, OpStatus::Retry);
    assert_eq!(
        results[0].missing.as_deref(),
        Some(format!("video/{unknown_video}").as_str())
    );
    assert_eq!(results[1].status, OpStatus::Retry);
    assert_eq!(
        results[1].missing.as_deref(),
        Some(format!("chat/{unknown_chat}").as_str())
    );
    assert_eq!(pull_all(&mut conn, 0).len(), states_before);
}

#[test]
fn s19_foreign_summary_uids_are_removed_from_chat() {
    let mut conn = db();
    let (v1, v2, s1, s2, pending, c) = (uid(1), uid(2), uid(3), uid(4), uid(5), uid(6));
    run_ok(
        &mut conn,
        DEV_A,
        vec![
            video(vdata(&v1, "T"), T1),
            video(
                VideoData {
                    youtube_id: "abcdefghij2".into(),
                    ..vdata(&v2, "T")
                },
                T1,
            ),
            summary(&s1, &v1),
            summary(&s2, &v2),
            chat_with(
                &c,
                &v1,
                "Chat",
                Some(vec![s1.clone(), s2.clone(), pending.clone()]),
            ),
        ],
    );
    match state_of(&mut conn, "chat", &c) {
        Some(State::Chat(data)) => {
            assert_eq!(data.context_options.summary_uids, Some(vec![s1, pending]))
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn chat_head_uses_lww() {
    let mut conn = db();
    let (v, c) = (uid(1), uid(2));
    run_ok(&mut conn, DEV_A, vec![video(vdata(&v, "T"), T1)]);
    let titled = |title: &str, at: &str| match chat_with(&c, &v, title, None) {
        Op::Chat { data, .. } => Op::Chat {
            data,
            changed_at: at.into(),
        },
        _ => unreachable!(),
    };
    run_ok(&mut conn, DEV_A, vec![titled("neu", T2)]);
    run_ok(&mut conn, DEV_B, vec![titled("alt", T1)]);
    assert!(matches!(
        state_of(&mut conn, "chat", &c),
        Some(State::Chat(ChatData { title, .. })) if title == "neu"
    ));
}

#[test]
fn s21_pull_pages_repeat_changed_keys_and_keep_next_monotonic() {
    let mut conn = db();
    let dataset = db::dataset_id(&conn).unwrap();
    let (a, b, c) = (uid(1), uid(2), uid(3));
    let other = |uid: &str, n: u32| VideoData {
        youtube_id: format!("abcdefghij{n}"),
        ..vdata(uid, "T")
    };
    run_ok(
        &mut conn,
        DEV_A,
        vec![
            video(other(&a, 1), T1),
            video(other(&b, 2), T1),
            video(other(&c, 3), T1),
        ],
    );
    let page1 = pull_page(&mut conn, &dataset, 0, 2, usize::MAX).unwrap();
    let keys = |page: &PullResponse| -> Vec<String> {
        page.states.iter().map(|s| s.state.entity_key().1).collect()
    };
    assert_eq!(keys(&page1), vec![a.clone(), b.clone()]);
    assert!(page1.more);
    // A ändert sich zwischen den Seiten.
    run_ok(&mut conn, DEV_A, vec![video(other(&a, 1), T2)]);
    let page2 = pull_page(&mut conn, &dataset, page1.next, 2, usize::MAX).unwrap();
    assert_eq!(keys(&page2), vec![c.clone(), a.clone()]);
    assert!(!page2.more);
    assert!(page2.next > page1.next);
    let empty = pull_page(&mut conn, &dataset, page2.next, 2, usize::MAX).unwrap();
    assert!(empty.states.is_empty());
    assert_eq!(empty.next, page2.next);
    assert!(!empty.more);
    // Byte-Grenze: mindestens ein Zustand je Seite.
    let tiny = pull_page(&mut conn, &dataset, 0, 500, 1).unwrap();
    assert_eq!(tiny.states.len(), 1);
    assert!(tiny.more);
}

fn temp_dir(name: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "sync-server-test-{}-{}-{name}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn s22_backup_once_a_day_missing_dir_and_leftover_tmp() {
    let root = temp_dir("backup");
    let mut conn = db::open(&root.join("sync.db")).unwrap();
    let dir = root.join("backups");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("2026-09-23.db.tmp"), b"Rest").unwrap();
    std::fs::write(dir.join("notizen.db"), b"fremd").unwrap();

    assert!(db::backup(&conn, &dir, "2026-09-24").unwrap());
    assert!(!db::backup(&conn, &dir, "2026-09-24").unwrap());
    assert_eq!(file_names(&dir), vec!["2026-09-24.db", "notizen.db"]);
    let copy = Connection::open(dir.join("2026-09-24.db")).unwrap();
    assert!(copy
        .query_row(
            "SELECT value FROM meta WHERE key = 'dataset_id'",
            [],
            |row| { row.get::<_, String>(0) }
        )
        .is_ok());

    // Aufbewahrung: nach Erfolg die ältesten über 7 hinaus löschen.
    for day in 25..=31 {
        db::backup(&conn, &dir, &format!("2026-09-{day}")).unwrap();
    }
    let names = file_names(&dir);
    assert_eq!(names.len(), 8, "{names:?}");
    assert!(!names.contains(&"2026-09-24.db".to_owned()));
    assert!(names.contains(&"notizen.db".to_owned()));

    // Verzeichnis fehlt: Fehler, protokolliert; der Dienst arbeitet weiter.
    let missing = root.join("fehlt");
    assert!(matches!(
        db::backup(&conn, &missing, "2026-10-01"),
        Err(Error::Io(_))
    ));
    db::backup_logged(&conn, &missing, "2026-10-01");
    run_ok(&mut conn, DEV_A, vec![video(vdata(&uid(1), "T"), T1)]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn s23_deleting_unknown_children_stores_gone_and_blocks_later_creation() {
    let mut conn = db();
    let (v, s, c, k) = (uid(1), uid(2), uid(3), uid(4));
    let results = run(
        &mut conn,
        DEV_A,
        vec![
            Op::SummaryDelete {
                uid: s.clone(),
                video_uid: v.clone(),
            },
            Op::ChatDelete {
                uid: c.clone(),
                video_uid: v.clone(),
            },
            Op::CollectionDelete { uid: k.clone() },
        ],
    );
    assert!(results.iter().all(|result| result.status == OpStatus::Ok));
    let results = run(
        &mut conn,
        DEV_B,
        vec![
            video(vdata(&v, "T"), T1),
            summary(&s, &v),
            chat(&c, &v),
            collection(&k, "KI", T1),
        ],
    );
    assert_eq!(results[0].status, OpStatus::Ok);
    results[1..].iter().for_each(assert_rejected);
    assert_eq!(
        state_of(&mut conn, "summary", &s),
        Some(State::SummaryGone { uid: s.clone() })
    );
    assert_eq!(
        state_of(&mut conn, "chat", &c),
        Some(State::ChatGone { uid: c.clone() })
    );
}

#[test]
fn first_gone_reason_stays() {
    let mut conn = db();
    let v = uid(1);
    run_ok(&mut conn, DEV_A, vec![video(vdata(&v, "T"), T1)]);
    run_ok(&mut conn, DEV_A, vec![gone(&v, GoneReason::Deleted)]);
    run_ok(&mut conn, DEV_B, vec![gone(&v, GoneReason::Withdrawn)]);
    assert!(matches!(
        state_of(&mut conn, "video", &v),
        Some(State::VideoGone {
            reason: GoneReason::Deleted,
            ..
        })
    ));
}

#[test]
fn delete_on_alias_acts_on_root() {
    let mut conn = db();
    let (a, b) = (uid(1), uid(2));
    run_ok(&mut conn, DEV_A, vec![video(vdata(&a, "A"), T1)]);
    run_ok(&mut conn, DEV_B, vec![video(vdata(&b, "B"), T1)]);
    run_ok(&mut conn, DEV_B, vec![gone(&b, GoneReason::Withdrawn)]);
    assert_eq!(live_videos(&conn), 0);
    assert!(matches!(
        state_of(&mut conn, "video", &a),
        Some(State::VideoGone {
            reason: GoneReason::Withdrawn,
            ..
        })
    ));
}

#[test]
fn failed_batch_leaves_no_partial_effect() {
    let mut conn = db();
    let bad = uid(99);
    conn.execute_batch(&format!(
        "CREATE TRIGGER fail BEFORE INSERT ON summaries WHEN NEW.uid = '{bad}'
         BEGIN SELECT RAISE(ABORT, 'Testfehler'); END;"
    ))
    .unwrap();
    let dataset = db::dataset_id(&conn).unwrap();
    let v = uid(1);
    let result = push(
        &mut conn,
        DEV_A,
        &dataset,
        &[video(vdata(&v, "T"), T1), summary(&bad, &v)],
    );
    assert!(matches!(result, Err(Error::Db(_))));
    assert_eq!(live_videos(&conn), 0);
    assert!(pull_all(&mut conn, 0).is_empty());
    assert_eq!(db::current_seq(&conn).unwrap(), 0);
}

#[test]
fn wrong_dataset_and_invalid_op_have_no_effect() {
    let mut conn = db();
    let dataset = db::dataset_id(&conn).unwrap();
    let ops = [video(vdata(&uid(1), "T"), T1)];
    assert!(matches!(
        push(&mut conn, DEV_A, "falsch", &ops),
        Err(Error::DatasetMismatch(id)) if id == dataset
    ));
    let invalid = [ops[0].clone(), video(vdata("kaputt", "T"), T1)];
    assert!(matches!(
        push(&mut conn, DEV_A, &dataset, &invalid),
        Err(Error::Invalid(1, _))
    ));
    assert_eq!(live_videos(&conn), 0);
    assert!(matches!(
        pull_page(&mut conn, "falsch", 0, 10, 10),
        Err(Error::DatasetMismatch(_))
    ));
    assert!(matches!(
        pull_page(&mut conn, &dataset, 1, 10, 10),
        Err(Error::CursorAhead)
    ));
    let rotated = db::rotate_dataset(&conn).unwrap();
    assert_ne!(rotated, dataset);
    assert!(matches!(
        push(&mut conn, DEV_A, &dataset, &ops),
        Err(Error::DatasetMismatch(_))
    ));
}

// ---------------------------------------------------------------------------
// Regressionen aus dem Review (`.herd/impl-sync-2-korrekturen.md`)
// ---------------------------------------------------------------------------

#[test]
fn backup_retention_keeps_foreign_numeric_and_invalid_date_files() {
    let root = temp_dir("backup-foreign");
    let conn = db::open(&root.join("sync.db")).unwrap();
    let dir = root.join("backups");
    std::fs::create_dir(&dir).unwrap();
    let foreign = [
        "1234567890.db",
        "2026-13-45.db",
        "2026-9-01x.db",
        "1234567890.db.tmp",
    ];
    for name in foreign {
        std::fs::write(dir.join(name), b"fremd").unwrap();
    }
    for day in 1..=7 {
        std::fs::write(dir.join(format!("2026-09-{day:02}.db")), b"alt").unwrap();
    }
    assert!(db::backup(&conn, &dir, "2026-09-24").unwrap());
    let names = file_names(&dir);
    for name in foreign {
        assert!(
            names.contains(&name.to_owned()),
            "{name} gelöscht: {names:?}"
        );
    }
    assert!(!names.contains(&"2026-09-01.db".to_owned()), "{names:?}");
    assert_eq!(names.len(), foreign.len() + 7, "{names:?}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn known_round_with_other_video_is_rejected_and_echoed() {
    let mut conn = db();
    let (v1, v2, c, r) = (uid(1), uid(2), uid(3), uid(4));
    run_ok(
        &mut conn,
        DEV_A,
        vec![
            video(vdata(&v1, "T"), T1),
            video(
                VideoData {
                    youtube_id: "abcdefghij2".into(),
                    ..vdata(&v2, "T")
                },
                T1,
            ),
            chat(&c, &v1),
            round(&r, &c, &v1),
        ],
    );
    let before = seq(&mut conn, "round", &r);
    let results = run(&mut conn, DEV_B, vec![round(&r, &c, &v2)]);
    assert_rejected(&results[0]);
    let (after, state) = state(&mut conn, "round", &r).unwrap();
    assert!(after > before);
    assert!(matches!(state, State::Round(RoundData { video_uid, .. }) if video_uid == v1));
    // Unverändert gesendet bleibt es `ok`.
    assert_eq!(
        run(&mut conn, DEV_B, vec![round(&r, &c, &v1)])[0].status,
        OpStatus::Ok
    );
}

#[test]
fn late_foreign_summary_is_removed_from_chat_and_chat_echoed() {
    let mut conn = db();
    let (v1, v2, c, own, foreign) = (uid(1), uid(2), uid(3), uid(4), uid(5));
    run_ok(
        &mut conn,
        DEV_A,
        vec![
            video(vdata(&v1, "T"), T1),
            video(
                VideoData {
                    youtube_id: "abcdefghij2".into(),
                    ..vdata(&v2, "T")
                },
                T1,
            ),
            chat_with(&c, &v1, "Chat", Some(vec![own.clone(), foreign.clone()])),
        ],
    );
    let before = seq(&mut conn, "chat", &c);
    run_ok(
        &mut conn,
        DEV_B,
        vec![summary(&own, &v1), summary(&foreign, &v2)],
    );
    let (after, state) = state(&mut conn, "chat", &c).unwrap();
    match state {
        State::Chat(data) => {
            assert_eq!(data.context_options.summary_uids, Some(vec![own]))
        }
        other => panic!("{other:?}"),
    }
    assert!(after > before);
}

#[test]
fn flattened_aliases_get_a_new_seq() {
    let mut conn = db();
    let (x, y, z) = (uid(1), uid(2), uid(3));
    run_ok(
        &mut conn,
        DEV_A,
        vec![
            collection(&x, "a", T1),
            collection(&y, "b", T1),
            collection(&z, "c", T1),
        ],
    );
    run_ok(&mut conn, DEV_A, vec![collection(&x, "b", T2)]); // X → Y
    let since = seq(&mut conn, "collection", &x);
    run_ok(&mut conn, DEV_A, vec![collection(&y, "c", T3)]); // Y → Z, X → Z
    let changes = pull_all(&mut conn, since);
    assert!(
        changes.iter().any(|item| item.state
            == State::CollectionGone {
                uid: x.clone(),
                reason: GoneReason::Merged,
                merged_into: Some(z.clone()),
            }),
        "{changes:?}"
    );
}

#[test]
fn wrong_dataset_precedes_invalid_operation() {
    let mut conn = db();
    let bad = [Op::CollectionDelete { uid: "bad".into() }];
    assert!(matches!(
        push(&mut conn, DEV_A, "alt", &bad),
        Err(Error::DatasetMismatch(_))
    ));
}
