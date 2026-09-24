//! Engine-Korrekturen aus dem Review (`.herd/impl-sync-3-korrekturen.md`):
//! blockierte Löschung, Status aus der DB, `running` unter dem Lauf-Mutex.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::e2e_early_tests::add_video;
use super::e2e_support::{device_at, mock, server, Device};
use super::{outbox_keys, video_uid};
use crate::storage;
use crate::sync::config::{self, SyncConfigInput};
use crate::sync::engine::{Mode, SyncEngine, SyncEvent, BLOCKED_DELETE};
use crate::sync::outbox;

#[derive(Default)]
struct Recorded {
    pushes: Mutex<Vec<String>>,
    pulls: AtomicUsize,
    accept_deletes: AtomicBool,
}

/// Mock: `videoGone` wird mit 400 (Index 0) bzw. 413 abgelehnt, solange
/// `accept_deletes` aus ist; alles andere `ok`.
fn delete_rejecting_server(
    recorded: Arc<Recorded>,
    status: u16,
) -> impl Fn(&str, &str, &[u8]) -> (u16, String) + Send + Sync {
    move |method, path, body| match (method, path.split('?').next().unwrap_or("")) {
        ("GET", "/v1/health") => (
            200,
            r#"{"status":"ok","protocol":1,"datasetId":"mock"}"#.into(),
        ),
        ("GET", "/v1/pull") => {
            recorded.pulls.fetch_add(1, Ordering::SeqCst);
            (200, r#"{"states":[],"next":0,"more":false}"#.into())
        }
        ("POST", "/v1/push") => {
            let text = String::from_utf8_lossy(body).to_string();
            recorded.pushes.lock().unwrap().push(text.clone());
            if text.contains(r#""type":"videoGone""#)
                && !recorded.accept_deletes.load(Ordering::SeqCst)
            {
                return if status == 413 {
                    (413, r#"{"error":"tooLarge"}"#.into())
                } else {
                    (
                        400,
                        r#"{"error":"invalid","index":0,"message":"Grabstein abgelehnt"}"#.into(),
                    )
                };
            }
            let count = serde_json::from_str::<serde_json::Value>(&text).unwrap()["ops"]
                .as_array()
                .unwrap()
                .len();
            let results = vec![r#"{"status":"ok"}"#; count].join(",");
            (200, format!(r#"{{"results":[{results}]}}"#))
        }
        _ => (404, r#"{"error":"notFound"}"#.into()),
    }
}

/// Befund 1: Eine nicht sendbare Löschung (400 mit Index bzw. 413) beendet
/// den Lauf ohne Upserts und ohne Pull, auch in späteren Läufen; eine neue
/// Version des Eintrags macht sie wieder sendbar.
#[tokio::test(flavor = "multi_thread")]
async fn unsendable_delete_blocks_upserts_and_pull() {
    for (status, reason) in [
        (400, "Grabstein abgelehnt"),
        (413, "vom Server als zu groß abgelehnt"),
    ] {
        let recorded = Arc::new(Recorded::default());
        let server = mock(Arc::new(delete_rejecting_server(recorded.clone(), status))).await;
        let device = device_at(&server.url);
        let video = add_video(&device, "vidAAAAAAA1");
        let uid = video_uid(&device.conn(), video);
        device
            .conn()
            .execute("UPDATE videos SET published = 1", [])
            .unwrap();
        device
            .conn()
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        storage::create_collection(&device.paths, "KI").unwrap();
        storage::delete_video(&device.paths, video).unwrap();

        let result = device.engine.run(Mode::Full).await;

        assert_eq!(
            result.last_error.as_deref(),
            Some(BLOCKED_DELETE),
            "{status}"
        );
        assert_eq!(result.stopped, None);
        assert_eq!(result.pending, 2, "{status}");
        assert_eq!(result.unsendable.len(), 1, "{status}");
        assert_eq!(result.unsendable[0].entity, "video");
        assert_eq!(result.unsendable[0].reason, reason);
        let pushes = recorded.pushes.lock().unwrap().clone();
        assert_eq!(pushes.len(), 1, "{status}: {pushes:?}");
        assert!(pushes[0].contains("videoGone"));
        assert!(!pushes
            .iter()
            .any(|body| body.contains(r#""type":"collection""#)));
        assert_eq!(recorded.pulls.load(Ordering::SeqCst), 0);

        // Nächster Lauf: die Löschung ist weiter nicht sendbar → nichts.
        let result = device.engine.run(Mode::Full).await;
        assert_eq!(result.last_error.as_deref(), Some(BLOCKED_DELETE));
        assert_eq!(recorded.pushes.lock().unwrap().len(), 1);
        assert_eq!(recorded.pulls.load(Ordering::SeqCst), 0);

        // Neue Version des Eintrags (Reparatur) → wieder sendbar, Reihenfolge
        // Löschung vor Upsert.
        outbox::put(
            &device.conn(),
            "video",
            &uid,
            Some(&uid),
            None,
            Some("deleted"),
        )
        .unwrap();
        recorded.accept_deletes.store(true, Ordering::SeqCst);
        let result = device.engine.run(Mode::Full).await;
        assert_eq!(result.last_error, None, "{status}");
        assert_eq!(result.pending, 0);
        assert!(result.unsendable.is_empty());
        let pushes = recorded.pushes.lock().unwrap().clone();
        assert_eq!(pushes.len(), 3, "{pushes:?}");
        assert!(pushes[1].contains("videoGone"));
        assert!(pushes[2].contains(r#""type":"collection""#));
        assert_eq!(recorded.pulls.load(Ordering::SeqCst), 1);
    }
}

/// Befund 3: `sync_status` liest `pending` aus der DB, auch bei
/// ausgeschaltetem Sync; Privatschalten sendet `sync://status`.
#[tokio::test(flavor = "multi_thread")]
async fn status_counts_come_from_the_database() {
    let (_temp, paths) = super::temp_paths();
    config::save(
        &paths,
        &SyncConfigInput {
            enabled: false,
            server_url: String::new(),
            token: None,
            new_videos_local: false,
        },
    )
    .unwrap();
    let events: Arc<Mutex<Vec<i64>>> = Arc::default();
    let recorder = events.clone();
    let engine = SyncEngine::new(
        paths.clone(),
        Box::new(move |event| {
            if let SyncEvent::Status(status) = event {
                recorder.lock().unwrap().push(status.pending);
            }
        }),
    )
    .unwrap();
    assert_eq!(engine.current_status().await.pending, 0);

    let video = storage::insert_video(&paths, super::sample_video("vidAAAAAAA1"), false).unwrap();
    let status = engine.current_status().await;
    assert!(!status.enabled);
    assert_eq!(status.pending, 1);

    storage::video_set_local_only(&paths, video.id, true).unwrap();
    events.lock().unwrap().clear();
    let status = engine.publish_status().await;
    assert_eq!(status.pending, 0);
    assert_eq!(events.lock().unwrap().last(), Some(&0));
    assert!(outbox_keys(&storage::open_db(&paths).unwrap()).is_empty());
}

/// Befund 2: Zwei überlappende Läufe melden `running` in sauberer Folge
/// (an, aus, an, aus); keiner schaltet `running` für den anderen ab.
#[tokio::test(flavor = "multi_thread")]
async fn overlapping_runs_report_running_in_order() {
    let server = server(1).await;
    let device = Device::new(&server, 0);
    add_video(&device, "vidAAAAAAA1");
    let running: Arc<Mutex<Vec<bool>>> = Arc::default();
    let recorder = running.clone();
    let engine = Arc::new(
        SyncEngine::new(
            device.paths.clone(),
            Box::new(move |event| {
                if let SyncEvent::Status(status) = event {
                    let mut seen = recorder.lock().unwrap();
                    if seen.last() != Some(&status.running) {
                        seen.push(status.running);
                    }
                }
            }),
        )
        .unwrap(),
    );

    for _ in 0..10 {
        running.lock().unwrap().clear();
        let first = tokio::spawn({
            let engine = engine.clone();
            async move { engine.run(Mode::Full).await }
        });
        let second = tokio::spawn({
            let engine = engine.clone();
            async move { engine.run(Mode::Full).await }
        });
        let (first, second) = (first.await.unwrap(), second.await.unwrap());
        assert_eq!(first.last_error, None);
        assert_eq!(second.last_error, None);
        assert_eq!(*running.lock().unwrap(), vec![true, false, true, false]);
    }
}

/// Nachprüfung: Eine als `unsendable` markierte Löschung sperrt nicht
/// dauerhaft. Die Schleife (`tick`) versucht es nicht erneut; „Jetzt
/// synchronisieren“ bzw. neue Einstellungen heben die Markierung auf, danach
/// gehen Löschung, Upserts und Pull in dieser Reihenfolge hinaus.
#[tokio::test(flavor = "multi_thread")]
async fn sync_now_and_new_settings_retry_unsendable_delete() {
    for trigger in ["sync_now", "sync_config_set"] {
        let recorded = Arc::new(Recorded::default());
        let server = mock(Arc::new(delete_rejecting_server(recorded.clone(), 400))).await;
        let device = device_at(&server.url);
        let video = add_video(&device, "vidAAAAAAA1");
        device
            .conn()
            .execute("UPDATE videos SET published = 1", [])
            .unwrap();
        device
            .conn()
            .execute("DELETE FROM sync_outbox", [])
            .unwrap();
        storage::create_collection(&device.paths, "KI").unwrap();
        storage::delete_video(&device.paths, video).unwrap();
        let status = device.engine.run(Mode::Full).await;
        assert_eq!(status.last_error.as_deref(), Some(BLOCKED_DELETE));
        assert!(BLOCKED_DELETE.ends_with("‚Jetzt synchronisieren‘ versucht es erneut"));
        assert_eq!(recorded.pushes.lock().unwrap().len(), 1);

        // Server „repariert“; die Schleife sendet trotzdem nichts.
        recorded.accept_deletes.store(true, Ordering::SeqCst);
        let mut last_full = None;
        device.engine.tick(&mut last_full).await;
        assert_eq!(recorded.pushes.lock().unwrap().len(), 1, "{trigger}");
        assert_eq!(recorded.pulls.load(Ordering::SeqCst), 0, "{trigger}");
        assert_eq!(device.engine.status().unsendable.len(), 1);

        let status = if trigger == "sync_now" {
            device.engine.sync_now().await
        } else {
            let input = SyncConfigInput {
                enabled: true,
                server_url: server.url.clone(),
                token: None,
                new_videos_local: false,
            };
            device.engine.set_config(&input).await.unwrap();
            let mut last_full = None;
            device.engine.tick(&mut last_full).await;
            device.engine.status()
        };

        assert_eq!(status.last_error, None, "{trigger}: {status:?}");
        assert!(status.unsendable.is_empty(), "{trigger}");
        assert_eq!(status.pending, 0, "{trigger}");
        let pushes = recorded.pushes.lock().unwrap().clone();
        assert_eq!(pushes.len(), 3, "{trigger}: {pushes:?}");
        assert!(pushes[1].contains(r#""type":"videoGone""#), "{trigger}");
        assert!(pushes[2].contains(r#""type":"collection""#), "{trigger}");
        assert_eq!(recorded.pulls.load(Ordering::SeqCst), 1, "{trigger}");
    }
}

/// Weiterhin nicht sendbar: „Jetzt synchronisieren“ markiert neu.
#[tokio::test(flavor = "multi_thread")]
async fn sync_now_remarks_still_unsendable_delete() {
    let recorded = Arc::new(Recorded::default());
    let server = mock(Arc::new(delete_rejecting_server(recorded.clone(), 400))).await;
    let device = device_at(&server.url);
    let video = add_video(&device, "vidAAAAAAA1");
    device
        .conn()
        .execute("UPDATE videos SET published = 1", [])
        .unwrap();
    device
        .conn()
        .execute("DELETE FROM sync_outbox", [])
        .unwrap();
    storage::delete_video(&device.paths, video).unwrap();
    device.engine.run(Mode::Full).await;

    let status = device.engine.sync_now().await;

    assert_eq!(status.last_error.as_deref(), Some(BLOCKED_DELETE));
    assert_eq!(status.unsendable.len(), 1);
    assert_eq!(status.pending, 1);
    assert_eq!(recorded.pushes.lock().unwrap().len(), 2);
    assert_eq!(recorded.pulls.load(Ordering::SeqCst), 0);
}
