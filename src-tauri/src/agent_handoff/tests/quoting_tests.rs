//! Referenzfaelle zur Maskierung (Q1-Q9) und zur Aufloesung (S1-S6).

use super::super::config::{builtin_templates, validate_command};
use super::super::quote::{shell_quote, Shell};
use super::super::resolve::{normalize_prompt, resolve_command, Values};
use super::sample_values;

// ------------------------------------------------------------ Q1-Q9 Maskierung --

#[test]
fn q1_q8_quoting_table() {
    let cases: [(&str, &str, &str, &str); 6] = [
        ("abc", r"'abc'", r"'abc'", r"'abc'"),
        ("it's", r"'it'\''s'", r"'it\'s'", r"'it''s'"),
        (
            "a b$(id)`x`;touch /tmp/PWNED",
            "'a b$(id)`x`;touch /tmp/PWNED'",
            "'a b$(id)`x`;touch /tmp/PWNED'",
            "'a b$(id)`x`;touch /tmp/PWNED'",
        ),
        ("", r"''", r"''", r"''"),
        ("a\\b", r"'a\b'", r"'a\\b'", r"'a\b'"),
        (
            "/home/x/yt agent/über-uns",
            r"'/home/x/yt agent/über-uns'",
            r"'/home/x/yt agent/über-uns'",
            r"'/home/x/yt agent/über-uns'",
        ),
    ];
    for (input, posix, fish, powershell) in cases {
        assert_eq!(
            shell_quote(input, Shell::Posix).unwrap(),
            posix,
            "posix: {input:?}"
        );
        assert_eq!(
            shell_quote(input, Shell::Fish).unwrap(),
            fish,
            "fish: {input:?}"
        );
        assert_eq!(
            shell_quote(input, Shell::Powershell).unwrap(),
            powershell,
            "powershell: {input:?}"
        );
    }
    // Q8: Sonderzeichen ohne Bedeutung in Single-Quotes.
    assert_eq!(shell_quote("a!b%c", Shell::Posix).unwrap(), r"'a!b%c'");
}

/// Q2 und Q3: Apostroph und Shell-Metazeichen in Single-Quotes.
#[test]
fn q2_q3_quote_metacharacters() {
    let payload = "a b$(id)`x`;touch /tmp/PWNED";
    let cases: [(&str, &str, &str, &str); 2] = [
        ("it's", r"'it'\''s'", r"'it\'s'", r"'it''s'"),
        (
            payload,
            r"'a b$(id)`x`;touch /tmp/PWNED'",
            r"'a b$(id)`x`;touch /tmp/PWNED'",
            r"'a b$(id)`x`;touch /tmp/PWNED'",
        ),
    ];
    for (input, posix, fish, powershell) in cases {
        assert_eq!(
            shell_quote(input, Shell::Posix).unwrap(),
            posix,
            "posix {input:?}"
        );
        assert_eq!(
            shell_quote(input, Shell::Fish).unwrap(),
            fish,
            "fish {input:?}"
        );
        assert_eq!(
            shell_quote(input, Shell::Powershell).unwrap(),
            powershell,
            "powershell {input:?}"
        );
    }
}

/// Q6 und Q9 fuer fish: Backslash-Verdopplung.
#[test]
fn q6_q9_fish_backslash_and_quote() {
    assert_eq!(
        shell_quote("a\\b", Shell::Fish).unwrap(),
        r"'a\\b'",
        "Q6 fish"
    );
    assert_eq!(
        shell_quote("\\'", Shell::Fish).unwrap(),
        r"'\\\''",
        "Q9 fish"
    );
}

#[test]
fn q9_backslash_before_apostrophe() {
    assert_eq!(
        shell_quote("\\'", Shell::Posix).unwrap(),
        r"'\'\'''",
        "posix"
    );
    assert_eq!(shell_quote("\\'", Shell::Fish).unwrap(), r"'\\\''", "fish");
    assert_eq!(
        shell_quote("\\'", Shell::Powershell).unwrap(),
        r"'\'''",
        "powershell"
    );
}

#[test]
fn q5_control_characters_are_rejected() {
    for value in [
        "a\nb",
        "a\rb",
        "a\0b",
        "a\u{1b}b",
        "a\tb",
        "\u{7f}",
        "a\u{2028}b",
        "a\u{2029}b",
    ] {
        for shell in [Shell::Posix, Shell::Fish, Shell::Powershell] {
            assert!(
                shell_quote(value, shell).is_err(),
                "{value:?} muss fuer {shell:?} abgelehnt werden"
            );
        }
    }
    // Leerzeichen und C1-Zeichen sind erlaubt.
    assert_eq!(shell_quote("a b", Shell::Posix).unwrap(), r"'a b'");
    assert!(shell_quote("a\u{9d}b", Shell::Posix).is_ok());
}

// ------------------------------------------------------------ S1-S3 Aufloesung --

#[test]
fn s1_placeholder_inside_value_stays_literal() {
    let values = Values {
        workdir: "/tmp/x/{prompt}".to_string(),
        context_file: "/tmp/x/{prompt}/context.md".to_string(),
        prompt: "x; touch /tmp/x/PWNED".to_string(),
        video_id: "vid".to_string(),
        video_url: "https://www.youtube.com/watch?v=vid".to_string(),
        db_path: "/tmp/x/videos.db".to_string(),
    };
    let command =
        resolve_command("cd {workdir} && echo AGENT {prompt}", &values, Shell::Posix).unwrap();
    assert_eq!(
        command,
        "cd '/tmp/x/{prompt}' && echo AGENT 'x; touch /tmp/x/PWNED'"
    );
}

#[test]
fn s2_context_file_with_placeholder_is_one_argument() {
    let context_file = "/tmp/x/{workdir}/context.md";
    let prompt = normalize_prompt("Lies {context_file} und {video_id}", context_file);
    let values = Values {
        workdir: "/tmp/x".to_string(),
        context_file: context_file.to_string(),
        prompt: prompt.clone(),
        video_id: "vid".to_string(),
        video_url: "https://www.youtube.com/watch?v=vid".to_string(),
        db_path: "/tmp/x/videos.db".to_string(),
    };
    let command = resolve_command("claude {prompt}", &values, Shell::Posix).unwrap();
    assert_eq!(command, format!("claude '{}'", prompt));
    assert_eq!(
        command,
        "claude 'Lies /tmp/x/{workdir}/context.md und {video_id}'"
    );
}

#[test]
fn s3_prompt_becomes_one_line_and_keeps_video_id() {
    let prompt = normalize_prompt("Zeile1\r\nZeile2 {video_id}\u{2028}Zeile3", "/tmp/c.md");
    assert_eq!(prompt, "Zeile1  Zeile2 {video_id} Zeile3");
    let values = Values {
        prompt,
        ..sample_values()
    };
    let command = resolve_command("claude {prompt}", &values, Shell::Posix).unwrap();
    assert!(!command.contains('\n') && !command.contains('\r'));
    assert_eq!(command, "claude 'Zeile1  Zeile2 {video_id} Zeile3'");
}

/// Tabulatoren im Prompt werden zu einem Leerzeichen, statt die Aufloesung
/// mit `Ungueltige Zeichen im Wert prompt` abzubrechen.
#[test]
fn h7_tab_in_prompt_becomes_a_space() {
    let prompt = normalize_prompt("Spalte1\tSpalte2", "/tmp/c.md");
    assert_eq!(prompt, "Spalte1 Spalte2");
    let values = Values {
        prompt,
        ..sample_values()
    };
    let command = resolve_command("claude {prompt}", &values, Shell::Posix).unwrap();
    assert_eq!(command, "claude 'Spalte1 Spalte2'");
    // Auch mit Tabulator bleibt der Wert ein einziges Argument.
    assert_eq!(command.matches("'Spalte1 Spalte2'").count(), 1);
}

#[test]
fn unknown_placeholders_stay_untouched() {
    let command =
        resolve_command("echo {unbekannt} {workdir}", &sample_values(), Shell::Posix).unwrap();
    assert_eq!(command, "echo {unbekannt} '/tmp/wd'");
}

// --------------------------------------------------------------- S5 Validierung --

#[test]
fn s5_command_validation_errors() {
    let cases = [
        (
            r#"cd "{workdir}" && x"#,
            "Platzhalter nicht in Anführungszeichen setzen – die App maskiert die Werte selbst",
        ),
        (
            r"x '{prompt}'",
            "Platzhalter nicht in Anführungszeichen setzen – die App maskiert die Werte selbst",
        ),
        ("cd {workdir}\n&& x", "Die Vorlage muss einzeilig sein"),
        ("echo hallo", "Die Vorlage nutzt keinen Kontext-Platzhalter"),
    ];
    for (command, expected) in cases {
        assert_eq!(
            validate_command(command).unwrap_err(),
            expected,
            "{command:?}"
        );
    }
    assert!(validate_command("cd {workdir} && claude {prompt}").is_ok());
    assert!(validate_command("backtick `{workdir}`").is_err());
}

// ----------------------------------------------------------------- S6 Vorlagen --

#[test]
fn s6_builtin_templates_are_single_line_and_start_with_cd() {
    let values = sample_values();
    for shell in [Shell::Posix, Shell::Fish, Shell::Powershell] {
        for template in builtin_templates(shell) {
            let command = resolve_command(&template.command, &values, shell).unwrap();
            assert!(!command.contains('\n'), "{template:?} {shell:?}");
            let workdir = shell_quote(&values.workdir, shell).unwrap();
            match shell {
                Shell::Posix | Shell::Fish => {
                    assert!(
                        command.starts_with(&format!("cd {workdir} && ")),
                        "{command}"
                    );
                }
                Shell::Powershell => {
                    assert!(
                        command.starts_with(&format!(
                            "if (Test-Path -LiteralPath {workdir}) {{ Set-Location -LiteralPath {workdir}; "
                        )),
                        "{command}"
                    );
                }
            }
            let quoted_prompt = shell_quote(&values.prompt, shell).unwrap();
            assert_eq!(command.matches(&quoted_prompt).count(), 1, "{command}");
        }
    }
}

#[test]
fn builtin_templates_use_the_documented_invocations() {
    let posix: Vec<String> = builtin_templates(Shell::Posix)
        .into_iter()
        .map(|template| template.command)
        .collect();
    assert_eq!(
        posix,
        vec![
            "cd {workdir} && claude {prompt}",
            "cd {workdir} && codex {prompt}",
            "cd {workdir} && agy -i {prompt}",
            "cd {workdir} && grok {prompt}",
            "cd {workdir} && pi {prompt}",
        ]
    );
    assert_eq!(
        builtin_templates(Shell::Fish)[0].command,
        "cd {workdir} && claude {prompt}"
    );
    assert_eq!(
        builtin_templates(Shell::Powershell)[0].command,
        "if (Test-Path -LiteralPath {workdir}) { Set-Location -LiteralPath {workdir}; claude {prompt} }"
    );
}
