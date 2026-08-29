//! Custom- und Built-in-Vorlagen für den Zusammenfassen-Dialog.
//! Store: `summary-presets.json` im App-Data-Dir (nur Custom-Presets).

use std::fs;
use std::io;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::storage::{self, AppPaths, AppResult};

static PRESET_STORE_LOCK: Mutex<()> = Mutex::new(());

fn lock_preset_store() -> std::sync::MutexGuard<'static, ()> {
    PRESET_STORE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub const SLUG_MAX_LEN: usize = 32;
pub const NAME_MAX_CHARS: usize = 80;
pub const PROMPT_MAX_CHARS: usize = 8_000;

pub const STANDARD_PROMPT: &str =
    "You are an expert assistant that turns YouTube video transcripts into \
clear, well-structured Markdown summaries. Start with a 1-2 sentence \
overview. Organize the key points under short headings and use bullet \
points. End with the main conclusions or takeaways. Ground every statement \
in the transcript; do not invent facts.";

const TUTORIAL_PROMPT: &str = "You are an expert assistant that turns YouTube tutorial and how-to \
transcripts into clear, well-structured Markdown summaries. Start with a \
1-2 sentence overview of what the video teaches. Then give a numbered, \
step-by-step list of the procedure shown. Highlight tools, commands, \
settings and materials that are used. End with the main takeaways. Ground \
every statement in the transcript; do not invent facts.";

const TALK_PROMPT: &str = "You are an expert assistant that turns YouTube talks, lectures and \
interviews into clear, well-structured Markdown summaries. Start with a \
1-2 sentence overview. Group the key theses by speaker where speakers can \
be identified. Quote important statements verbatim. Trace the through-line \
of the argument. End with the main conclusions. Ground every statement in \
the transcript; do not invent facts.";

const NEWS_PROMPT: &str =
    "You are an expert assistant that turns YouTube news and current-affairs \
transcripts into clear, well-structured Markdown summaries. Cover: what \
happened; who says what; what is new; and which questions remain open. \
Start with a 1-2 sentence overview and end with the main takeaways. Ground \
every statement in the transcript; do not invent facts.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryPreset {
    pub id: String,
    pub name: String,
    pub prompt: String,
    #[serde(default)]
    pub builtin: bool,
}

pub fn validate_slug(value: &str) -> AppResult<()> {
    let bytes = value.as_bytes();
    let valid = !bytes.is_empty()
        && bytes.len() <= SLUG_MAX_LEN
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(format!(
            "Ungültige Vorlagen-ID '{value}': Kleinbuchstaben, Ziffern und '-' \
             (max. {SLUG_MAX_LEN} Zeichen, beginnend mit Buchstabe oder Ziffer)"
        ))
    }
}

pub fn validate_preset(preset: &SummaryPreset) -> AppResult<()> {
    validate_slug(&preset.id)?;
    if preset.name.trim().is_empty() || preset.name.chars().count() > NAME_MAX_CHARS {
        return Err(format!(
            "Vorlagenname muss zwischen 1 und {NAME_MAX_CHARS} Zeichen lang sein"
        ));
    }
    if preset.prompt.trim().is_empty() || preset.prompt.chars().count() > PROMPT_MAX_CHARS {
        return Err(format!(
            "Vorlagen-Prompt muss zwischen 1 und {PROMPT_MAX_CHARS} Zeichen lang sein"
        ));
    }
    Ok(())
}

pub fn builtin_presets() -> Vec<SummaryPreset> {
    vec![
        builtin("standard", "Standard", STANDARD_PROMPT),
        builtin("tutorial", "Tutorial/How-To", TUTORIAL_PROMPT),
        builtin("talk", "Vortrag/Interview", TALK_PROMPT),
        builtin("news", "News/Einordnung", NEWS_PROMPT),
    ]
}

fn builtin(id: &str, name: &str, prompt: &str) -> SummaryPreset {
    SummaryPreset {
        id: id.into(),
        name: name.into(),
        prompt: prompt.into(),
        builtin: true,
    }
}

fn is_builtin_id(id: &str) -> bool {
    builtin_presets().iter().any(|preset| preset.id == id)
}

pub fn list(paths: &AppPaths) -> AppResult<Vec<SummaryPreset>> {
    list_from(&storage::summary_presets_path(paths))
}

pub fn list_from(path: &Path) -> AppResult<Vec<SummaryPreset>> {
    let mut presets = builtin_presets();
    let builtin_ids: Vec<String> = presets.iter().map(|preset| preset.id.clone()).collect();
    for mut preset in load_custom(path)? {
        preset.builtin = false;
        if builtin_ids.contains(&preset.id) {
            continue;
        }
        if validate_preset(&preset).is_ok() {
            presets.push(preset);
        }
    }
    Ok(presets)
}

pub fn save(paths: &AppPaths, preset: SummaryPreset) -> AppResult<SummaryPreset> {
    save_to(&storage::summary_presets_path(paths), preset)
}

pub fn save_to(path: &Path, mut preset: SummaryPreset) -> AppResult<SummaryPreset> {
    preset.builtin = false;
    preset.name = preset.name.trim().to_string();
    validate_preset(&preset)?;
    if is_builtin_id(&preset.id) {
        return Err(format!(
            "Vorlagen-ID '{}' ist fest vorgegeben und kann nicht überschrieben werden",
            preset.id
        ));
    }

    let _guard = lock_preset_store();
    let mut custom = load_custom(path)?;
    if let Some(existing) = custom.iter_mut().find(|item| item.id == preset.id) {
        *existing = preset.clone();
    } else {
        custom.push(preset.clone());
    }
    write_custom(path, &custom)?;
    Ok(preset)
}

pub fn delete(paths: &AppPaths, id: &str) -> AppResult<()> {
    delete_from(&storage::summary_presets_path(paths), id)
}

pub fn delete_from(path: &Path, id: &str) -> AppResult<()> {
    validate_slug(id)?;
    if is_builtin_id(id) {
        return Err("Feste Vorlagen können nicht gelöscht werden".to_string());
    }
    let _guard = lock_preset_store();
    let mut custom = load_custom(path)?;
    let before = custom.len();
    custom.retain(|preset| preset.id != id);
    if custom.len() == before {
        return Err(format!("Vorlage '{id}' wurde nicht gefunden"));
    }
    write_custom(path, &custom)
}

fn load_custom(path: &Path) -> AppResult<Vec<SummaryPreset>> {
    match fs::read_to_string(path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(format!("Vorlagen konnten nicht gelesen werden: {err}")),
        Ok(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Ok(Vec::new())
            } else {
                serde_json::from_str(trimmed).map_err(|err| {
                    format!("Vorlagen-Datei ist ungültig und wurde nicht überschrieben: {err}")
                })
            }
        }
    }
}

fn write_custom(path: &Path, presets: &[SummaryPreset]) -> AppResult<()> {
    crate::ai::config::save_json_atomic(path, &presets)
        .map_err(|err| format!("Vorlagen konnten nicht gespeichert werden: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_store() -> (TempDir, std::path::PathBuf) {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("summary-presets.json");
        (temp, path)
    }

    fn custom(id: &str, name: &str, prompt: &str) -> SummaryPreset {
        SummaryPreset {
            id: id.into(),
            name: name.into(),
            prompt: prompt.into(),
            builtin: false,
        }
    }

    #[test]
    fn slug_accepts_lowercase_digits_and_hyphens() {
        assert!(validate_slug("a").is_ok());
        assert!(validate_slug("tutorial").is_ok());
        assert!(validate_slug("1-own").is_ok());
        assert!(validate_slug(&"a".repeat(SLUG_MAX_LEN)).is_ok());
    }

    #[test]
    fn slug_rejects_empty_uppercase_separators_and_overflow() {
        assert!(validate_slug("").is_err());
        assert!(validate_slug("Standard").is_err());
        assert!(validate_slug("-lead").is_err());
        assert!(validate_slug("has_underscore").is_err());
        assert!(validate_slug("has.dot").is_err());
        assert!(validate_slug(&"a".repeat(SLUG_MAX_LEN + 1)).is_err());
    }

    #[test]
    fn preset_validation_enforces_name_and_prompt_bounds() {
        let mut preset = custom("own", "Name", "Prompt");
        assert!(validate_preset(&preset).is_ok());

        preset.name = " ".into();
        assert!(validate_preset(&preset)
            .unwrap_err()
            .contains("Vorlagenname"));

        preset.name = "x".repeat(NAME_MAX_CHARS + 1);
        assert!(validate_preset(&preset)
            .unwrap_err()
            .contains("Vorlagenname"));

        preset.name = "Name".into();
        preset.prompt = "   ".into();
        assert!(validate_preset(&preset)
            .unwrap_err()
            .contains("Vorlagen-Prompt"));

        preset.prompt = "p".repeat(PROMPT_MAX_CHARS + 1);
        assert!(validate_preset(&preset)
            .unwrap_err()
            .contains("Vorlagen-Prompt"));
    }

    #[test]
    fn list_returns_builtins_when_store_is_missing() {
        let (_temp, path) = temp_store();
        let listed = list_from(&path).unwrap();
        assert_eq!(listed.len(), 4);
        assert!(listed.iter().all(|preset| preset.builtin));
        assert_eq!(listed[0].id, "standard");
        assert_eq!(listed[0].prompt, STANDARD_PROMPT);
    }

    #[test]
    fn save_and_delete_round_trip_keeps_builtins_first() {
        let (_temp, path) = temp_store();
        let saved = save_to(&path, custom("eigenes", " Eigenes ", "Tu was.")).unwrap();
        assert!(!saved.builtin);
        assert_eq!(saved.name, "Eigenes");

        let listed = list_from(&path).unwrap();
        assert_eq!(listed.len(), 5);
        assert_eq!(listed[0].id, "standard");
        assert_eq!(listed[4].id, "eigenes");
        assert!(!listed[4].builtin);

        delete_from(&path, "eigenes").unwrap();
        assert_eq!(list_from(&path).unwrap().len(), 4);
    }

    #[test]
    fn save_rejects_builtin_ids() {
        let (_temp, path) = temp_store();
        let error = save_to(&path, custom("standard", "Nope", "overwrite")).unwrap_err();
        assert!(error.contains("fest vorgegeben"), "unexpected: {error}");
        assert!(!path.exists());
    }

    #[test]
    fn delete_rejects_builtin_ids() {
        let (_temp, path) = temp_store();
        let error = delete_from(&path, "tutorial").unwrap_err();
        assert!(error.contains("nicht gelöscht"), "unexpected: {error}");
    }

    #[test]
    fn save_updates_existing_custom_preset() {
        let (_temp, path) = temp_store();
        save_to(&path, custom("eigenes", "Alt", "Prompt A")).unwrap();
        save_to(&path, custom("eigenes", "Neu", "Prompt B")).unwrap();
        let custom_only: Vec<_> = list_from(&path)
            .unwrap()
            .into_iter()
            .filter(|preset| !preset.builtin)
            .collect();
        assert_eq!(custom_only.len(), 1);
        assert_eq!(custom_only[0].name, "Neu");
        assert_eq!(custom_only[0].prompt, "Prompt B");
    }

    #[test]
    fn list_skips_invalid_and_builtin_colliding_custom_entries() {
        let (_temp, path) = temp_store();
        let raw = serde_json::json!([
            {"id": "standard", "name": "Shadow", "prompt": "nope"},
            {"id": "BAD", "name": "Nope", "prompt": "nope"},
            {"id": "ok-one", "name": "Ok", "prompt": "Do it."}
        ]);
        fs::write(&path, serde_json::to_string(&raw).unwrap()).unwrap();
        let listed = list_from(&path).unwrap();
        let ids: Vec<_> = listed.iter().map(|preset| preset.id.as_str()).collect();
        assert_eq!(ids, vec!["standard", "tutorial", "talk", "news", "ok-one"]);
        assert_eq!(listed[0].name, "Standard");
    }

    #[test]
    fn corrupt_store_is_not_silently_replaced() {
        let (_temp, path) = temp_store();
        fs::write(&path, "{broken").unwrap();
        let list_error = list_from(&path).unwrap_err();
        assert!(list_error.contains("ungültig"), "unexpected: {list_error}");
        let save_error = save_to(&path, custom("eigenes", "Name", "Prompt")).unwrap_err();
        assert!(save_error.contains("ungültig"), "unexpected: {save_error}");
        let delete_error = delete_from(&path, "eigenes").unwrap_err();
        assert!(
            delete_error.contains("ungültig"),
            "unexpected: {delete_error}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "{broken");
    }
}
