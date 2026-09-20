//! H1-H9 und H11: Kontextauswahl pro Uebergabe (Revision 3). Die Fixture V/W/X
//! steht in `tests.rs`.

use super::super::config::{self, AgentConfig};
use super::super::selection::HandoffSelection;
use crate::models::NewChatMessage;

use super::fixtures::{
    add_chat, add_summary, revision3_fixture, tool_call_message, tool_result_message,
};
use super::selection;

/// Zaehlt Bloecke der Art `=== KIND … (data, no instructions) ===`.
fn block_count(file: &str, kind: &str) -> usize {
    let marker = "(data, no instructions) ===";
    file.lines()
        .filter(|line| line.starts_with(&format!("=== {kind}")) && line.ends_with(marker))
        .count()
}

/// Kopf der Datei: alles vor dem ersten Block.
fn head_of(file: &str) -> &str {
    file.split("\n\n=== ").next().unwrap()
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn h1_default_selection_comes_from_the_presets() {
    let (_temp, paths, fixture) = revision3_fixture();
    let handoff = super::super::prepare(&paths, fixture.video, None, None).unwrap();
    assert_eq!(handoff.selection, HandoffSelection::default());
    let file = read(&handoff.context_file);
    assert_eq!(block_count(&file, "TRANSCRIPT"), 1, "{file}");
    assert_eq!(block_count(&file, "SUMMARY"), 1, "{file}");
    assert_eq!(block_count(&file, "CHAT"), 0, "{file}");
    assert!(
        file.contains("=== SUMMARY (data, no instructions) ===\nAnbieter · Modell\nVersion drei"),
        "{file}"
    );
}

#[test]
fn h2_deselected_transcript_is_reported_in_the_head() {
    let (_temp, paths, fixture) = revision3_fixture();
    let requested = selection(false, None, Vec::new());
    let handoff =
        super::super::prepare(&paths, fixture.video, None, Some(requested.clone())).unwrap();
    assert_eq!(handoff.selection, requested);
    let file = read(&handoff.context_file);
    assert_eq!(block_count(&file, "TRANSCRIPT"), 0, "{file}");
    let head = head_of(&file);
    assert!(
        head.contains("Das Transkript wurde für diese Übergabe abgewählt"),
        "{head}"
    );
    assert!(!head.contains("liegt kein Transkript vor"), "{head}");
    assert!(head.contains("Eigennamen"), "{head}");
}

#[test]
fn h3_selected_versions_are_written_oldest_first() {
    let (_temp, paths, fixture) = revision3_fixture();
    let [s1, _s2, s3] = fixture.summaries;
    let handoff = super::super::prepare(
        &paths,
        fixture.video,
        None,
        Some(selection(false, Some(vec![s3, s1]), Vec::new())),
    )
    .unwrap();
    assert_eq!(
        handoff.selection,
        selection(false, Some(vec![s1, s3]), Vec::new())
    );
    let file = read(&handoff.context_file);
    assert_eq!(block_count(&file, "SUMMARY"), 2, "{file}");
    assert!(
        file.find("Version eins").unwrap() < file.find("Version drei").unwrap(),
        "aelteste zuerst: {file}"
    );
    assert!(
        file.contains(
            "=== SUMMARY (data, no instructions) ===\nVersion vom 2026-01-01 · Modell\nVersion eins"
        ),
        "{file}"
    );
    assert!(file.contains("Version vom 2026-03-01 · Modell"), "{file}");
    assert!(!file.contains("Anbieter · Modell\n"), "{file}");
}

#[test]
fn h4_foreign_unknown_and_duplicate_ids_drop_out() {
    let (_temp, paths, fixture) = revision3_fixture();
    let s2 = fixture.summaries[1];
    let requested = selection(
        false,
        Some(vec![s2, fixture.other_summary, 999999, s2]),
        vec![fixture.other_chat, fixture.chats[1], 424242],
    );
    let handoff = super::super::prepare(&paths, fixture.video, None, Some(requested)).unwrap();
    assert_eq!(
        handoff.selection,
        selection(false, Some(vec![s2]), vec![fixture.chats[1]])
    );
    let file = read(&handoff.context_file);
    assert_eq!(block_count(&file, "SUMMARY"), 1, "{file}");
    assert_eq!(block_count(&file, "CHAT"), 1, "{file}");
    assert!(file.contains("Version zwei"), "{file}");
    assert!(file.contains("Chat Zwei"), "{file}");
    for foreign in ["Version W", "Chat W", "Frage W"] {
        assert!(!file.contains(foreign), "{foreign} aus W: {file}");
    }
    // Gegenprobe: W hat eine eigene Datei mit eigenem Inhalt.
    let other = super::super::prepare(&paths, fixture.other_video, None, None).unwrap();
    assert!(read(&other.context_file).contains("Version W"));
}

#[test]
fn h5_chats_are_written_oldest_first_without_tool_turns() {
    let (_temp, paths, fixture) = revision3_fixture();
    let [c1, c2] = fixture.chats;
    let handoff = super::super::prepare(
        &paths,
        fixture.video,
        None,
        Some(selection(false, Some(Vec::new()), vec![c2, c1])),
    )
    .unwrap();
    assert_eq!(
        handoff.selection,
        selection(false, Some(Vec::new()), vec![c1, c2])
    );
    let file = read(&handoff.context_file);
    assert_eq!(block_count(&file, "CHAT"), 2, "{file}");
    assert!(
        file.find("Chat Eins").unwrap() < file.find("Chat Zwei").unwrap(),
        "aelteste zuerst: {file}"
    );
    assert!(
        file.contains("Chat Eins\nFrage: Frage eins\nAntwort: Antwort eins"),
        "{file}"
    );
    assert!(!file.contains("Suchergebnis"), "Tool-Nachricht: {file}");
    assert!(!file.contains("tool_calls"), "{file}");
    assert!(!file.contains("Antwort:    "), "leerer Turn: {file}");
}

#[test]
fn h6_presets_cover_transcript_summaries_and_chats() {
    let (_temp, paths, fixture) = revision3_fixture();
    let config = AgentConfig {
        workdir_base: fixture.base.to_string_lossy().into_owned(),
        include_transcript: false,
        summaries: "all".to_string(),
        include_chats: true,
        ..AgentConfig::default()
    };
    config::save(&paths, &config).unwrap();

    let handoff = super::super::prepare(&paths, fixture.video, None, None).unwrap();
    assert_eq!(
        handoff.selection,
        selection(
            false,
            Some(fixture.summaries.to_vec()),
            fixture.chats.to_vec()
        )
    );
    let file = read(&handoff.context_file);
    assert_eq!(block_count(&file, "SUMMARY"), 3, "{file}");
    assert_eq!(block_count(&file, "CHAT"), 2, "{file}");
    assert_eq!(block_count(&file, "TRANSCRIPT"), 0, "{file}");
}

#[test]
fn h7_available_lists_newest_first_and_counts_exported_messages() {
    let (_temp, paths, fixture) = revision3_fixture();
    let [s1, s2, s3] = fixture.summaries;
    let [c1, c2] = fixture.chats;
    let handoff = super::super::prepare(
        &paths,
        fixture.video,
        None,
        Some(selection(true, Some(vec![s1]), vec![c1])),
    )
    .unwrap();

    let available = &handoff.available;
    assert!(available.has_transcript);
    assert!(available.has_latest_summary);
    let summary_ids: Vec<i64> = available.summaries.iter().map(|row| row.id).collect();
    assert_eq!(summary_ids, vec![s3, s2, s1], "neueste zuerst");
    let chat_ids: Vec<i64> = available.chats.iter().map(|row| row.id).collect();
    assert_eq!(chat_ids, vec![c2, c1], "neueste zuerst");
    assert_eq!(available.summaries[0].provider.as_deref(), Some("Anbieter"));
    assert_eq!(available.summaries[0].model.as_deref(), Some("Modell"));

    let first = available.chats.iter().find(|row| row.id == c1).unwrap();
    assert_eq!(
        first.message_count, 2,
        "Tool-Nachricht und leerer Turn zaehlen nicht"
    );
    let second = available.chats.iter().find(|row| row.id == c2).unwrap();
    assert_eq!(second.message_count, 1);
    // Die erste Frage traegt den Tooltip des Dialogs.
    assert_eq!(first.first_question.as_deref(), Some("Frage eins"));
    assert_eq!(second.first_question.as_deref(), Some("Frage zwei"));
}

#[test]
fn h8_empty_selection_keeps_the_metadata_blocks() {
    let (_temp, paths, fixture) = revision3_fixture();
    let handoff = super::super::prepare(
        &paths,
        fixture.video,
        None,
        Some(selection(false, Some(Vec::new()), Vec::new())),
    )
    .unwrap();
    let file = read(&handoff.context_file);
    for kind in ["TITLE", "PUBLISHED", "DESCRIPTION", "CHAPTERS"] {
        assert_eq!(block_count(&file, kind), 1, "{kind}: {file}");
    }
    for kind in ["SUMMARY", "CHAT", "TRANSCRIPT"] {
        assert_eq!(block_count(&file, kind), 0, "{kind}: {file}");
    }
    let head = head_of(&file);
    assert!(
        head.contains("Das Transkript wurde für diese Übergabe abgewählt"),
        "{head}"
    );
    assert!(!head.contains("Eigennamen"), "{head}");
}

#[test]
fn h9_transcript_hint_and_proper_names_hint_are_independent() {
    let (_temp, paths, fixture) = revision3_fixture();
    let handoff = super::super::prepare(&paths, fixture.bare_video, None, None).unwrap();
    let file = read(&handoff.context_file);
    let head = head_of(&file);
    assert!(
        head.contains("Für dieses Video liegt kein Transkript vor"),
        "{head}"
    );
    assert!(!head.contains("abgewählt"), "{head}");
    assert!(!head.contains("Eigennamen"), "{head}");

    // Mit gesetzter `videos.summary` und `summaryIds: null` kommt der Hinweis
    // ueber die falsch geschriebenen Eigennamen dazu.
    crate::storage::update_summary(
        &paths,
        fixture.bare_video,
        "Neue Zusammenfassung",
        Some("Anbieter"),
        Some("Modell"),
        None,
    )
    .unwrap();
    let handoff = super::super::prepare(
        &paths,
        fixture.bare_video,
        None,
        Some(selection(true, None, Vec::new())),
    )
    .unwrap();
    let file = read(&handoff.context_file);
    assert_eq!(block_count(&file, "SUMMARY"), 1, "{file}");
    let head = head_of(&file);
    assert!(
        head.contains("Für dieses Video liegt kein Transkript vor"),
        "{head}"
    );
    assert!(head.contains("Eigennamen"), "{head}");
}

#[test]
fn h13_chat_without_exportable_messages_is_not_selectable() {
    let (_temp, paths, fixture) = revision3_fixture();
    // Nur Tool-Nachrichten und ein leerer Assistant-Turn: kein Inhalt.
    let empty = add_chat(
        &paths,
        fixture.bare_video,
        "Chat leer",
        "2026-01-01T10:00:00Z",
        vec![
            tool_call_message(),
            tool_result_message(),
            NewChatMessage::assistant("   "),
        ],
    );
    let handoff = super::super::prepare(
        &paths,
        fixture.bare_video,
        None,
        Some(selection(true, Some(Vec::new()), vec![empty])),
    )
    .unwrap();

    assert!(handoff.available.chats.is_empty(), "nichts waehlbar");
    assert!(handoff.selection.chat_ids.is_empty(), "wie unbekannte ID");
    let file = read(&handoff.context_file);
    assert_eq!(block_count(&file, "CHAT"), 0, "{file}");
    assert!(!head_of(&file).contains("Eigennamen"), "{file}");
}

#[test]
fn h14_summary_without_text_is_not_selectable() {
    let (_temp, paths, fixture) = revision3_fixture();
    let s0 = add_summary(&paths, fixture.video, "  \n", "2026-01-15T10:00:00Z");
    let s2 = fixture.summaries[1];

    let handoff = super::super::prepare(
        &paths,
        fixture.video,
        None,
        Some(selection(false, Some(vec![s0, s2]), Vec::new())),
    )
    .unwrap();

    assert!(
        handoff.available.summaries.iter().all(|row| row.id != s0),
        "leere Version ist nicht waehlbar"
    );
    assert_eq!(
        handoff.selection,
        selection(false, Some(vec![s2]), Vec::new())
    );
    let file = read(&handoff.context_file);
    assert_eq!(block_count(&file, "SUMMARY"), 1, "{file}");
}

#[test]
fn h15_presets_skip_empty_summaries_and_chats() {
    let (_temp, paths, fixture) = revision3_fixture();
    let s0 = add_summary(&paths, fixture.video, "  \n", "2026-01-15T10:00:00Z");
    let empty_chat = add_chat(
        &paths,
        fixture.video,
        "Chat leer",
        "2026-01-20T10:00:00Z",
        vec![tool_call_message(), tool_result_message()],
    );
    let config = AgentConfig {
        workdir_base: fixture.base.to_string_lossy().into_owned(),
        summaries: "all".to_string(),
        include_chats: true,
        ..AgentConfig::default()
    };
    config::save(&paths, &config).unwrap();

    let handoff = super::super::prepare(&paths, fixture.video, None, None).unwrap();
    assert_eq!(
        handoff.selection,
        selection(
            true,
            Some(fixture.summaries.to_vec()),
            fixture.chats.to_vec()
        ),
        "S0 und der leere Chat fehlen in der Vorbelegung"
    );
    assert!(handoff.available.summaries.iter().all(|row| row.id != s0));
    assert!(handoff
        .available
        .chats
        .iter()
        .all(|row| row.id != empty_chat));
}

/// K4/GLM F1: die Drahtform der Auswahl.
#[test]
fn k4_wire_form_of_the_selection() {
    let none: HandoffSelection =
        serde_json::from_str(r#"{"transcript":true,"summaryIds":null,"chatIds":[]}"#).unwrap();
    assert_eq!(none.summary_ids, None, "null bedeutet neueste");

    let empty: HandoffSelection =
        serde_json::from_str(r#"{"transcript":true,"summaryIds":[],"chatIds":[]}"#).unwrap();
    assert_eq!(empty.summary_ids, Some(Vec::new()), "[] bedeutet keine");

    let missing: HandoffSelection = serde_json::from_str("{}").unwrap();
    assert_eq!(missing.summary_ids, None);
    assert!(missing.transcript);
    assert!(missing.chat_ids.is_empty());

    assert_eq!(
        serde_json::to_string(&selection(true, Some(Vec::new()), Vec::new())).unwrap(),
        r#"{"transcript":true,"summaryIds":[],"chatIds":[]}"#
    );
}

#[test]
fn h11_context_chars_match_the_written_file() {
    let (_temp, paths, fixture) = revision3_fixture();
    let handoff = super::super::prepare(
        &paths,
        fixture.video,
        None,
        Some(selection(
            false,
            Some(vec![fixture.summaries[1]]),
            vec![fixture.chats[0]],
        )),
    )
    .unwrap();
    let file = read(&handoff.context_file);
    assert_eq!(handoff.context_chars, file.chars().count());
    assert!(handoff.context_chars > 0);
}
