//! Video an einen lokalen Agenten uebergeben.
//!
//! Stufe 1: Die App schreibt eine Kontextdatei mit den Daten genau eines
//! Videos und liefert ein fertiges, shell-sicher maskiertes Kommando, das der
//! Benutzer selbst in die Zwischenablage kopiert. Die App startet **keinen**
//! Prozess.

mod config;
mod context;
mod quote;
mod resolve;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::models::{Chat, ChatMessageRecord};
use crate::storage::{self, AppPaths, AppResult};

use config::{AgentConfig, AgentTemplate};
use quote::Shell;
use resolve::Values;

/// Alle Chats eines Videos mit ihren Nachrichten.
type ChatHistory = (Vec<Chat>, BTreeMap<i64, Vec<ChatMessageRecord>>);

/// Antwort von `agent_config_get`: gespeicherte Konfiguration plus die fuer die
/// Oberflaeche abgeleiteten Werte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfigView {
    pub config: AgentConfig,
    /// Eingebaute Vorlagen der wirksamen Shell.
    pub builtin_templates: Vec<AgentTemplate>,
    pub default_workdir_base: String,
    pub effective_shell: String,
}

/// Ergebnis von `agent_prepare`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHandoff {
    pub command: String,
    pub workdir: String,
    pub context_file: String,
}

// --------------------------------------------------------------- Commands --

#[tauri::command]
pub fn agent_config_get(paths: State<'_, AppPaths>) -> AppResult<AgentConfigView> {
    Ok(config_view(&paths))
}

#[tauri::command]
pub fn agent_config_set(
    paths: State<'_, AppPaths>,
    config: AgentConfig,
) -> AppResult<AgentConfigView> {
    config::save(&paths, &config)?;
    Ok(config_view(&paths))
}

#[tauri::command]
pub fn agent_prepare(
    paths: State<'_, AppPaths>,
    video_id: i64,
    template_id: Option<String>,
) -> AppResult<AgentHandoff> {
    prepare(&paths, video_id, template_id)
}

#[tauri::command]
pub fn agent_preview(
    paths: State<'_, AppPaths>,
    command: String,
    shell: String,
) -> AppResult<String> {
    preview(&paths, &command, &shell)
}

// ---------------------------------------------------------------- Kernlogik --

fn config_view(paths: &AppPaths) -> AgentConfigView {
    let config = config::load(paths);
    // Eine defekte Shell-Angabe darf die Oberflaeche nicht blockieren.
    let shell = Shell::parse(&config.shell).unwrap_or_else(|_| Shell::platform_default());
    AgentConfigView {
        config,
        builtin_templates: config::builtin_templates(shell),
        default_workdir_base: to_string(&home_dir().join("yt-agent")),
        effective_shell: shell.name().to_string(),
    }
}

/// Schreibt die Kontextdatei und loest die Vorlage auf.
pub fn prepare(
    paths: &AppPaths,
    video_id: i64,
    template_id: Option<String>,
) -> AppResult<AgentHandoff> {
    let config = config::load(paths);
    let shell = Shell::parse(&config.shell)?;
    let video =
        storage::get_video(paths, video_id)?.ok_or_else(|| "Video nicht gefunden".to_string())?;

    let requested = template_id.unwrap_or_else(|| config.active_template.clone());
    let template = config::find_template(&config, shell, &requested)
        .ok_or_else(|| "Vorlage nicht gefunden".to_string())?;

    // Erst pruefen, dann schreiben: eine unbrauchbare Video-ID legt nichts an.
    if !resolve::valid_video_id(&video.video_id) {
        return Err("Ungültige YouTube-ID im Datensatz".to_string());
    }

    let home = home_dir();
    let workdir = resolve::resolve_workdir_base(&config.workdir_base, &home)
        .join(resolve::slug(&video.title, &video.video_id));
    let context_path = workdir.join("context.md");
    let context_file = to_string(&context_path);

    let values = Values {
        workdir: to_string(&workdir),
        context_file: context_file.clone(),
        prompt: resolve::normalize_prompt(&config.effective_prompt(), &context_file),
        video_id: video.video_id.clone(),
        video_url: crate::youtube::video_url(&video.video_id),
        db_path: to_string(&paths.db_path),
    };
    // Vor dem Schreiben pruefen, damit ungueltige Werte nichts anlegen.
    values.validate()?;

    let summaries = if config.summaries == "all" {
        storage::get_summaries(paths, video_id)?
    } else {
        Vec::new()
    };
    let (chats, messages) = if config.include_chats {
        load_chat_history(paths, video_id)?
    } else {
        (Vec::new(), BTreeMap::new())
    };

    let exported_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let contents = context::render(&context::ContextSources {
        video: &video,
        summaries: &summaries,
        chats: &chats,
        messages: &messages,
        summary_mode: &config.summaries,
        include_chats: config.include_chats,
        exported_at: &exported_at,
    });
    context::write_atomic(&context_path, &contents)?;

    let command = resolve::substitute(&template.command, &values, shell)?;
    Ok(AgentHandoff {
        workdir: values.workdir,
        context_file: values.context_file,
        command,
    })
}

/// Vorschau einer Vorlage mit festen Beispielwerten; schreibt nichts.
pub fn preview(paths: &AppPaths, command: &str, shell: &str) -> AppResult<String> {
    config::validate_command(command)?;
    let shell = Shell::parse(shell)?;
    let config = config::load(paths);

    let workdir = "/home/user/yt-agent/beispiel-video-dQw4w9WgXcQ".to_string();
    let context_file = format!("{workdir}/context.md");
    let values = Values {
        workdir,
        context_file: context_file.clone(),
        prompt: resolve::normalize_prompt(&config.effective_prompt(), &context_file),
        video_id: "dQw4w9WgXcQ".to_string(),
        video_url: crate::youtube::video_url("dQw4w9WgXcQ"),
        db_path: "/home/user/.local/share/app/videos.db".to_string(),
    };
    resolve::resolve_command(command, &values, shell)
}

fn load_chat_history(paths: &AppPaths, video_id: i64) -> AppResult<ChatHistory> {
    let chats = storage::list_chats(paths, video_id)?;
    let mut messages = BTreeMap::new();
    for chat in &chats {
        messages.insert(chat.id, storage::get_chat_messages(paths, chat.id)?);
    }
    Ok((chats, messages))
}

fn to_string(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Home-Verzeichnis ohne zusaetzliche Abhaengigkeit.
pub fn home_dir() -> PathBuf {
    for key in ["HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(key) {
            let path = PathBuf::from(value);
            if !path.as_os_str().is_empty() {
                return path;
            }
        }
    }
    if let (Some(drive), Some(rest)) = (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH"))
    {
        let mut path = PathBuf::from(drive);
        path.push(rest);
        return path;
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
