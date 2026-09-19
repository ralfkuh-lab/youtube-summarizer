//! Pfad-/Slug-Bildung und Einmal-Aufloesung der Kommando-Vorlage.

use std::path::{Path, PathBuf};

use crate::storage::AppResult;

use super::config::PLACEHOLDERS;
use super::quote::{has_forbidden_control, shell_quote, Shell};

/// Unter Windows reservierte Geraetenamen.
const WINDOWS_RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Alle Werte, die in einer Vorlage eingesetzt werden koennen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Values {
    pub workdir: String,
    pub context_file: String,
    pub prompt: String,
    pub video_id: String,
    pub video_url: String,
    pub db_path: String,
}

impl Values {
    pub fn get(&self, name: &str) -> Option<&str> {
        match name {
            "workdir" => Some(&self.workdir),
            "context_file" => Some(&self.context_file),
            "prompt" => Some(&self.prompt),
            "video_id" => Some(&self.video_id),
            "video_url" => Some(&self.video_url),
            "db_path" => Some(&self.db_path),
            _ => None,
        }
    }

    /// Jeder Wert muss einzeilig sein (`\0`-`\x1f`, `\x7f`, U+2028, U+2029).
    pub fn validate(&self) -> AppResult<()> {
        for name in PLACEHOLDERS {
            let value = self.get(name).unwrap_or_default();
            if has_forbidden_control(value) {
                return Err(format!("Ungültige Zeichen im Wert {name}"));
            }
        }
        Ok(())
    }
}

/// Loest die Vorlage auf: erst Werte pruefen, dann die **Originalvorlage**
/// genau einmal von links nach rechts durchlaufen.
pub fn resolve_command(template: &str, values: &Values, shell: Shell) -> AppResult<String> {
    values.validate()?;
    substitute(template, values, shell)
}

/// Ersetzt bekannte `{name}` durch den maskierten Wert. Eingesetzte Werte
/// werden nicht erneut durchsucht; unbekannte `{…}` bleiben stehen.
pub fn substitute(template: &str, values: &Values, shell: Shell) -> AppResult<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(offset) = rest.find('{') {
        out.push_str(&rest[..offset]);
        let tail = &rest[offset..];
        let matched = PLACEHOLDERS
            .iter()
            .find(|name| tail.starts_with(&format!("{{{name}}}")));
        match matched {
            Some(name) => {
                let value = values.get(name).unwrap_or_default();
                out.push_str(&shell_quote(value, shell)?);
                rest = &tail[name.len() + 2..];
            }
            None => {
                out.push('{');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// `^[A-Za-z0-9_-]{1,32}$`.
pub fn valid_video_id(video_id: &str) -> bool {
    let count = video_id.chars().count();
    (1..=32).contains(&count)
        && video_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

/// Basisverzeichnis der Kontexte: leer -> `<Home>/yt-agent`, fuehrendes `~`
/// bzw. `~/` -> Home. Nach der `~`-Aufloesung wird ein nicht absoluter Pfad
/// gegen das Home-Verzeichnis aufgeloest, damit App und Terminal dasselbe
/// Verzeichnis meinen.
pub fn resolve_workdir_base(base: &str, home: &Path) -> PathBuf {
    let resolved = if base.is_empty() {
        home.join("yt-agent")
    } else if base == "~" {
        home.to_path_buf()
    } else if let Some(rest) = base.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(base)
    };
    if resolved.is_absolute() {
        resolved
    } else {
        home.join(resolved)
    }
}

/// `<titel-slug>-<video_id>` (bzw. nur die Video-ID, wenn der Titel nichts
/// hergibt). Ein reservierter Geraetename wird in jedem Fall mit `v-`
/// entschaerft - auch wenn erst die Video-ID den Namen ergibt.
pub fn slug(title: &str, video_id: &str) -> String {
    let mut title_slug = title_slug(title);
    if is_windows_reserved(&title_slug) {
        title_slug = format!("v-{title_slug}");
    }
    let slug = if title_slug.is_empty() {
        video_id.to_string()
    } else {
        format!("{title_slug}-{video_id}")
    };
    if is_windows_reserved(&slug) {
        format!("v-{slug}")
    } else {
        slug
    }
}

fn is_windows_reserved(name: &str) -> bool {
    WINDOWS_RESERVED.contains(&name)
}

/// Slug-Schritte 1-4: NFC-nahe Komposition, Kleinschreibung, Transliteration,
/// Nicht-Alphanumerisches zu `-`, Laeufe zusammenfassen, 60 Zeichen, trimmen.
pub fn title_slug(title: &str) -> String {
    let lowered = compose_nfc(title).to_lowercase();
    let mut mapped = String::with_capacity(lowered.len());
    for ch in lowered.chars() {
        match ch {
            'ä' => mapped.push_str("ae"),
            'ö' => mapped.push_str("oe"),
            'ü' => mapped.push_str("ue"),
            'ß' => mapped.push_str("ss"),
            ch if ch.is_ascii_lowercase() || ch.is_ascii_digit() => mapped.push(ch),
            _ => mapped.push('-'),
        }
    }

    let mut collapsed = String::with_capacity(mapped.len());
    for ch in mapped.chars() {
        if ch == '-' && collapsed.ends_with('-') {
            continue;
        }
        collapsed.push(ch);
    }

    // Erst kuerzen, danach fuehrende/abschliessende Bindestriche entfernen.
    let truncated: String = collapsed.chars().take(60).collect();
    truncated.trim_matches('-').to_string()
}

/// Bringt zerlegte Umlaute (Base + U+0308) in die komponierte Form. Andere
/// kombinierende Zeichen ueber ASCII-Buchstaben werden wie ihr komponiertes
/// Ergebnis behandelt (nicht-ASCII -> `-`).
fn compose_nfc(text: &str) -> String {
    let mut out: Vec<char> = Vec::with_capacity(text.chars().count());
    for ch in text.chars() {
        if ch == '\u{0308}' {
            if let Some(last) = out.last().copied() {
                let umlaut = match last.to_ascii_lowercase() {
                    'a' => Some('ä'),
                    'o' => Some('ö'),
                    'u' => Some('ü'),
                    _ => None,
                };
                if let Some(umlaut) = umlaut {
                    out.pop();
                    out.push(if last.is_ascii_uppercase() {
                        umlaut.to_ascii_uppercase()
                    } else {
                        umlaut
                    });
                    continue;
                }
            }
            out.push('-');
            continue;
        }
        if is_combining_mark(ch) && out.last().is_some_and(|last| last.is_ascii_alphabetic()) {
            // NFC wuerde hier ein einzelnes Nicht-ASCII-Zeichen bilden, das im
            // Slug zu `-` wird.
            out.pop();
            out.push('-');
            continue;
        }
        out.push(ch);
    }
    out.into_iter().collect()
}

fn is_combining_mark(ch: char) -> bool {
    ('\u{0300}'..='\u{036f}').contains(&ch)
}

/// Prompt fuer die Aufloesung: Zeilenumbrueche **und Tabulatoren** zu einem
/// Leerzeichen, `{context_file}` roh eingesetzt, alles andere bleibt woertlich.
pub fn normalize_prompt(prompt: &str, context_file: &str) -> String {
    let mut single_line = String::with_capacity(prompt.len());
    for ch in prompt.chars() {
        match ch {
            '\r' | '\n' | '\t' | '\u{2028}' | '\u{2029}' => single_line.push(' '),
            ch => single_line.push(ch),
        }
    }
    single_line.replace("{context_file}", context_file)
}
