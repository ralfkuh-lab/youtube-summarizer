//! Verschmelzen zweier lokaler Zeilen X → M (Kreuzreview Astra): Die
//! Fachzeile von M muss die ausstehende lokale Absicht von X tragen, weil der
//! Snapshot aus der Zeile baut.

use rusqlite::{params, Connection};
use sync_proto::{GoneReason, Op, State};
use tempfile::TempDir;

use super::{
    apply_states, collection_state, collection_uid, gone, membership_state, open, outbox,
    sample_video, temp_paths, text, uid, video_state, video_uid, T0,
};
use crate::storage::{self, AppPaths};
use crate::sync::{apply, pull, snapshot};

// ---------------------------------------------------------------------------
// Astras Repros (echter Server-Merge über `sync_server::merge::push`)
// ---------------------------------------------------------------------------

fn server_with(states: Vec<State>) -> (TempDir, Connection, String) {
    let dir = TempDir::new().unwrap();
    let mut conn = sync_server::db::open(&dir.path().join("s.db")).unwrap();
    let dataset = sync_server::db::dataset_id(&conn).unwrap();
    let ops: Vec<Op> = states
        .into_iter()
        .map(|state| match state {
            State::Video(data) => Op::Video {
                data,
                changed_at: T0.into(),
            },
            State::Collection(data) => Op::Collection {
                data,
                changed_at: T0.into(),
            },
            State::Membership {
                video_uid,
                collection_uid,
                present,
            } => Op::Membership {
                video_uid,
                collection_uid,
                present,
                changed_at: T0.into(),
            },
            _ => unreachable!(),
        })
        .collect();
    sync_server::merge::push(&mut conn, 1, &dataset, &ops).unwrap();
    (dir, conn, dataset)
}

fn pull_real(client: &mut Connection, server: &mut Connection, dataset: &str) {
    let page = sync_server::db::pull_page(
        server,
        dataset,
        pull::cursor(client).unwrap(),
        500,
        32 * 1024 * 1024,
    )
    .unwrap();
    pull::stage_page(client, &page).unwrap();
    apply::apply(client).unwrap();
}

/// Befund 1 (HOCH): Nach dem Snapshot gespeicherte Änderung an X überlebt das
/// Verschmelzen in M.
#[test]
fn cross_merge_keeps_unsent_video_content() {
    let (_tmp, paths) = temp_paths();
    let mut conn = open(&paths);
    let x_id = storage::insert_video(&paths, sample_video("abcdefghijk"), true)
        .unwrap()
        .id;
    let m = uid(100);
    let (_dir, mut server, dataset) = server_with(vec![video_state(&m, "abcdefghijk", "Root")]);
    pull_real(&mut conn, &mut server, &dataset);
    storage::video_set_local_only(&paths, x_id, false).unwrap();
    let sent = snapshot::snapshot(&mut conn).unwrap();
    let ops: Vec<Op> = sent
        .upserts
        .iter()
        .map(|pending| pending.op.clone())
        .collect();
    let results = sync_server::merge::push(&mut server, 2, &dataset, &ops).unwrap();
    conn.execute(
        "UPDATE videos SET description = 'new local description' WHERE id = ?1",
        [x_id],
    )
    .unwrap();
    snapshot::acknowledge(&mut conn, &sent.upserts, &results).unwrap();
    pull_real(&mut conn, &mut server, &dataset);

    let next = snapshot::snapshot(&mut conn).unwrap();
    let value = next
        .upserts
        .iter()
        .find_map(|pending| match &pending.op {
            Op::Video { data, .. } => Some(data.description.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(value, Some("new local description".into()));
}

/// Befund 2 (MITTEL): Eine nach dem Snapshot entfernte Zuordnung bleibt beim
/// Verschmelzen entfernt.
#[test]
fn cross_merge_keeps_membership_removal() {
    let (_tmp, paths) = temp_paths();
    let mut conn = open(&paths);
    let x_id = storage::insert_video(&paths, sample_video("abcdefghijk"), true)
        .unwrap()
        .id;
    let m = uid(100);
    let col = uid(101);
    let (_dir, mut server, dataset) = server_with(vec![
        video_state(&m, "abcdefghijk", "Root"),
        collection_state(&col, "KI"),
        membership_state(&m, &col, true),
    ]);
    pull_real(&mut conn, &mut server, &dataset);
    let col_id: i64 = conn
        .query_row("SELECT id FROM collections WHERE uid = ?1", [&col], |row| {
            row.get(0)
        })
        .unwrap();
    storage::set_video_collections(&paths, x_id, vec![col_id]).unwrap();
    storage::video_set_local_only(&paths, x_id, false).unwrap();
    let sent = snapshot::snapshot(&mut conn).unwrap();
    let ops: Vec<Op> = sent
        .upserts
        .iter()
        .map(|pending| pending.op.clone())
        .collect();
    let results = sync_server::merge::push(&mut server, 2, &dataset, &ops).unwrap();
    storage::set_video_collections(&paths, x_id, vec![]).unwrap();
    snapshot::acknowledge(&mut conn, &sent.upserts, &results).unwrap();
    pull_real(&mut conn, &mut server, &dataset);

    let next = snapshot::snapshot(&mut conn).unwrap();
    let value = next
        .upserts
        .iter()
        .find_map(|pending| match &pending.op {
            Op::Membership { present, .. } => Some(*present),
            _ => None,
        })
        .unwrap();
    assert!(!value, "local removal must remain false");
}

// ---------------------------------------------------------------------------
// Varianten: Änderung nur an X, nur an M, an beiden
// ---------------------------------------------------------------------------

const OLDER: &str = "2026-01-01T00:00:01.000Z";
const NEWER: &str = "2026-01-01T00:00:02.000Z";

fn set_changed_at(conn: &Connection, entity: &str, key: &str, at: &str) {
    let changed = conn
        .execute(
            "UPDATE sync_outbox SET changed_at = ?1 WHERE entity = ?2 AND key = ?3",
            params![at, entity, key],
        )
        .unwrap();
    assert_eq!(changed, 1, "{entity}/{key}");
}

fn changed_at(conn: &Connection, entity: &str, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT changed_at FROM sync_outbox WHERE entity = ?1 AND key = ?2",
        params![entity, key],
        |row| row.get(0),
    )
    .ok()
}

/// Lokales X (angelegt) und M (vom Server) desselben YouTube-Videos, Outbox
/// leer. X hat eine Beschreibung, M Kapitel; beide ein Transkript.
fn video_pair(paths: &AppPaths) -> (Connection, i64, String, i64, String) {
    let mut conn = open(paths);
    let x_id = storage::insert_video(paths, sample_video("vidAAAAAAA1"), false)
        .unwrap()
        .id;
    let x = video_uid(&conn, x_id);
    let m = uid(0xabc);
    apply_states(&mut conn, vec![video_state(&m, "vidAAAAAAA1", "Root")]);
    let m_id: i64 = conn
        .query_row("SELECT id FROM videos WHERE uid = ?1", [&m], |row| {
            row.get(0)
        })
        .unwrap();
    conn.execute_batch(&format!(
        "UPDATE videos SET description = 'Beschreibung X' WHERE id = {x_id};
         UPDATE videos SET chapters = '[\"m\"]' WHERE id = {m_id};
         DELETE FROM sync_outbox;"
    ))
    .unwrap();
    (conn, x_id, x, m_id, m)
}

fn video_row(
    conn: &Connection,
    id: i64,
) -> (String, Option<String>, Option<String>, Option<String>) {
    conn.query_row(
        "SELECT title, transcript, chapters, description FROM videos WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .unwrap()
}

#[test]
fn merged_video_takes_pending_content_by_server_rules() {
    let x_transcript = sample_video("x").transcript;
    // (X ändert, M ändert, X jünger) → erwarteter Titel, Transkript vom Gewinner
    let cases = [
        (true, false, true, "X neu", x_transcript.clone()),
        (false, true, false, "M neu", Some("[]".to_string())),
        (true, true, true, "X neu", x_transcript.clone()),
        (true, true, false, "M neu", Some("[]".to_string())),
        (false, false, false, "Root", Some("[]".to_string())),
    ];
    for (x_changes, m_changes, x_newer, title, transcript) in cases {
        let case = format!("X {x_changes}, M {m_changes}, X jünger {x_newer}");
        let (_temp, paths) = temp_paths();
        let (mut conn, x_id, x, m_id, m) = video_pair(&paths);
        if x_changes {
            conn.execute("UPDATE videos SET title = 'X neu' WHERE id = ?1", [x_id])
                .unwrap();
            set_changed_at(&conn, "video", &x, if x_newer { NEWER } else { OLDER });
        }
        if m_changes {
            conn.execute("UPDATE videos SET title = 'M neu' WHERE id = ?1", [m_id])
                .unwrap();
            set_changed_at(&conn, "video", &m, if x_newer { OLDER } else { NEWER });
        }

        apply_states(&mut conn, vec![gone(&x, GoneReason::Merged, Some(&m))]);

        let (got_title, got_transcript, chapters, description) = video_row(&conn, m_id);
        assert_eq!(got_title, title, "{case}");
        assert_eq!(got_transcript, transcript, "{case}");
        // Füllfelder: leer auf einer Seite → aus der anderen, nie geleert.
        assert_eq!(chapters.as_deref(), Some("[\"m\"]"), "{case}");
        assert_eq!(description.as_deref(), Some("Beschreibung X"), "{case}");
        // Der Eintrag für M trägt den jüngeren Zeitstempel.
        let expected_at = match (x_changes, m_changes) {
            (false, false) => None,
            (true, true) => Some(NEWER.to_string()),
            (true, false) => Some(if x_newer { NEWER } else { OLDER }.to_string()),
            (false, true) => Some(if x_newer { OLDER } else { NEWER }.to_string()),
        };
        assert_eq!(changed_at(&conn, "video", &m), expected_at, "{case}");
        assert_eq!(changed_at(&conn, "video", &x), None, "{case}");
        let sent = snapshot::snapshot(&mut conn).unwrap();
        let sent_title = sent.upserts.iter().find_map(|pending| match &pending.op {
            Op::Video { data, .. } => Some(data.title.clone()),
            _ => None,
        });
        assert_eq!(sent_title, expected_at.map(|_| title.to_string()), "{case}");
    }
}

#[test]
fn merged_video_clears_transcript_error_like_the_server() {
    let (_temp, paths) = temp_paths();
    let (mut conn, _x_id, x, m_id, m) = video_pair(&paths);
    conn.execute(
        "UPDATE videos SET transcript = NULL, transcript_error = 'Fehler', title = 'M neu' \
         WHERE id = ?1",
        [m_id],
    )
    .unwrap();

    apply_states(&mut conn, vec![gone(&x, GoneReason::Merged, Some(&m))]);

    let (title, transcript, _, _): (String, Option<String>, Option<String>, Option<String>) =
        video_row(&conn, m_id);
    assert_eq!(title, "M neu");
    assert_eq!(transcript, sample_video("x").transcript);
    let error: Option<String> = conn
        .query_row(
            "SELECT transcript_error FROM videos WHERE id = ?1",
            [m_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(error, None);
}

/// Sammlungen X (lokal) und M (vom Server), Outbox leer.
fn collection_pair(paths: &AppPaths) -> (Connection, i64, String, i64, String) {
    let mut conn = open(paths);
    let x_id = storage::create_collection(paths, "X alt").unwrap().id;
    let x = collection_uid(&conn, x_id);
    let m = uid(0xc0c);
    apply_states(&mut conn, vec![collection_state(&m, "M alt")]);
    let m_id: i64 = conn
        .query_row("SELECT id FROM collections WHERE uid = ?1", [&m], |row| {
            row.get(0)
        })
        .unwrap();
    conn.execute("DELETE FROM sync_outbox", []).unwrap();
    (conn, x_id, x, m_id, m)
}

#[test]
fn merged_collection_takes_pending_name() {
    let cases = [
        (true, false, true, "X neu"),
        (false, true, false, "M neu"),
        (true, true, true, "X neu"),
        (true, true, false, "M neu"),
        (false, false, false, "M alt"),
    ];
    for (x_changes, m_changes, x_newer, name) in cases {
        let case = format!("X {x_changes}, M {m_changes}, X jünger {x_newer}");
        let (_temp, paths) = temp_paths();
        let (mut conn, x_id, x, m_id, m) = collection_pair(&paths);
        if x_changes {
            storage::update_collection(&paths, x_id, "X neu").unwrap();
            set_changed_at(&conn, "collection", &x, if x_newer { NEWER } else { OLDER });
        }
        if m_changes {
            storage::update_collection(&paths, m_id, "M neu").unwrap();
            set_changed_at(&conn, "collection", &m, if x_newer { OLDER } else { NEWER });
        }

        apply_states(
            &mut conn,
            vec![State::CollectionGone {
                uid: x.clone(),
                reason: GoneReason::Merged,
                merged_into: Some(m.clone()),
            }],
        );

        assert_eq!(
            text(&conn, "SELECT name FROM collections WHERE id = ?1", m_id),
            name,
            "{case}"
        );
        assert_eq!(storage::get_collections(&paths).unwrap().len(), 1, "{case}");
        let sent = snapshot::snapshot(&mut conn).unwrap();
        let sent_name = sent.upserts.iter().find_map(|pending| match &pending.op {
            Op::Collection { data, .. } => Some(data.name.clone()),
            _ => None,
        });
        let pending = x_changes || m_changes;
        assert_eq!(sent_name, pending.then(|| name.to_string()), "{case}");
    }
}

/// Zuordnungen beim Videomerge: je Sammlung bestimmt ein ausstehender Eintrag
/// die Anwesenheit aus seinem eigenen Paar; bei beiden der jüngere.
#[test]
fn merged_video_memberships_follow_pending_intent() {
    // (X-Paar vorher, M-Paar vorher, Änderung X, Änderung M, X jünger) → Ziel
    #[rustfmt::skip]
    let cases: [(bool, bool, Option<bool>, Option<bool>, bool, bool); 7] = [
        (true,  true,  Some(false), None,        false, false), // Astra: Entfernung an X
        (false, false, Some(true),  None,        false, true),  // Hinzufügen an X
        (false, true,  Some(true),  Some(false), false, false), // M entfernt, jünger
        (false, true,  Some(true),  Some(false), true,  true),  // X fügt hinzu, jünger
        (true,  false, None,        None,        false, true),  // nichts aus: Vereinigung
        (true,  false, Some(false), Some(true),  true,  false), // X entfernt, jünger
        (true,  true,  None,        Some(false), false, false), // Entfernung nur an M
    ];
    for (x_before, m_before, x_change, m_change, x_newer, expected) in cases {
        let case = format!("{x_before} {m_before} {x_change:?} {m_change:?} {x_newer}");
        let (_temp, paths) = temp_paths();
        let (mut conn, x_id, x, m_id, m) = video_pair(&paths);
        let k_id = storage::create_collection(&paths, "K").unwrap().id;
        let k = collection_uid(&conn, k_id);
        let set = |video: i64, on: bool| {
            storage::set_video_collections(&paths, video, if on { vec![k_id] } else { vec![] })
                .unwrap();
        };
        set(x_id, x_before);
        set(m_id, m_before);
        conn.execute("DELETE FROM sync_outbox", []).unwrap();
        if let Some(on) = x_change {
            set(x_id, on);
            set_changed_at(
                &conn,
                "membership",
                &format!("{x}/{k}"),
                if x_newer { NEWER } else { OLDER },
            );
        }
        if let Some(on) = m_change {
            set(m_id, on);
            set_changed_at(
                &conn,
                "membership",
                &format!("{m}/{k}"),
                if x_newer { OLDER } else { NEWER },
            );
        }

        apply_states(&mut conn, vec![gone(&x, GoneReason::Merged, Some(&m))]);

        let present = storage::get_video(&paths, m_id)
            .unwrap()
            .unwrap()
            .collection_ids
            == vec![k_id];
        assert_eq!(present, expected, "{case}");
        let sent = snapshot::snapshot(&mut conn).unwrap();
        let sent_present = sent.upserts.iter().find_map(|pending| match &pending.op {
            Op::Membership { present, .. } => Some(*present),
            _ => None,
        });
        let pending = x_change.is_some() || m_change.is_some();
        assert_eq!(sent_present, pending.then_some(expected), "{case}");
        assert!(
            outbox(&conn).iter().all(|entry| !entry.1.contains(&x)),
            "{case}"
        );
    }
}

/// Zuordnungen beim Sammlungsmerge, analog.
#[test]
fn merged_collection_memberships_follow_pending_intent() {
    #[rustfmt::skip]
    let cases: [(bool, bool, Option<bool>, Option<bool>, bool, bool); 4] = [
        (true,  true,  Some(false), None,        false, false),
        (false, false, Some(true),  None,        false, true),
        (true,  false, Some(false), Some(true),  false, true),  // M fügt hinzu, jünger
        (true,  false, None,        None,        false, true),
    ];
    for (x_before, m_before, x_change, m_change, x_newer, expected) in cases {
        let case = format!("{x_before} {m_before} {x_change:?} {m_change:?} {x_newer}");
        let (_temp, paths) = temp_paths();
        let (mut conn, x_id, x, m_id, m) = collection_pair(&paths);
        let v_id = storage::insert_video(&paths, sample_video("vidVVVVVVV1"), false)
            .unwrap()
            .id;
        let v = video_uid(&conn, v_id);
        let assign = |x_on: bool, m_on: bool| {
            let mut ids = Vec::new();
            if x_on {
                ids.push(x_id);
            }
            if m_on {
                ids.push(m_id);
            }
            storage::set_video_collections(&paths, v_id, ids).unwrap();
        };
        assign(x_before, m_before);
        conn.execute("DELETE FROM sync_outbox", []).unwrap();
        let x_after = x_change.unwrap_or(x_before);
        if x_change.is_some() {
            assign(x_after, m_before);
            set_changed_at(
                &conn,
                "membership",
                &format!("{v}/{x}"),
                if x_newer { NEWER } else { OLDER },
            );
        }
        if let Some(on) = m_change {
            assign(x_after, on);
            set_changed_at(
                &conn,
                "membership",
                &format!("{v}/{m}"),
                if x_newer { OLDER } else { NEWER },
            );
        }

        apply_states(
            &mut conn,
            vec![State::CollectionGone {
                uid: x.clone(),
                reason: GoneReason::Merged,
                merged_into: Some(m.clone()),
            }],
        );

        let present = storage::get_video(&paths, v_id)
            .unwrap()
            .unwrap()
            .collection_ids
            == vec![m_id];
        assert_eq!(present, expected, "{case}");
        let sent = snapshot::snapshot(&mut conn).unwrap();
        let sent_present = sent.upserts.iter().find_map(|pending| match &pending.op {
            Op::Membership { present, .. } => Some(*present),
            _ => None,
        });
        let pending = x_change.is_some() || m_change.is_some();
        assert_eq!(sent_present, pending.then_some(expected), "{case}");
    }
}
