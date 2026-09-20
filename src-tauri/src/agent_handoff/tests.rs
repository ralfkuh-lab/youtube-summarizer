//! Referenzfaelle aus `docs/spec-agent-handoff.md`: A1-A3, A5, A6, die
//! Konfigurationsfaelle und `agent_preview`. Maskierung/Aufloesung und
//! Kontextdatei liegen in den Untermodulen.

mod context_tests;
mod fixtures;
mod prepare_tests;
mod quoting_tests;
mod selection_tests;

use std::collections::BTreeMap;

use tempfile::TempDir;

use crate::models::{Chat, ChatMessageRecord, NewVideo, Summary, Video};
use crate::storage::AppPaths;

use super::config::{self, AgentConfig, AgentTemplate, DEFAULT_PROMPT, MAX_PROMPT_CHARS};
use super::context::{self, ContextSources};
use super::quote::Shell;
use super::resolve::{normalize_prompt, slug, title_slug, Values};
use super::selection::{self, HandoffSelection};

pub(crate) fn temp_paths() -> (TempDir, AppPaths) {
    let temp = TempDir::new().unwrap();
    let paths = AppPaths {
        db_path: temp.path().join("videos.db"),
        config_path: temp.path().join("config.json"),
    };
    crate::storage::init_db(&paths).unwrap();
    (temp, paths)
}

pub(crate) const EXPORTED: &str = "2026-09-19T12:00:00Z";

pub(crate) fn render(video: &Video) -> String {
    render_with(
        video,
        &[],
        &[],
        &BTreeMap::new(),
        &HandoffSelection::default(),
    )
}

pub(crate) fn render_with(
    video: &Video,
    summaries: &[Summary],
    chats: &[Chat],
    messages: &BTreeMap<i64, Vec<ChatMessageRecord>>,
    selection: &HandoffSelection,
) -> String {
    let resolved = selection::resolve(selection, summaries, chats);
    context::render(&ContextSources {
        video,
        summaries: &resolved.summaries,
        chats: &resolved.chats,
        messages,
        selection: &resolved.selection,
        exported_at: EXPORTED,
    })
}

pub(crate) fn integration_video(video_id: &str, title: &str) -> NewVideo {
    NewVideo {
        video_id: video_id.to_string(),
        url: format!("https://www.youtube.com/watch?v={video_id}"),
        title: title.to_string(),
        thumbnail_url: "https://example.com/t.jpg".to_string(),
        thumbnail_data: None,
        transcript: Some(r#"[{"text":"hallo welt","start":0.0,"time":"0:00"}]"#.to_string()),
        chapters: None,
        published_at: None,
        description: None,
        transcript_error: None,
    }
}

/// Auswahl mit allen Feldern gesetzt (Serde-Defaults sind in den Tests nicht
/// gemeint).
pub(crate) fn selection(
    transcript: bool,
    summary_ids: Option<Vec<i64>>,
    chat_ids: Vec<i64>,
) -> HandoffSelection {
    HandoffSelection {
        transcript,
        summary_ids,
        chat_ids,
    }
}

pub(crate) fn store_config(
    paths: &crate::storage::AppPaths,
    base: &std::path::Path,
    template: &str,
) {
    let config = AgentConfig {
        workdir_base: base.to_string_lossy().into_owned(),
        active_template: "test".to_string(),
        custom_templates: vec![AgentTemplate {
            id: "test".to_string(),
            name: "Test".to_string(),
            command: template.to_string(),
        }],
        ..AgentConfig::default()
    };
    config::save(paths, &config).unwrap();
}

pub(crate) fn read_context(paths: &crate::storage::AppPaths, video_id: i64) -> String {
    let handoff = super::prepare(paths, video_id, None, None).unwrap();
    std::fs::read_to_string(handoff.context_file).unwrap()
}

// --------------------------------------------------------------------------- A --

pub(crate) fn sample_video() -> Video {
    Video {
        id: 1,
        video_id: "dQw4w9WgXcQ".to_string(),
        url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string(),
        title: "Titel".to_string(),
        thumbnail_url: "https://example.com/t.jpg".to_string(),
        thumbnail: None,
        transcript: Some(r#"[{"text":"hallo","start":0.0,"time":"0:00"}]"#.to_string()),
        chapters: None,
        summary: None,
        summary_provider: None,
        summary_model: None,
        published_at: None,
        description: None,
        collection_ids: Vec::new(),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        transcript_error: None,
        has_transcript: true,
        has_summary: false,
    }
}

pub(crate) fn sample_values() -> Values {
    let workdir = "/tmp/wd".to_string();
    let context_file = format!("{workdir}/context.md");
    Values {
        workdir,
        context_file: context_file.clone(),
        prompt: normalize_prompt(DEFAULT_PROMPT, &context_file),
        video_id: "dQw4w9WgXcQ".to_string(),
        video_url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_string(),
        db_path: "/tmp/videos.db".to_string(),
    }
}

fn custom_template(id: &str, command: &str) -> AgentTemplate {
    AgentTemplate {
        id: id.to_string(),
        name: "Mein Agent".to_string(),
        command: command.to_string(),
    }
}

// --------------------------------------------------------------------- Slug A1-A6 --

#[test]
fn a1_slug_from_title_and_id() {
    assert_eq!(
        slug("Jev explained in 7min..", "abc_-123"),
        "jev-explained-in-7min-abc_-123"
    );
}

#[test]
fn a2_nfc_and_nfd_titles_yield_the_same_slug() {
    let nfc = "Über Größe & \"Quotes\"; rm -rf ~";
    let nfd = "U\u{308}ber Gro\u{308}\u{df}e & \"Quotes\"; rm -rf ~";
    assert_eq!(title_slug(nfc), "ueber-groesse-quotes-rm-rf");
    assert_eq!(title_slug(nfd), "ueber-groesse-quotes-rm-rf");
    assert_eq!(title_slug(nfd), title_slug(nfc));
    assert_eq!(slug(nfc, "vid"), "ueber-groesse-quotes-rm-rf-vid");
}

#[test]
fn a3_title_without_letters_falls_back_to_video_id() {
    assert_eq!(title_slug("🎬🎥"), "");
    assert_eq!(slug("🎬🎥", "vid"), "vid");
    assert_eq!(slug("   ", "vid"), "vid");
}

#[test]
fn a5_truncation_does_not_leave_dangling_dashes() {
    let title = format!("{}-b", "a".repeat(59));
    let slugged = title_slug(&title);
    assert_eq!(slugged, "a".repeat(59));
    let full = slug(&title, "vid");
    assert!(!full.contains("--"), "{full}");
    assert!(!full.starts_with('-'), "{full}");
    assert_eq!(full, format!("{}-vid", "a".repeat(59)));
}

#[test]
fn a6_windows_reserved_names_get_a_prefix() {
    assert_eq!(slug("CON", "vid1"), "v-con-vid1");
    assert_eq!(slug("nul", "vid1"), "v-nul-vid1");
    assert_eq!(slug("com7", "vid1"), "v-com7-vid1");
    assert_eq!(slug("Konsole", "vid1"), "konsole-vid1");
    // Auch der reine ID-Fall (Titel ohne Buchstaben/Ziffern) wird entschaerft.
    assert_eq!(slug("🎬", "con"), "v-con");
    assert_eq!(slug("   ", "nul"), "v-nul");
    assert_eq!(slug("🎬🎥", "com1"), "v-com1");
    assert_eq!(slug("🎬", "vid1"), "vid1");
}

#[test]
fn video_id_rule() {
    assert!(super::resolve::valid_video_id("abc_-123"));
    assert!(super::resolve::valid_video_id(&"a".repeat(32)));
    assert!(!super::resolve::valid_video_id(""));
    assert!(!super::resolve::valid_video_id(&"a".repeat(33)));
    assert!(!super::resolve::valid_video_id("../../etc"));
    assert!(!super::resolve::valid_video_id("a b"));
    assert!(!super::resolve::valid_video_id("ä"));
}

#[test]
fn workdir_base_resolution() {
    let home = std::path::Path::new("/home/test");
    assert_eq!(
        super::resolve::resolve_workdir_base("", home),
        std::path::PathBuf::from("/home/test/yt-agent")
    );
    assert_eq!(
        super::resolve::resolve_workdir_base("~", home),
        std::path::PathBuf::from("/home/test")
    );
    assert_eq!(
        super::resolve::resolve_workdir_base("~/yt", home),
        std::path::PathBuf::from("/home/test/yt")
    );
    assert_eq!(
        super::resolve::resolve_workdir_base("/srv/yt agent/it's", home),
        std::path::PathBuf::from("/srv/yt agent/it's")
    );
    // Relative Angaben gelten ab dem Home-Verzeichnis (H2), auch ein
    // woertliches `~user`, das nicht aufgeloest wird.
    assert_eq!(
        super::resolve::resolve_workdir_base("yt-agent", home),
        std::path::PathBuf::from("/home/test/yt-agent")
    );
    assert_eq!(
        super::resolve::resolve_workdir_base("unter/ordner", home),
        std::path::PathBuf::from("/home/test/unter/ordner")
    );
    assert_eq!(
        super::resolve::resolve_workdir_base("~user/yt", home),
        std::path::PathBuf::from("/home/test/~user/yt")
    );
    for base in ["", "~", "~/yt", "/srv/yt", "yt-agent", "~user/yt"] {
        assert!(
            super::resolve::resolve_workdir_base(base, home).is_absolute(),
            "{base:?} muss absolut sein"
        );
    }
}

// ------------------------------------------------------------------ Konfiguration --

#[test]
fn config_missing_file_yields_defaults() {
    let (_temp, paths) = temp_paths();
    let config = config::load(&paths);
    assert_eq!(config, AgentConfig::default());
    assert_eq!(config.shell, "auto");
    assert_eq!(config.summaries, "latest");
    assert_eq!(config.active_template, "claude");
    assert_eq!(config.effective_prompt(), DEFAULT_PROMPT);
    assert!(config.custom_templates.is_empty());
}

#[test]
fn config_broken_or_empty_file_yields_defaults() {
    let (_temp, paths) = temp_paths();
    let path = config::config_path(&paths);
    assert_eq!(path.file_name().unwrap(), "agent.json");

    std::fs::write(&path, "{kaputt").unwrap();
    assert_eq!(config::load(&paths), AgentConfig::default());
    std::fs::write(&path, "   ").unwrap();
    assert_eq!(config::load(&paths), AgentConfig::default());
}

#[test]
fn config_roundtrip_is_atomic_and_stores_all_fields() {
    let (_temp, paths) = temp_paths();
    let stored = AgentConfig {
        workdir_base: "~/yt".to_string(),
        shell: "fish".to_string(),
        summaries: "all".to_string(),
        include_transcript: false,
        include_chats: true,
        prompt: "Lies {context_file}".to_string(),
        active_template: "mein-agent".to_string(),
        custom_templates: vec![custom_template(
            "mein-agent",
            "cd {workdir} && agent {prompt}",
        )],
    };
    let saved = config::save(&paths, &stored).unwrap();
    assert_eq!(saved, stored);
    assert_eq!(config::load(&paths), stored);
    assert!(config::config_path(&paths).exists());
}

#[test]
fn config_id_and_field_rules() {
    let (_temp, paths) = temp_paths();

    let invalid_ids = ["", "Agent", "1Agent!", "-agent", &"a".repeat(33)];
    for id in invalid_ids {
        let config = AgentConfig {
            custom_templates: vec![custom_template(id, "cd {workdir} && x {prompt}")],
            ..AgentConfig::default()
        };
        assert!(config::save(&paths, &config).is_err(), "id {id:?}");
    }

    let builtin = AgentConfig {
        custom_templates: vec![custom_template("claude", "cd {workdir} && x {prompt}")],
        ..AgentConfig::default()
    };
    assert!(config::save(&paths, &builtin).is_err());

    let duplicate = AgentConfig {
        custom_templates: vec![
            custom_template("agent", "cd {workdir} && x {prompt}"),
            custom_template("agent", "cd {workdir} && y {prompt}"),
        ],
        ..AgentConfig::default()
    };
    assert!(config::save(&paths, &duplicate).is_err());

    let long_name = AgentConfig {
        custom_templates: vec![AgentTemplate {
            id: "agent".to_string(),
            name: "n".repeat(41),
            command: "cd {workdir} && x {prompt}".to_string(),
        }],
        ..AgentConfig::default()
    };
    assert!(config::save(&paths, &long_name).is_err());

    let long_command = AgentConfig {
        custom_templates: vec![custom_template(
            "agent",
            &format!("cd {{workdir}} && x {{prompt}} {}", "a".repeat(2000)),
        )],
        ..AgentConfig::default()
    };
    assert!(config::save(&paths, &long_command).is_err());

    // Gueltige Vorlagen gehen durch, auch mit unbekannter aktiver ID.
    let valid = AgentConfig {
        custom_templates: vec![custom_template("agent", "cd {workdir} && x {prompt}")],
        active_template: "verschwunden".to_string(),
        ..AgentConfig::default()
    };
    assert!(config::save(&paths, &valid).is_ok());
}

#[test]
fn config_rejects_unknown_shell_and_summaries() {
    let (_temp, paths) = temp_paths();
    let config = AgentConfig {
        shell: "cmd".to_string(),
        ..AgentConfig::default()
    };
    assert_eq!(
        config::save(&paths, &config).unwrap_err(),
        "Unbekannte Shell"
    );

    let config = AgentConfig {
        summaries: "alle".to_string(),
        ..AgentConfig::default()
    };
    assert!(config::save(&paths, &config).is_err());
}

#[test]
fn h10_missing_include_transcript_field_defaults_to_true() {
    let (_temp, paths) = temp_paths();
    let path = config::config_path(&paths);
    std::fs::write(
        &path,
        r#"{"shell":"posix","summaries":"latest","includeChats":false,"prompt":"","activeTemplate":"claude","customTemplates":[]}"#,
    )
    .unwrap();

    let config = config::load(&paths);
    assert!(config.include_transcript, "fehlendes Feld bedeutet true");
    config::save(&paths, &config).unwrap();
    let stored = std::fs::read_to_string(&path).unwrap();
    assert!(stored.contains("\"includeTranscript\": true"), "{stored}");
}

#[test]
fn config_prompt_length_is_bounded() {
    let (_temp, paths) = temp_paths();
    let config = AgentConfig {
        prompt: "a".repeat(MAX_PROMPT_CHARS + 1),
        ..AgentConfig::default()
    };
    assert!(config::save(&paths, &config).is_err());

    let config = AgentConfig {
        prompt: "a".repeat(MAX_PROMPT_CHARS),
        ..AgentConfig::default()
    };
    assert!(config::save(&paths, &config).is_ok());
}

#[test]
fn config_view_exposes_builtins_and_default_base() {
    let (_temp, paths) = temp_paths();
    let view = super::config_view(&paths);
    assert_eq!(view.config, AgentConfig::default());
    assert_eq!(view.builtin_templates.len(), 5);
    assert_eq!(view.effective_shell, Shell::platform_default().name());
    assert!(view.default_workdir_base.ends_with("yt-agent"));

    let config = AgentConfig {
        shell: "fish".to_string(),
        ..AgentConfig::default()
    };
    config::save(&paths, &config).unwrap();
    let view = super::config_view(&paths);
    assert_eq!(view.effective_shell, "fish");
    assert_eq!(
        view.builtin_templates[2].command,
        "cd {workdir} && agy -i {prompt}"
    );
}

// ----------------------------------------------------------------- Vorschau --

#[test]
fn preview_uses_fixed_example_values() {
    let (_temp, paths) = temp_paths();
    let command = super::preview(&paths, "cd {workdir} && claude {prompt}", "posix").unwrap();
    assert!(command.starts_with("cd '/home/user/yt-agent/beispiel-video-dQw4w9WgXcQ' && claude '"));
    assert!(command.contains("/home/user/yt-agent/beispiel-video-dQw4w9WgXcQ/context.md"));
    assert!(!command.contains("youtube.com/watch"));
    assert!(!command.contains('\n'));
}

#[test]
fn preview_reports_validation_errors() {
    let (_temp, paths) = temp_paths();
    assert_eq!(
        super::preview(&paths, "echo hallo", "posix").unwrap_err(),
        "Die Vorlage nutzt keinen Kontext-Platzhalter"
    );
    assert_eq!(
        super::preview(&paths, "cd \"{workdir}\"", "posix").unwrap_err(),
        "Platzhalter nicht in Anführungszeichen setzen – die App maskiert die Werte selbst"
    );
    assert_eq!(
        super::preview(&paths, "cd {workdir}", "cmd").unwrap_err(),
        "Unbekannte Shell"
    );
}
