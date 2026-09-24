//! Übrige Ende-zu-Ende-Fälle E2, E3, E5–E7, E9, E11–E17, E19, E22–E24,
//! dazu `sync_now` im Backoff und der Status nach 413/400 (Mock-Server).

use std::sync::Arc;
use std::time::Duration;

use rusqlite::params;
use sync_proto::{
    GoneReason, OpStatus, State, MAX_OPS_PER_PUSH, MAX_OP_BYTES, MAX_PUSH_BODY_BYTES,
};

use super::e2e_early_tests::{
    add_video, chat_with_rounds, copies, e10_scenario, live_videos, only_video, summary_texts,
};
use super::e2e_support::{device_at, mock, projection, server, Device};
use super::{dump, inbox_count, outbox_keys, sample_video, strings, text, video_uid};
use crate::storage;
use crate::sync::client::{self, StopReason};
use crate::sync::config;
use crate::sync::engine::{self, Mode};
use crate::sync::{pull, snapshot};

fn collection_members(device: &Device, name: &str) -> Vec<String> {
    strings(
        &device.conn(),
        "SELECT v.video_id FROM video_collections vc JOIN videos v ON v.id = vc.video_id \
         JOIN collections c ON c.id = vc.collection_id WHERE c.name = ?1 ORDER BY v.video_id",
        name,
    )
}

fn collection_names(device: &Device) -> Vec<String> {
    storage::get_collections(&device.paths)
        .unwrap()
        .into_iter()
        .map(|collection| collection.name)
        .collect()
}

fn gone_reason(states: &[State], uid: &str) -> Option<GoneReason> {
    states.iter().find_map(|state| match state {
        State::VideoGone {
            uid: gone, reason, ..
        } if gone == uid => Some(*reason),
        _ => None,
    })
}

/// E2: Beide legen offline „KI“ an und ordnen je ein Video zu; sync A, B, A
/// → eine „KI“ mit beiden Videos auf beiden.
#[tokio::test(flavor = "multi_thread")]
async fn e2_same_collection_name_merges() {
    let server = server(2).await;
    let a = Device::new(&server, 0);
    let b = Device::new(&server, 1);
    for (device, youtube_id) in [(&a, "vidAAAAAAA1"), (&b, "vidBBBBBBB2")] {
        let video = add_video(device, youtube_id);
        let ki = storage::create_collection(&device.paths, "KI").unwrap();
        storage::set_video_collections(&device.paths, video, vec![ki.id]).unwrap();
    }

    a.sync().await;
    b.sync().await;
    a.sync().await;

    for device in [&a, &b] {
        assert_eq!(collection_names(device), vec!["KI"]);
        assert_eq!(
            collection_members(device, "KI"),
            vec!["vidAAAAAAA1", "vidBBBBBBB2"]
        );
    }
    assert_eq!(projection(&a.paths), projection(&b.paths));
}

/// E3: A löscht V, B hängt offline eine Summary an; sync A, B, A → V auf
/// beiden weg.
#[tokio::test(flavor = "multi_thread")]
async fn e3_delete_wins_over_offline_child() {
    let server = server(2).await;
    let a = Device::new(&server, 0);
    let b = Device::new(&server, 1);
    let video_a = add_video(&a, "vidAAAAAAA1");
    a.sync().await;
    b.sync().await;
    let video_b = only_video(&b, "vidAAAAAAA1");

    storage::delete_video(&a.paths, video_a).unwrap();
    storage::update_summary(&b.paths, video_b, "offline", None, None, None).unwrap();
    a.sync().await;
    b.sync().await;
    a.sync().await;

    for device in [&a, &b] {
        assert!(storage::get_videos(&device.paths).unwrap().is_empty());
        assert_eq!(device.engine.status().pending, 0);
    }
    assert!(live_videos(&server.states().await, "vidAAAAAAA1").is_empty());
}

/// E5: B mit `newVideosLocal`; neues Video; sync B, A → Server und A kennen
/// es nicht.
#[tokio::test(flavor = "multi_thread")]
async fn e5_new_videos_local_stay_on_device() {
    let server = server(2).await;
    let a = Device::new(&server, 0);
    let b = Device::new(&server, 1);
    let mut input = server.input(1);
    input.new_videos_local = true;
    b.engine.set_config(&input).await.unwrap();
    // Wie `add_video`: `insert_video(…, config.new_videos_local)`.
    let local = config::load(&b.paths).new_videos_local;
    let video = storage::insert_video(&b.paths, sample_video("vidAAAAAAA1"), local).unwrap();
    assert!(video.local_only);
    storage::update_summary(&b.paths, video.id, "S", None, None, None).unwrap();

    b.sync().await;
    a.sync().await;

    assert!(live_videos(&server.states().await, "vidAAAAAAA1").is_empty());
    assert!(storage::get_videos(&a.paths).unwrap().is_empty());
    assert_eq!(b.engine.status().pending, 0);
}

/// E6: Titel offline auf A (später) und B; sync B, A, B → beide = A.
#[tokio::test(flavor = "multi_thread")]
async fn e6_later_title_wins() {
    let server = server(2).await;
    let a = Device::new(&server, 0);
    let b = Device::new(&server, 1);
    let video_a = add_video(&a, "vidAAAAAAA1");
    a.sync().await;
    b.sync().await;
    let video_b = only_video(&b, "vidAAAAAAA1");

    b.conn()
        .execute(
            "UPDATE videos SET title = 'von B' WHERE id = ?1",
            params![video_b],
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(20));
    a.conn()
        .execute(
            "UPDATE videos SET title = 'von A' WHERE id = ?1",
            params![video_a],
        )
        .unwrap();
    b.sync().await;
    a.sync().await;
    b.sync().await;

    for device in [&a, &b] {
        let id = only_video(device, "vidAAAAAAA1");
        assert_eq!(
            storage::get_video(&device.paths, id)
                .unwrap()
                .unwrap()
                .title,
            "von A"
        );
    }
}

/// E7: Server nicht erreichbar → Fehlerstatus, Outbox und Fachdaten
/// unverändert (`published` darf gesetzt sein).
#[tokio::test(flavor = "multi_thread")]
async fn e7_unreachable_server_changes_nothing() {
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    let device = device_at(&closed);
    let video = add_video(&device, "vidAAAAAAA1");
    storage::update_summary(&device.paths, video, "S", None, None, None).unwrap();
    let outbox = outbox_keys(&device.conn());
    let data = projection(&device.paths);

    let status = device.engine.run(Mode::Full).await;

    assert!(
        status
            .last_error
            .as_deref()
            .unwrap_or("")
            .contains("nicht erreichbar"),
        "{status:?}"
    );
    assert_eq!(status.stopped, None);
    assert!(!status.running);
    assert_eq!(status.pending, 2);
    assert_eq!(outbox_keys(&device.conn()), outbox);
    assert_eq!(projection(&device.paths), data);
}

/// E9: Grabstein vor verspätetem Anlegen derselben uid → Video bleibt weg.
#[tokio::test(flavor = "multi_thread")]
async fn e9_late_create_after_tombstone_stays_gone() {
    let server = server(2).await;
    let a = Device::new(&server, 0);
    a.sync().await;
    let video = add_video(&a, "vidAAAAAAA1");
    let uid = video_uid(&a.conn(), video);
    let late = snapshot::snapshot(&mut a.conn()).unwrap().upserts;
    assert_eq!(late.len(), 1);

    storage::delete_video(&a.paths, video).unwrap();
    a.sync().await;
    let http = client::http_client().unwrap();
    let ops = late.iter().map(|pending| pending.op.clone()).collect();
    let results = engine::send(&http, &a.config(), &a.dataset(), ops)
        .await
        .unwrap();

    assert_eq!(results[0].status, OpStatus::Rejected);
    let states = server.states().await;
    assert!(live_videos(&states, "vidAAAAAAA1").is_empty());
    assert_eq!(gone_reason(&states, &uid), Some(GoneReason::Deleted));
    let b = Device::new(&server, 1);
    b.sync().await;
    assert!(storage::get_videos(&b.paths).unwrap().is_empty());
}

/// E11: E10, danach gibt B seine private Kopie frei; sync B, A → eine
/// Existenz; ursprünglich gemeinsame Summaries/Chats erscheinen doppelt
/// (bekannte Grenze), keine geht verloren.
#[tokio::test(flavor = "multi_thread")]
async fn e11_releasing_private_copy_merges_with_duplicates() {
    let server = server(2).await;
    let a = Device::new(&server, 0);
    let b = Device::new(&server, 1);
    let (video_a, private_b, _) = e10_scenario(&a, &b).await;

    storage::video_set_local_only(&b.paths, private_b, false).unwrap();
    b.sync().await;
    a.sync().await;
    b.sync().await;

    assert_eq!(live_videos(&server.states().await, "vidAAAAAAA1").len(), 1);
    for device in [&a, &b] {
        let id = only_video(device, "vidAAAAAAA1");
        // A: S1 (S2 gelöscht); B privat: S1, S2 → S1 doppelt, nichts fehlt.
        assert_eq!(summary_texts(device, id), vec!["S1", "S1", "S2"]);
        let chats = storage::list_chats(&device.paths, id).unwrap();
        assert_eq!(chats.len(), 2);
        for chat in chats {
            assert_eq!(
                storage::get_chat_messages(&device.paths, chat.id)
                    .unwrap()
                    .len(),
                4
            );
        }
    }
    assert_eq!(copies(&a, "vidAAAAAAA1").len(), 1);
    assert!(storage::get_video(&a.paths, video_a).unwrap().is_some());
    assert_eq!(projection(&a.paths), projection(&b.paths));
}

/// E12: A löscht V und legt es neu an; B schickt Altbestand → die neue
/// Existenz bleibt unberührt.
#[tokio::test(flavor = "multi_thread")]
async fn e12_stale_changes_do_not_touch_new_existence() {
    let server = server(2).await;
    let a = Device::new(&server, 0);
    let b = Device::new(&server, 1);
    let old = add_video(&a, "vidAAAAAAA1");
    a.sync().await;
    b.sync().await;
    let old_b = only_video(&b, "vidAAAAAAA1");
    b.conn()
        .execute(
            "UPDATE videos SET title = 'Altbestand' WHERE id = ?1",
            params![old_b],
        )
        .unwrap();
    storage::update_summary(&b.paths, old_b, "Altbestand", None, None, None).unwrap();

    storage::delete_video(&a.paths, old).unwrap();
    let renewed = add_video(&a, "vidAAAAAAA1");
    storage::update_summary(&a.paths, renewed, "neu", None, None, None).unwrap();
    a.sync().await;
    b.sync().await;
    a.sync().await;

    let new_uid = video_uid(&a.conn(), renewed);
    assert_eq!(
        live_videos(&server.states().await, "vidAAAAAAA1"),
        vec![new_uid.clone()]
    );
    for device in [&a, &b] {
        assert_eq!(
            copies(device, "vidAAAAAAA1"),
            vec![(new_uid.clone(), false)]
        );
        let id = only_video(device, "vidAAAAAAA1");
        assert_eq!(summary_texts(device, id), vec!["neu"]);
        assert_eq!(
            storage::get_video(&device.paths, id)
                .unwrap()
                .unwrap()
                .title,
            "Testvideo"
        );
    }
}

/// E13: Zwei Push-Blöcke, der zweite bricht ab (keine Quittung), Neustart →
/// alles genau einmal, der Rest ist sendbar.
#[tokio::test(flavor = "multi_thread")]
async fn e13_aborted_second_block_is_resent_once() {
    let server = server(2).await;
    let mut a = Device::new(&server, 0);
    a.sync().await;
    for index in 0..600 {
        storage::create_collection(&a.paths, &format!("Sammlung {index:03}")).unwrap();
    }
    let upserts = snapshot::snapshot(&mut a.conn()).unwrap().upserts;
    let (blocks, oversized) =
        engine::pack(upserts, MAX_OPS_PER_PUSH, MAX_PUSH_BODY_BYTES, MAX_OP_BYTES);
    assert!(oversized.is_empty());
    assert_eq!(
        blocks.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![500, 100]
    );
    let http = client::http_client().unwrap();
    let first = blocks[0].clone();
    let ops = first.iter().map(|pending| pending.op.clone()).collect();
    let results = engine::send(&http, &a.config(), &a.dataset(), ops)
        .await
        .unwrap();
    snapshot::acknowledge(&mut a.conn(), &first, &results).unwrap();
    // Zweiter Block kommt beim Server an, die Antwort nicht mehr.
    let ops = blocks[1].iter().map(|pending| pending.op.clone()).collect();
    engine::send(&http, &a.config(), &a.dataset(), ops)
        .await
        .unwrap();
    assert_eq!(outbox_keys(&a.conn()).len(), 100);

    a.restart();
    let status = a.sync().await;

    assert_eq!(status.pending, 0);
    assert!(status.unsendable.is_empty());
    let states = server.states().await;
    let mut names: Vec<String> = states
        .iter()
        .filter_map(|state| match state {
            State::Collection(data) => Some(data.name.clone()),
            _ => None,
        })
        .collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 600);
    assert_eq!(collection_names(&a).len(), 600);
    let b = Device::new(&server, 1);
    b.sync().await;
    assert_eq!(projection(&b.paths), projection(&a.paths));
}

/// E14: Pull bricht nach Seite 1 ab, Neustart → Inbox wird fortgesetzt,
/// der Endstand ist korrekt.
#[tokio::test(flavor = "multi_thread")]
async fn e14_pull_resumes_after_first_page() {
    let server = server(2).await;
    let a = Device::new(&server, 0);
    for index in 0..600 {
        storage::create_collection(&a.paths, &format!("Sammlung {index:03}")).unwrap();
    }
    a.sync().await;
    let mut b = Device::new(&server, 1);
    b.engine.run(Mode::PushOnly).await;
    let http = client::http_client().unwrap();
    let page = engine::pull_page(&http, &b.config(), &b.dataset(), 0)
        .await
        .unwrap();
    assert!(page.more);
    pull::stage_page(&mut b.conn(), &page).unwrap();
    assert_eq!(inbox_count(&b.conn()), page.states.len() as i64);
    assert_eq!(pull::cursor(&b.conn()).unwrap(), page.next);
    assert!(collection_names(&b).is_empty());

    b.restart();
    b.sync().await;

    assert_eq!(inbox_count(&b.conn()), 0);
    assert_eq!(collection_names(&b).len(), 600);
    assert_eq!(projection(&b.paths), projection(&a.paths));
}

/// E15: Operation über der Einzelgrenze und eine kleine → die kleine geht
/// durch, die große wird `unsendable` (mit Grund im Status).
#[tokio::test(flavor = "multi_thread")]
async fn e15_oversized_operation_becomes_unsendable() {
    let server = server(1).await;
    let a = Device::new(&server, 0);
    let mut big = sample_video("vidAAAAAAA1");
    let text = "x".repeat(MAX_OP_BYTES + 1024);
    big.transcript =
        Some(serde_json::json!([{"text": text, "start": 0.0, "time": "0:00"}]).to_string());
    storage::insert_video(&a.paths, big, false).unwrap();
    storage::create_collection(&a.paths, "Klein").unwrap();

    let status = a.sync().await;

    assert_eq!(status.pending, 1);
    assert_eq!(status.unsendable.len(), 1);
    assert_eq!(status.unsendable[0].entity, "video");
    assert!(
        status.unsendable[0].reason.contains("zu groß"),
        "{:?}",
        status.unsendable
    );
    let states = server.states().await;
    assert!(live_videos(&states, "vidAAAAAAA1").is_empty());
    assert!(states
        .iter()
        .any(|state| matches!(state, State::Collection(data) if data.name == "Klein")));
    // Eine neue Version des Eintrags ist wieder sendbar.
    a.conn()
        .execute("UPDATE videos SET transcript = '[]'", [])
        .unwrap();
    let status = a.sync().await;
    assert_eq!(status.pending, 0);
    assert!(status.unsendable.is_empty());
    assert_eq!(live_videos(&server.states().await, "vidAAAAAAA1").len(), 1);
}

/// E16: `dataset rotate` bei ausstehendem Löschwunsch → keine Wirkung auf dem
/// neuen Datensatz vor der Bestätigung; nach „Neu abgleichen“ ist der Bestand
/// wieder oben.
#[tokio::test(flavor = "multi_thread")]
async fn e16_rotated_dataset_waits_for_rebaseline() {
    let server = server(1).await;
    let mut a = Device::new(&server, 0);
    let video = add_video(&a, "vidAAAAAAA1");
    let kept = add_video(&a, "vidBBBBBBB2");
    storage::create_collection(&a.paths, "KI").unwrap();
    a.sync().await;
    let gone_uid = video_uid(&a.conn(), video);
    storage::delete_video(&a.paths, video).unwrap();

    server.rotate_dataset();
    // Ungesendet: ein Snapshot würde `published` setzen (lokale Wirkung).
    let fresh = add_video(&a, "vidCCCCCCC3");
    // Laufende Engine (Datensatz gebunden): der Server lehnt mit 409 ab. Der
    // Snapshot davor darf `published` setzen (wie E7), Fachdaten und Outbox
    // bleiben.
    let (data, outbox) = (projection(&a.paths), outbox_keys(&a.conn()));
    let status = a.engine.run(Mode::Full).await;
    assert_eq!(status.stopped, Some(StopReason::Dataset));
    assert_eq!(
        (projection(&a.paths), outbox_keys(&a.conn())),
        (data, outbox)
    );
    assert_eq!(live_videos(&server.states().await, "vidAAAAAAA1").len(), 1);
    assert!(live_videos(&server.states().await, "vidCCCCCCC3").is_empty());
    // Nach Neustart hält die Datensatz-Prüfung vor jeder lokalen Wirkung an
    // (auch vor dem Snapshot für ein weiteres ungesendetes Video).
    let second = add_video(&a, "vidDDDDDDD4");
    a.restart();
    let before = dump(&a.conn());
    let status = a.engine.run(Mode::Full).await;
    assert_eq!(status.stopped, Some(StopReason::Dataset));
    assert_eq!(dump(&a.conn()), before);
    assert_eq!(live_videos(&server.states().await, "vidAAAAAAA1").len(), 1);

    a.engine.rebaseline().await.unwrap();
    let status = a.sync().await;

    assert_eq!(status.pending, 0);
    let states = server.states().await;
    assert!(live_videos(&states, "vidAAAAAAA1").is_empty());
    assert_eq!(gone_reason(&states, &gone_uid), Some(GoneReason::Deleted));
    assert_eq!(
        live_videos(&states, "vidBBBBBBB2"),
        vec![video_uid(&a.conn(), kept)]
    );
    assert!(states
        .iter()
        .any(|state| matches!(state, State::Collection(data) if data.name == "KI")));
    assert_eq!(
        live_videos(&states, "vidCCCCCCC3"),
        vec![video_uid(&a.conn(), fresh)]
    );
    assert_eq!(
        live_videos(&states, "vidDDDDDDD4"),
        vec![video_uid(&a.conn(), second)]
    );
}

/// E17: Runden offline auf A und B zur gleichen Zeit → identischer Verlauf.
#[tokio::test(flavor = "multi_thread")]
async fn e17_simultaneous_rounds_sort_identically() {
    let server = server(2).await;
    let a = Device::new(&server, 0);
    let b = Device::new(&server, 1);
    let video_a = add_video(&a, "vidAAAAAAA1");
    let chat_a = chat_with_rounds(&a.paths, video_a, &["Start"]);
    a.sync().await;
    b.sync().await;
    let video_b = only_video(&b, "vidAAAAAAA1");
    let chat_b = storage::list_chats(&b.paths, video_b).unwrap()[0].id;

    chat_with_rounds_in(&a, video_a, chat_a, "von A");
    chat_with_rounds_in(&b, video_b, chat_b, "von B");
    // Beide neuen Runden zur selben Millisekunde.
    for (device, content) in [(&a, "von A"), (&b, "von B")] {
        device
            .conn()
            .execute(
                "UPDATE chat_messages SET created_at = '2030-01-01T00:00:00.000Z' \
                 WHERE round_uid = (SELECT round_uid FROM chat_messages WHERE content = ?1)",
                params![content],
            )
            .unwrap();
    }
    a.sync().await;
    b.sync().await;
    a.sync().await;

    let history = |device: &Device, chat: i64| -> Vec<String> {
        storage::get_chat_messages(&device.paths, chat)
            .unwrap()
            .into_iter()
            .map(|message| message.content)
            .collect()
    };
    let on_a = history(&a, chat_a);
    assert_eq!(on_a.len(), 6);
    assert_eq!(on_a, history(&b, chat_b));
    assert_eq!(projection(&a.paths), projection(&b.paths));
}

fn chat_with_rounds_in(device: &Device, video: i64, chat: i64, text: &str) {
    storage::append_chat_turn(
        &device.paths,
        video,
        Some(chat),
        "Chat",
        vec![
            crate::models::NewChatMessage::user(text),
            crate::models::NewChatMessage::assistant("Antwort"),
        ],
        None,
    )
    .unwrap();
}

/// E19: A löscht einen Chat mit neuer, ungesendeter Runde; sync A → kein
/// `unsendable`, Chat-Grabstein.
#[tokio::test(flavor = "multi_thread")]
async fn e19_deleting_chat_drops_unsent_round() {
    let server = server(1).await;
    let a = Device::new(&server, 0);
    let video = add_video(&a, "vidAAAAAAA1");
    let chat = chat_with_rounds(&a.paths, video, &["R1"]);
    a.sync().await;
    let chat_uid = text(&a.conn(), "SELECT uid FROM chats WHERE id = ?1", chat);
    chat_with_rounds_in(&a, video, chat, "ungesendet");
    let new_round = text(
        &a.conn(),
        "SELECT round_uid FROM chat_messages WHERE content = ?1",
        "ungesendet",
    );

    storage::delete_chat(&a.paths, chat).unwrap();
    let status = a.sync().await;

    assert!(status.unsendable.is_empty());
    assert_eq!(status.pending, 0);
    let states = server.states().await;
    assert!(states
        .iter()
        .any(|state| matches!(state, State::ChatGone { uid } if *uid == chat_uid)));
    assert!(!states
        .iter()
        .any(|state| matches!(state, State::Round(data) if data.uid == new_round)));
}

/// E22: URL-Wechsel auf einen Server mit anderer Datensatz-ID und höherem
/// Zähler, alte Inbox gefüllt → Anhalten vor jeder Wirkung; nach „Neu
/// abgleichen“ vollständig.
#[tokio::test(flavor = "multi_thread")]
async fn e22_switch_to_other_dataset_stops_before_any_effect() {
    let old = server(2).await;
    let new = server(2).await;
    let a = Device::new(&old, 0);
    add_video(&a, "vidAAAAAAA1");
    storage::create_collection(&a.paths, "KI").unwrap();
    a.sync().await;
    // Alte Inbox: B schiebt etwas, A holt die Seite, wendet aber nicht an.
    let b = Device::new(&old, 1);
    add_video(&b, "vidBBBBBBB2");
    b.sync().await;
    let http = client::http_client().unwrap();
    let cursor = pull::cursor(&a.conn()).unwrap();
    let page = engine::pull_page(&http, &a.config(), &a.dataset(), cursor)
        .await
        .unwrap();
    pull::stage_page(&mut a.conn(), &page).unwrap();
    assert!(inbox_count(&a.conn()) > 0);
    // Ungesendet: ein Snapshot würde `published` setzen (lokale Wirkung).
    add_video(&a, "vidCCCCCCC3");
    // Neuer Server mit höherem Zähler.
    let c = Device::new(&new, 1);
    for index in 0..20 {
        storage::create_collection(&c.paths, &format!("Neu {index}")).unwrap();
    }
    c.sync().await;

    let before = dump(&a.conn());
    a.engine.set_config(&new.input(0)).await.unwrap();
    let status = a.engine.run(Mode::Full).await;

    assert_eq!(status.stopped, Some(StopReason::Dataset));
    assert_eq!(dump(&a.conn()), before);
    assert_eq!(
        new.states().await.len(),
        20,
        "nichts von A auf dem neuen Server"
    );

    a.engine.rebaseline().await.unwrap();
    assert_eq!(inbox_count(&a.conn()), 0);
    a.sync().await;
    c.sync().await;

    assert_eq!(projection(&a.paths), projection(&c.paths));
    assert_eq!(collection_names(&a).len(), 21);
    assert_eq!(copies(&a, "vidBBBBBBB2"), Vec::new());
    assert_eq!(copies(&c, "vidAAAAAAA1").len(), 1);
    assert_eq!(copies(&c, "vidCCCCCCC3").len(), 1);
}

/// E23: Neu abgleichen, danach löscht der Benutzer ein früher
/// veröffentlichtes Video vor dem ersten Snapshot → der Grabstein wird
/// gesendet.
#[tokio::test(flavor = "multi_thread")]
async fn e23_delete_after_rebaseline_sends_tombstone() {
    let server = server(1).await;
    let mut a = Device::new(&server, 0);
    let video = add_video(&a, "vidAAAAAAA1");
    a.sync().await;
    let uid = video_uid(&a.conn(), video);
    server.rotate_dataset();
    a.restart();
    assert_eq!(
        a.engine.run(Mode::Full).await.stopped,
        Some(StopReason::Dataset)
    );

    a.engine.rebaseline().await.unwrap();
    storage::delete_video(&a.paths, video).unwrap();
    a.sync().await;

    let states = server.states().await;
    assert!(live_videos(&states, "vidAAAAAAA1").is_empty());
    assert_eq!(gone_reason(&states, &uid), Some(GoneReason::Deleted));
}

/// E24: `deleted` und `withdrawn` für dieselbe uid in beiden
/// Ankunftsreihenfolgen → der erste Grund bleibt; ein drittes Gerät löscht
/// bzw. privatisiert entsprechend.
#[tokio::test(flavor = "multi_thread")]
async fn e24_first_tombstone_reason_wins() {
    for deleted_first in [true, false] {
        let case = if deleted_first {
            "deleted zuerst"
        } else {
            "withdrawn zuerst"
        };
        let server = server(3).await;
        let a = Device::new(&server, 0);
        let b = Device::new(&server, 1);
        let c = Device::new(&server, 2);
        let video_a = add_video(&a, "vidAAAAAAA1");
        let uid = video_uid(&a.conn(), video_a);
        a.sync().await;
        b.sync().await;
        c.sync().await;
        let video_b = only_video(&b, "vidAAAAAAA1");

        storage::delete_video(&a.paths, video_a).unwrap();
        storage::video_set_local_only(&b.paths, video_b, true).unwrap();
        if deleted_first {
            a.sync().await;
            b.sync().await;
        } else {
            b.sync().await;
            a.sync().await;
        }
        c.sync().await;

        let expected = if deleted_first {
            GoneReason::Deleted
        } else {
            GoneReason::Withdrawn
        };
        assert_eq!(
            gone_reason(&server.states().await, &uid),
            Some(expected),
            "{case}"
        );
        let on_c = copies(&c, "vidAAAAAAA1");
        if deleted_first {
            assert!(on_c.is_empty(), "{case}: {on_c:?}");
        } else {
            assert_eq!(on_c.len(), 1, "{case}");
            assert!(on_c[0].1 && on_c[0].0 != uid, "{case}: {on_c:?}");
        }
        assert!(copies(&a, "vidAAAAAAA1").is_empty(), "{case}");
        let on_b = copies(&b, "vidAAAAAAA1");
        assert_eq!(on_b.len(), 1, "{case}");
        assert!(on_b[0].1, "{case}");
    }
}

/// Mock-Push: mehr als zwei Operationen oder eine Sammlung „RIESIG“ → 413;
/// eine Sammlung „KAPUTT“ → 400 mit ihrem Index; sonst `ok`.
fn picky_server(method: &str, path: &str, body: &[u8]) -> (u16, String) {
    match (method, path.split('?').next().unwrap_or("")) {
        ("GET", "/v1/health") => (
            200,
            r#"{"status":"ok","protocol":1,"datasetId":"mock"}"#.into(),
        ),
        ("GET", "/v1/pull") => (200, r#"{"states":[],"next":0,"more":false}"#.into()),
        ("POST", "/v1/push") => {
            let request: serde_json::Value = serde_json::from_slice(body).unwrap();
            let ops = request["ops"].as_array().unwrap();
            let name = |op: &serde_json::Value| op["name"].as_str().unwrap_or("").to_string();
            if ops.len() > 2 || ops.iter().any(|op| name(op) == "RIESIG") {
                return (413, r#"{"error":"tooLarge"}"#.into());
            }
            if let Some(index) = ops.iter().position(|op| name(op) == "KAPUTT") {
                return (
                    400,
                    format!(r#"{{"error":"invalid","index":{index},"message":"Name abgelehnt"}}"#),
                );
            }
            let results = vec![r#"{"status":"ok"}"#; ops.len()].join(",");
            (200, format!(r#"{{"results":[{results}]}}"#))
        }
        _ => (404, r#"{"error":"notFound"}"#.into()),
    }
}

/// 413 halbiert bis zur Einzeloperation, 400 mit Index markiert genau diese
/// Operation; der Status nennt beide mit Grund, der Rest ist quittiert.
#[tokio::test(flavor = "multi_thread")]
async fn status_reports_unsendable_after_413_and_400() {
    let server = mock(Arc::new(picky_server)).await;
    let device = device_at(&server.url);
    for name in ["Eins", "KAPUTT", "Zwei", "RIESIG", "Drei"] {
        storage::create_collection(&device.paths, name).unwrap();
    }

    let status = device.engine.run(Mode::Full).await;

    assert_eq!(status.last_error, None, "{status:?}");
    assert_eq!(status.pending, 2);
    let mut reasons: Vec<(String, String)> = status
        .unsendable
        .iter()
        .map(|entry| (entry.entity.clone(), entry.reason.clone()))
        .collect();
    reasons.sort();
    assert_eq!(
        reasons,
        vec![
            ("collection".to_string(), "Name abgelehnt".to_string()),
            (
                "collection".to_string(),
                "vom Server als zu groß abgelehnt".to_string()
            ),
        ]
    );
    // Nicht sendbare Einträge werden nicht erneut geschickt.
    let requests = server.requests();
    let status = device.engine.run(Mode::Full).await;
    assert_eq!(status.unsendable.len(), 2);
    assert_eq!(server.requests(), requests + 1, "nur der Pull");
}

/// `sync_now` (= `run(Full)`) überspringt den Backoff nach 5xx; der Takt der
/// Schleife wartet ihn ab.
#[tokio::test(flavor = "multi_thread")]
async fn sync_now_skips_backoff_but_tick_waits() {
    let failing = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let flag = failing.clone();
    let server = mock(Arc::new(move |method: &str, path: &str, body: &[u8]| {
        if flag.load(std::sync::atomic::Ordering::SeqCst) {
            return (503, r#"{"error":"internal"}"#.to_string());
        }
        picky_server(method, path, body)
    }))
    .await;
    let device = device_at(&server.url);
    storage::create_collection(&device.paths, "Eins").unwrap();

    let status = device.engine.run(Mode::Full).await;
    assert!(
        status.last_error.as_deref().unwrap_or("").contains("503"),
        "{status:?}"
    );
    assert_eq!(status.stopped, None);
    let after_failure = server.requests();

    // Takt im Backoff: kein Netz.
    let mut last_full = None;
    device.engine.tick(&mut last_full).await;
    assert_eq!(server.requests(), after_failure);

    failing.store(false, std::sync::atomic::Ordering::SeqCst);
    let status = device.engine.run(Mode::Full).await;
    assert!(server.requests() > after_failure);
    assert_eq!(status.last_error, None, "{status:?}");
    assert_eq!(status.pending, 0);
    assert!(status.last_success_at.is_some());
}
