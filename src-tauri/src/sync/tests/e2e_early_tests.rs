//! Frühe Ende-zu-Ende-Fälle E1, E4, E8, E10, E18, E20, E21 gegen den echten
//! Server (in-process) sowie „kein Netz ohne aktive Einstellungen“.

use std::time::Duration;

use rusqlite::params;
use sync_proto::{GoneReason, State};

use super::e2e_support::{projection, server, Device};
use super::{sample_video, strings, text, video_uid};
use crate::models::NewChatMessage;
use crate::storage::{self, AppPaths};
use crate::sync::config::{self, SyncConfigInput};
use crate::sync::engine::{self, Mode, SyncEngine};
use crate::sync::{apply, pull, snapshot};

pub(super) fn add_video(device: &Device, youtube_id: &str) -> i64 {
    storage::insert_video(&device.paths, sample_video(youtube_id), false)
        .unwrap()
        .id
}

pub(super) fn chat_with_rounds(paths: &AppPaths, video: i64, rounds: &[&str]) -> i64 {
    let mut chat = None;
    for text in rounds {
        let (created, _) = storage::append_chat_turn(
            paths,
            video,
            chat,
            "Chat",
            vec![
                NewChatMessage::user(*text),
                NewChatMessage::assistant("Antwort"),
            ],
            None,
        )
        .unwrap();
        chat = Some(created.id);
    }
    chat.unwrap()
}

/// Alle lokalen Videos mit dieser YouTube-ID: `(uid, local_only)`.
pub(super) fn copies(device: &Device, youtube_id: &str) -> Vec<(String, bool)> {
    let conn = device.conn();
    let mut stmt = conn
        .prepare("SELECT uid, local_only FROM videos WHERE video_id = ?1 ORDER BY id")
        .unwrap();
    let rows = stmt
        .query_map(params![youtube_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    rows.map(Result::unwrap).collect()
}

pub(super) fn live_videos(states: &[State], youtube_id: &str) -> Vec<String> {
    states
        .iter()
        .filter_map(|state| match state {
            State::Video(data) if data.youtube_id == youtube_id => Some(data.uid.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn summary_texts(device: &Device, video: i64) -> Vec<String> {
    let mut texts: Vec<String> = storage::get_summaries(&device.paths, video)
        .unwrap()
        .into_iter()
        .map(|summary| summary.summary)
        .collect();
    texts.sort();
    texts
}

/// Die einzige Kopie dieser YouTube-ID auf dem Gerät (lokale id).
pub(super) fn only_video(device: &Device, youtube_id: &str) -> i64 {
    let conn = device.conn();
    let ids = strings(
        &conn,
        "SELECT CAST(id AS TEXT) FROM videos WHERE video_id = ?1",
        youtube_id,
    );
    assert_eq!(ids.len(), 1, "{youtube_id}: {ids:?}");
    ids[0].parse().unwrap()
}

/// E1: A mit Video, 2 Summaries, Chat mit 2 Runden, Sammlung; sync A, B →
/// B identisch.
#[tokio::test(flavor = "multi_thread")]
async fn e1_second_device_receives_everything() {
    let server = server(2).await;
    let a = Device::new(&server, 0);
    let b = Device::new(&server, 1);
    let video = add_video(&a, "vidAAAAAAA1");
    storage::update_summary(&a.paths, video, "Eins", Some("P"), Some("M"), None).unwrap();
    std::thread::sleep(Duration::from_millis(5));
    storage::update_summary(&a.paths, video, "Zwei", Some("P"), Some("M"), None).unwrap();
    chat_with_rounds(&a.paths, video, &["R1", "R2"]);
    let collection = storage::create_collection(&a.paths, "KI").unwrap();
    storage::set_video_collections(&a.paths, video, vec![collection.id]).unwrap();

    a.sync().await;
    let status = b.sync().await;

    assert_eq!(projection(&b.paths), projection(&a.paths));
    assert_eq!(projection(&b.paths).len(), 1 + 2 + 1 + 4 + 1 + 1);
    assert_eq!(status.pending, 0);
    assert_eq!(a.sync().await.pending, 0);
}

/// E4: A schaltet V privat; sync A, B → Server ohne V; A und B je eine
/// private Kopie mit eigener uid.
#[tokio::test(flavor = "multi_thread")]
async fn e4_withdrawn_video_becomes_private_on_every_device() {
    let server = server(2).await;
    let a = Device::new(&server, 0);
    let b = Device::new(&server, 1);
    let video = add_video(&a, "vidAAAAAAA1");
    storage::update_summary(&a.paths, video, "S", None, None, None).unwrap();
    a.sync().await;
    b.sync().await;
    let shared = video_uid(&a.conn(), video);

    storage::video_set_local_only(&a.paths, video, true).unwrap();
    a.sync().await;
    b.sync().await;

    assert!(live_videos(&server.states().await, "vidAAAAAAA1").is_empty());
    let on_a = copies(&a, "vidAAAAAAA1");
    let on_b = copies(&b, "vidAAAAAAA1");
    assert_eq!(on_a.len(), 1);
    assert_eq!(on_b.len(), 1);
    assert!(on_a[0].1 && on_b[0].1, "{on_a:?} {on_b:?}");
    assert_ne!(on_a[0].0, shared);
    assert_ne!(on_b[0].0, shared);
    assert_ne!(on_a[0].0, on_b[0].0);
    let b_video = only_video(&b, "vidAAAAAAA1");
    assert_eq!(summary_texts(&b, b_video), vec!["S"]);
}

/// E8: A legt ein Video an, die Push-Antwort geht verloren; A schaltet es
/// privat und löscht es; sync A → Server hat einen Grabstein, kein Video.
#[tokio::test(flavor = "multi_thread")]
async fn e8_lost_push_answer_still_leaves_tombstone() {
    let server = server(2).await;
    let a = Device::new(&server, 0);
    a.sync().await;
    let video = add_video(&a, "vidAAAAAAA1");
    let uid = video_uid(&a.conn(), video);

    let mut conn = a.conn();
    let snapshot = snapshot::snapshot(&mut conn).unwrap();
    let ops = snapshot
        .upserts
        .iter()
        .map(|pending| pending.op.clone())
        .collect();
    let http = crate::sync::client::http_client().unwrap();
    engine::send(&http, &a.config(), &a.dataset(), ops)
        .await
        .unwrap();
    // Antwort verworfen: nichts quittiert.
    storage::video_set_local_only(&a.paths, video, true).unwrap();
    storage::delete_video(&a.paths, video).unwrap();
    a.sync().await;

    let states = server.states().await;
    assert!(live_videos(&states, "vidAAAAAAA1").is_empty(), "{states:?}");
    assert!(states.iter().any(|state| matches!(
        state,
        State::VideoGone { uid: gone, reason: GoneReason::Withdrawn, .. } if *gone == uid
    )));
    let b = Device::new(&server, 1);
    b.sync().await;
    assert!(storage::get_videos(&b.paths).unwrap().is_empty());
}

/// E10: E4 mit Summary, Chat und 2 Runden; A gibt frei; sync A, B; A löscht
/// und ändert Summary und Chat der geteilten Existenz; sync A, B → B hat die
/// geteilte Existenz neben seiner privaten Kopie, die unverändert bleibt.
#[tokio::test(flavor = "multi_thread")]
async fn e10_reshared_copy_lives_next_to_private_copy() {
    let server = server(2).await;
    let a = Device::new(&server, 0);
    let b = Device::new(&server, 1);
    e10_scenario(&a, &b).await;
}

/// Ablauf und Prüfungen von E10; Ergebnis `(Video auf A, private Kopie auf B,
/// geteilte Existenz auf B)` für E11.
pub(super) async fn e10_scenario(a: &Device, b: &Device) -> (i64, i64, i64) {
    let video = add_video(&a, "vidAAAAAAA1");
    storage::update_summary(&a.paths, video, "S1", None, None, None).unwrap();
    std::thread::sleep(Duration::from_millis(5));
    storage::update_summary(&a.paths, video, "S2", None, None, None).unwrap();
    chat_with_rounds(&a.paths, video, &["R1", "R2"]);
    a.sync().await;
    b.sync().await;
    storage::video_set_local_only(&a.paths, video, true).unwrap();
    a.sync().await;
    b.sync().await;
    let private_b = only_video(b, "vidAAAAAAA1");
    let private_before = private_projection(b, private_b);

    storage::video_set_local_only(&a.paths, video, false).unwrap();
    a.sync().await;
    b.sync().await;
    let shared_uid = video_uid(&a.conn(), video);
    assert_eq!(copies(b, "vidAAAAAAA1").len(), 2);
    let shared_b: i64 = b
        .conn()
        .query_row(
            "SELECT id FROM videos WHERE uid = ?1",
            params![shared_uid],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(summary_texts(b, shared_b), vec!["S1", "S2"]);

    let newest = storage::get_summaries(&a.paths, video).unwrap()[0].id;
    storage::delete_summary(&a.paths, newest).unwrap();
    let chat = storage::list_chats(&a.paths, video).unwrap()[0].id;
    storage::set_chat_context(
        &a.paths,
        chat,
        &crate::models::ChatContextOptions {
            transcript: false,
            summary_ids: Some(Vec::new()),
        },
    )
    .unwrap();
    a.sync().await;
    b.sync().await;

    assert_eq!(summary_texts(b, shared_b), vec!["S1"]);
    let chat_b = &storage::list_chats(&b.paths, shared_b).unwrap()[0];
    assert!(!chat_b.context_options.transcript);
    assert_eq!(chat_b.context_options.summary_ids, Some(Vec::new()));
    assert_eq!(
        storage::get_chat_messages(&b.paths, chat_b.id)
            .unwrap()
            .len(),
        4
    );
    assert_eq!(private_projection(b, private_b), private_before);
    assert_eq!(
        text(
            &b.conn(),
            "SELECT CAST(local_only AS TEXT) FROM videos WHERE id = ?1",
            private_b
        ),
        "1"
    );
    (video, private_b, shared_b)
}

/// Inhalt eines Videos samt Kindern und uids.
pub(super) fn private_projection(device: &Device, video: i64) -> Vec<String> {
    let conn = device.conn();
    let mut lines = strings(
        &conn,
        "SELECT uid || local_only || published || title || COALESCE(summary, '') \
         FROM videos WHERE id = ?1",
        video,
    );
    lines.extend(strings(
        &conn,
        "SELECT uid || created_at || summary FROM summaries WHERE video_id = ?1 ORDER BY uid",
        video,
    ));
    lines.extend(strings(
        &conn,
        "SELECT c.uid || c.title || COALESCE(c.context_options, '') || m.round_uid || m.position \
         || m.content FROM chats c JOIN chat_messages m ON m.chat_id = c.id \
         WHERE c.video_id = ?1 ORDER BY m.id",
        video,
    ));
    lines
}

/// E18: Beide importieren dieselbe YouTube-ID mit je einer Summary offline
/// → eine Existenz mit beiden Summaries auf beiden.
#[tokio::test(flavor = "multi_thread")]
async fn e18_independent_imports_merge_into_one_existence() {
    let server = server(2).await;
    let a = Device::new(&server, 0);
    let b = Device::new(&server, 1);
    let video_a = add_video(&a, "vidAAAAAAA1");
    let video_b = add_video(&b, "vidAAAAAAA1");
    storage::update_summary(&a.paths, video_a, "von A", None, None, None).unwrap();
    storage::update_summary(&b.paths, video_b, "von B", None, None, None).unwrap();

    a.sync().await;
    b.sync().await;
    a.sync().await;

    assert_eq!(live_videos(&server.states().await, "vidAAAAAAA1").len(), 1);
    for device in [&a, &b] {
        let id = only_video(device, "vidAAAAAAA1");
        assert_eq!(summary_texts(device, id), vec!["von A", "von B"]);
    }
    assert_eq!(projection(&a.paths), projection(&b.paths));
}

#[derive(Clone, Copy, Debug)]
enum Renewal {
    DeleteAndAdd,
    PrivateAndShare,
}

#[derive(Clone, Copy, Debug)]
enum Delivery {
    OneRun,
    TwoRuns,
    LostTombstoneAnswer,
}

/// E20: A löscht V und legt dieselbe YouTube-ID neu an (bzw. privat + sofort
/// wieder freigeben) – in einem Lauf, auf zwei Läufe verteilt, mit verworfener
/// Grabstein-Antwort → die neue Existenz lebt mit neuem Inhalt.
#[tokio::test(flavor = "multi_thread")]
async fn e20_renewed_video_survives_its_own_tombstone() {
    for renewal in [Renewal::DeleteAndAdd, Renewal::PrivateAndShare] {
        for delivery in [
            Delivery::OneRun,
            Delivery::TwoRuns,
            Delivery::LostTombstoneAnswer,
        ] {
            let case = format!("{renewal:?}/{delivery:?}");
            let server = server(2).await;
            let a = Device::new(&server, 0);
            let b = Device::new(&server, 1);
            let old = add_video(&a, "vidAAAAAAA1");
            storage::update_summary(&a.paths, old, "alt", None, None, None).unwrap();
            a.sync().await;
            b.sync().await;
            let old_uid = video_uid(&a.conn(), old);

            let renewed = match renewal {
                Renewal::DeleteAndAdd => {
                    storage::delete_video(&a.paths, old).unwrap();
                    if matches!(delivery, Delivery::TwoRuns) {
                        a.sync().await;
                    }
                    add_video(&a, "vidAAAAAAA1")
                }
                Renewal::PrivateAndShare => {
                    storage::video_set_local_only(&a.paths, old, true).unwrap();
                    if matches!(delivery, Delivery::TwoRuns) {
                        a.sync().await;
                    }
                    storage::video_set_local_only(&a.paths, old, false).unwrap();
                    old
                }
            };
            storage::update_summary(&a.paths, renewed, "neu", None, None, None).unwrap();
            if matches!(delivery, Delivery::LostTombstoneAnswer) {
                let mut conn = a.conn();
                let snapshot = snapshot::snapshot(&mut conn).unwrap();
                assert_eq!(snapshot.deletes.len(), 1, "{case}");
                let ops = snapshot
                    .deletes
                    .iter()
                    .map(|pending| pending.op.clone())
                    .collect();
                let http = crate::sync::client::http_client().unwrap();
                engine::send(&http, &a.config(), &a.dataset(), ops)
                    .await
                    .unwrap();
            }
            a.sync().await;
            b.sync().await;

            let new_uid = video_uid(&a.conn(), renewed);
            assert_ne!(new_uid, old_uid, "{case}");
            let live = live_videos(&server.states().await, "vidAAAAAAA1");
            assert_eq!(live, vec![new_uid.clone()], "{case}");
            // B: die neue Existenz geteilt; beim Zurückziehen behält B
            // zusätzlich eine private Kopie der alten (wie E4).
            let on_b_copies = copies(&b, "vidAAAAAAA1");
            let shared: Vec<&(String, bool)> =
                on_b_copies.iter().filter(|(_, private)| !private).collect();
            assert_eq!(shared, vec![&(new_uid.clone(), false)], "{case}");
            let private_copies = on_b_copies.len() - shared.len();
            let expected_private = usize::from(matches!(renewal, Renewal::PrivateAndShare));
            assert_eq!(private_copies, expected_private, "{case}");
            let on_b: i64 = b
                .conn()
                .query_row(
                    "SELECT id FROM videos WHERE uid = ?1",
                    params![new_uid],
                    |row| row.get(0),
                )
                .unwrap();
            let expected: Vec<String> = match renewal {
                // Die private Zwischenkopie behielt ihre Summaries.
                Renewal::PrivateAndShare => vec!["alt".into(), "neu".into()],
                Renewal::DeleteAndAdd => vec!["neu".into()],
            };
            assert_eq!(summary_texts(&b, on_b), expected, "{case}");
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum LocalKi {
    Keep,
    Rename,
    Delete,
}

/// E21: Server-Sammlung Y „KI“ mit Zuordnung (V, Y); B legt nach seinem
/// Snapshot eine eigene „KI“ an; zwei volle Läufe, dazwischen App-Neustart →
/// B hat Y und (V, Y), Y bleibt unverändert.
#[tokio::test(flavor = "multi_thread")]
async fn e21_deferred_server_collection_arrives_on_every_variant() {
    for variant in [LocalKi::Keep, LocalKi::Rename, LocalKi::Delete] {
        let case = format!("{variant:?}");
        let server = server(2).await;
        let a = Device::new(&server, 0);
        let mut b = Device::new(&server, 1);
        let video = add_video(&a, "vidAAAAAAA1");
        let y = storage::create_collection(&a.paths, "KI").unwrap();
        storage::set_video_collections(&a.paths, video, vec![y.id]).unwrap();
        a.sync().await;
        let y_uid = text(&a.conn(), "SELECT uid FROM collections WHERE id = ?1", y.id);
        let video_uid_a = video_uid(&a.conn(), video);
        // Erster Lauf von B in Einzelschritten: Bindung und (leerer) Push,
        // danach legt B „KI“ an, dann Pull und Apply.
        let status = b.engine.run(Mode::PushOnly).await;
        assert_eq!(status.last_error, None);
        let mut conn = b.conn();
        assert!(snapshot::snapshot(&mut conn).unwrap().upserts.is_empty());
        let local = storage::create_collection(&b.paths, "KI").unwrap();
        let http = crate::sync::client::http_client().unwrap();
        let mut since = 0;
        loop {
            let page = engine::pull_page(&http, &b.config(), &b.dataset(), since)
                .await
                .unwrap();
            since = page.next;
            let more = page.more;
            pull::stage_page(&mut conn, &page).unwrap();
            if !more {
                break;
            }
        }
        apply::apply(&mut conn).unwrap();
        drop(conn);
        assert_eq!(super::inbox_count(&b.conn()), 2, "{case}");

        b.restart();
        match variant {
            LocalKi::Keep => {}
            LocalKi::Rename => {
                storage::update_collection(&b.paths, local.id, "Forschung").unwrap();
            }
            LocalKi::Delete => storage::delete_collection(&b.paths, local.id).unwrap(),
        }
        b.sync().await;
        b.sync().await;

        let conn = b.conn();
        assert_eq!(super::inbox_count(&conn), 0, "{case}");
        let name = text(&conn, "SELECT name FROM collections WHERE uid = ?1", &y_uid);
        assert_eq!(name, "KI", "{case}");
        let members = strings(
            &conn,
            "SELECT v.uid FROM video_collections vc JOIN videos v ON v.id = vc.video_id \
             JOIN collections c ON c.id = vc.collection_id WHERE c.uid = ?1",
            &y_uid,
        );
        assert_eq!(members, vec![video_uid_a.clone()], "{case}");
        // Y auf dem Server unverändert: Name, Wurzel, genau eine Zuordnung.
        let states = server.states().await;
        let collections: Vec<(String, String)> = states
            .iter()
            .filter_map(|state| match state {
                State::Collection(data) => Some((data.uid.clone(), data.name.clone())),
                _ => None,
            })
            .collect();
        assert!(
            collections.contains(&(y_uid.clone(), "KI".to_string())),
            "{case}: {collections:?}"
        );
        let memberships: Vec<(String, bool)> = states
            .iter()
            .filter_map(|state| match state {
                State::Membership {
                    video_uid,
                    collection_uid,
                    present,
                } if *collection_uid == y_uid => Some((video_uid.clone(), *present)),
                _ => None,
            })
            .collect();
        assert_eq!(memberships, vec![(video_uid_a, true)], "{case}");
        a.sync().await;
        assert_eq!(
            text(
                &a.conn(),
                "SELECT name FROM collections WHERE uid = ?1",
                &y_uid
            ),
            "KI",
            "{case}"
        );
    }
}

/// Ohne aktive Einstellungen öffnet die Engine keine Verbindung: weder ein
/// Lauf noch der Hintergrundtakt berühren den (erreichbaren) Server.
#[tokio::test(flavor = "multi_thread")]
async fn disabled_sync_makes_no_network_access() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (_temp, paths) = super::temp_paths();
    config::save(
        &paths,
        &SyncConfigInput {
            enabled: false,
            server_url: url,
            token: Some("geheim".into()),
            new_videos_local: false,
        },
    )
    .unwrap();
    let engine = SyncEngine::new(paths.clone(), Box::new(|_| {})).unwrap();
    storage::insert_video(&paths, sample_video("vidAAAAAAA1"), false).unwrap();

    let status = engine.run(Mode::Full).await;
    let mut last_full = None;
    engine.tick(&mut last_full).await;
    engine.tick(&mut last_full).await;

    assert!(!status.enabled);
    assert_eq!(status.last_error, None);
    let accepted = tokio::time::timeout(Duration::from_millis(300), listener.accept()).await;
    assert!(accepted.is_err(), "unerwartete Verbindung");
    // Ohne `sync.json` ebenso.
    let (_temp, fresh) = super::temp_paths();
    let engine = SyncEngine::new(fresh, Box::new(|_| {})).unwrap();
    assert!(!engine.run(Mode::Full).await.enabled);
}

/// Packen nach Anzahl und Bytes; zu große Operationen kommen mit Grund zurück.
#[test]
fn pack_respects_count_bytes_and_single_op_limit() {
    let op = |seq: i64, size: usize| snapshot::Pending {
        seq,
        op: sync_proto::Op::CollectionDelete {
            uid: "a".repeat(size),
        },
    };
    let len = |size: usize| serde_json::to_vec(&op(0, size).op).unwrap().len();
    let (blocks, oversized) = engine::pack((1..=5).map(|seq| op(seq, 10)).collect(), 2, 1000, 1000);
    let sizes: Vec<usize> = blocks.iter().map(Vec::len).collect();
    assert_eq!(sizes, vec![2, 2, 1]);
    assert!(oversized.is_empty());

    // Body: 10 Bytes Rahmen + Operationen + Kommas.
    let body = 10 + 2 * len(10) + 1;
    let (blocks, _) = engine::pack((1..=3).map(|seq| op(seq, 10)).collect(), 500, body, 1000);
    let sizes: Vec<usize> = blocks.iter().map(Vec::len).collect();
    assert_eq!(sizes, vec![2, 1]);

    let (blocks, oversized) =
        engine::pack(vec![op(1, 10), op(2, 400), op(3, 10)], 500, 10_000, len(10));
    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].iter().map(|p| p.seq).collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_eq!(oversized.len(), 1);
    assert_eq!(oversized[0].0, 2);
}

/// 401 hält an (bis zur Einstellungsänderung); Fehlertext und Status
/// enthalten das Token nicht; die Outbox bleibt.
#[tokio::test(flavor = "multi_thread")]
async fn wrong_token_stops_until_settings_change() {
    let server = server(1).await;
    let device = Device::new(&server, 0);
    add_video(&device, "vidAAAAAAA1");
    let token = "falsches-token-0123456789";
    config::save(
        &device.paths,
        &SyncConfigInput {
            enabled: true,
            server_url: server.url.clone(),
            token: Some(token.into()),
            new_videos_local: false,
        },
    )
    .unwrap();
    let status = device.engine.run(Mode::Full).await;
    assert_eq!(status.stopped, Some(crate::sync::client::StopReason::Auth));
    let json = serde_json::to_string(&status).unwrap();
    assert!(!json.contains(token), "{json}");
    assert_eq!(status.pending, 1);
    // Angehalten: kein weiterer Versuch, auch nicht per sync_now.
    assert_eq!(device.engine.run(Mode::Full).await.stopped, status.stopped);

    device
        .engine
        .set_config(&SyncConfigInput {
            enabled: true,
            server_url: server.url.clone(),
            token: Some(server.config(0).token),
            new_videos_local: false,
        })
        .await
        .unwrap();
    let status = device.sync().await;
    assert_eq!(status.pending, 0);
}
