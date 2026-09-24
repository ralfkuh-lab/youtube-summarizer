//! Integrationsfaelle um `agent_prepare`: A4, A7-A9b, S4 und H1/H2.

use super::super::config::{self, AgentConfig};
use super::super::context;
use super::super::quote::Shell;
use super::super::resolve::Values;
use super::{integration_video, render, sample_values, sample_video, store_config, temp_paths};

#[test]
fn a4_invalid_video_id_writes_nothing() {
    let (temp, paths) = temp_paths();
    let video =
        crate::storage::insert_video(&paths, integration_video("../../etc", "Titel"), false)
            .unwrap();
    let base = temp.path().join("agent");
    store_config(&paths, &base, "cd {workdir} && claude {prompt}");

    let error = super::super::prepare(&paths, video.id, None, None).unwrap_err();
    assert_eq!(error, "Ungültige YouTube-ID im Datensatz");
    assert!(!base.exists(), "es darf nichts angelegt werden");
}

#[cfg(unix)]
#[test]
fn a7_context_file_replaces_a_symlink_and_leaves_siblings() {
    let (temp, paths) = temp_paths();
    let video =
        crate::storage::insert_video(&paths, integration_video("vid00000001", "Titel"), false)
            .unwrap();
    let base = temp.path().join("agent");
    store_config(&paths, &base, "cd {workdir} && claude {prompt}");

    let workdir = base.join("titel-vid00000001");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(workdir.join("notizen.md"), "bleibt").unwrap();
    let secret = workdir.join("geheim.txt");
    std::fs::write(&secret, "SECRET").unwrap();
    std::os::unix::fs::symlink(&secret, workdir.join("context.md")).unwrap();

    let handoff = super::super::prepare(&paths, video.id, None, None).unwrap();
    assert_eq!(
        std::fs::read_to_string(&secret).unwrap(),
        "SECRET",
        "das Symlink-Ziel bleibt unberuehrt"
    );
    assert_eq!(
        std::fs::read_to_string(workdir.join("notizen.md")).unwrap(),
        "bleibt"
    );
    let metadata = std::fs::symlink_metadata(&handoff.context_file).unwrap();
    assert!(
        metadata.file_type().is_file(),
        "context.md ist eine regulaere Datei"
    );
}

#[test]
fn a8_title_never_reaches_the_command() {
    let (temp, paths) = temp_paths();
    let video = crate::storage::insert_video(
        &paths,
        integration_video("vid00000002", "\"; touch /tmp/PWNED"),
        false,
    )
    .unwrap();
    let base = temp.path().join("agent");
    store_config(&paths, &base, "cd {workdir} && claude {prompt}");

    let handoff = super::super::prepare(&paths, video.id, None, None).unwrap();
    assert!(!handoff.command.contains("PWNED"), "{}", handoff.command);
    assert!(
        !handoff.command.contains("/tmp/PWNED"),
        "{}",
        handoff.command
    );
    assert!(
        !handoff.command.contains("\"; touch"),
        "{}",
        handoff.command
    );
    assert!(handoff.command.contains("claude '"));
}

#[test]
fn a9a_workdir_base_with_spaces_and_apostrophe_string_oracle() {
    let (temp, paths) = temp_paths();
    let video = crate::storage::insert_video(
        &paths,
        integration_video("abc_-123", "Jev explained in 7min.."),
        false,
    )
    .unwrap();
    let base = temp.path().join("agent it's");
    store_config(&paths, &base, "cd {workdir} && claude {prompt}");

    let handoff = super::super::prepare(&paths, video.id, None, None).unwrap();
    let workdir = format!("{}/jev-explained-in-7min-abc_-123", base.to_string_lossy());
    let context_file = format!("{workdir}/context.md");
    let prompt = super::super::resolve::normalize_prompt(
        super::super::config::DEFAULT_PROMPT,
        &context_file,
    );
    let expected = format!(
        "cd '{}' && claude '{}'",
        workdir.replace('\'', r"'\''"),
        prompt.replace('\'', r"'\''")
    );
    assert_eq!(handoff.command, expected);
    assert_eq!(handoff.workdir, workdir);
    assert_eq!(handoff.context_file, context_file);
}

#[cfg(unix)]
#[test]
fn a9b_quoted_workdir_works_in_a_shell() {
    let temp = tempfile::TempDir::new().unwrap();
    let directory = temp.path().join("agent it's");
    std::fs::create_dir_all(&directory).unwrap();
    let quoted = super::super::quote::shell_quote(
        &directory.to_string_lossy(),
        super::super::quote::Shell::Posix,
    )
    .unwrap();
    let command = format!("cd {quoted} && pwd");
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .output()
        .unwrap();
    assert!(output.status.success(), "{command}");
    let printed = String::from_utf8_lossy(&output.stdout);
    assert_eq!(printed.trim_end(), directory.to_string_lossy());
}

#[test]
fn s4_control_characters_in_workdir_base_write_nothing() {
    // Zeilenumbrueche, Tabulator und die Unicode-Zeilentrenner U+2028/U+2029
    // (H1) duerfen kein Kommando mit Zeilenumbruch erzeugen.
    for forbidden in ["\n", "\r", "\t", "\u{7f}", "\u{2028}", "\u{2029}"] {
        let (temp, paths) = temp_paths();
        let video =
            crate::storage::insert_video(&paths, integration_video("vid00000003", "Titel"), false)
                .unwrap();
        let base = temp.path().join(format!("agent{forbidden}neu"));
        store_config(&paths, &base, "cd {workdir} && claude {prompt}");

        let error = super::super::prepare(&paths, video.id, None, None).unwrap_err();
        assert_eq!(
            error, "Ungültige Zeichen im Wert workdir",
            "Zeichen {forbidden:?}"
        );
        assert!(!base.exists(), "Zeichen {forbidden:?}");
        let entries: Vec<String> = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            entries.len(),
            2,
            "nur Datenbank und Konfiguration bei {forbidden:?}: {entries:?}"
        );
        assert!(entries.iter().any(|name| name == "videos.db"));
        assert!(entries.iter().any(|name| name == "agent.json"));
    }
}

#[test]
fn h1_unicode_line_separators_are_rejected_in_values() {
    for value in ["a\u{2028}b", "a\u{2029}b"] {
        let values = Values {
            workdir: value.to_string(),
            ..sample_values()
        };
        assert_eq!(
            values.validate().unwrap_err(),
            "Ungültige Zeichen im Wert workdir"
        );
        assert!(super::super::quote::shell_quote(value, Shell::Posix).is_err());
    }
}

#[test]
fn h2_relative_workdir_base_is_resolved_against_home() {
    let (temp, paths) = temp_paths();
    let video =
        crate::storage::insert_video(&paths, integration_video("vid00000008", "Titel"), false)
            .unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let config = AgentConfig {
        workdir_base: "yt-agent".to_string(),
        ..AgentConfig::default()
    };
    config::save(&paths, &config).unwrap();

    let handoff = super::super::prepare_with_home(&paths, video.id, None, None, &home).unwrap();
    let expected = home.join("yt-agent").join("titel-vid00000008");
    assert_eq!(handoff.workdir, expected.to_string_lossy());
    assert!(std::path::Path::new(&handoff.workdir).is_absolute());
    assert!(std::path::Path::new(&handoff.context_file).is_absolute());
    assert_eq!(
        handoff.context_file,
        expected.join("context.md").to_string_lossy()
    );
    assert!(expected.join("context.md").is_file());
    assert!(handoff.command.contains(&handoff.workdir));
}

#[test]
fn missing_video_and_template_errors() {
    let (temp, paths) = temp_paths();
    let error = super::super::prepare(&paths, 42, None, None).unwrap_err();
    assert_eq!(error, "Video nicht gefunden");

    let video =
        crate::storage::insert_video(&paths, integration_video("vid00000006", "Titel"), false)
            .unwrap();
    let config = AgentConfig {
        workdir_base: temp.path().to_string_lossy().into_owned(),
        active_template: "weg".to_string(),
        ..AgentConfig::default()
    };
    config::save(&paths, &config).unwrap();
    assert_eq!(
        super::super::prepare(&paths, video.id, None, None).unwrap_err(),
        "Vorlage nicht gefunden"
    );
    assert_eq!(
        super::super::prepare(&paths, video.id, Some("gibt-es-nicht".to_string()), None)
            .unwrap_err(),
        "Vorlage nicht gefunden"
    );
    // Die eingebaute Vorlage ist weiterhin waehlbar.
    assert!(super::super::prepare(&paths, video.id, Some("codex".to_string()), None).is_ok());
}

#[test]
fn write_atomic_targets_a_fresh_path() {
    let (_temp, paths) = temp_paths();
    let mut video = sample_video();
    video.transcript = None;
    let directory = paths
        .config_path
        .parent()
        .unwrap()
        .join("agent")
        .join("slug");
    let path = directory.join("context.md");
    context::write_atomic(&path, &render(&video)).unwrap();
    let first = std::fs::read_to_string(&path).unwrap();
    context::write_atomic(&path, "zweite Fassung").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "zweite Fassung");
    assert!(first.starts_with("# YouTube-Kontext"));
}

#[test]
fn shell_selection_matches_the_platform() {
    assert_eq!(Shell::parse("auto").unwrap(), Shell::platform_default());
    assert!(Shell::parse("nushell").is_err());
}
