use reqwest::Client;
use tauri::State;

use crate::ai;
use crate::models::{AiConfig, NewVideo, Video};
use crate::storage::{self, AppPaths, AppResult};
use crate::youtube;

#[tauri::command]
pub fn get_config(paths: State<'_, AppPaths>) -> AppResult<AiConfig> {
    storage::get_ai_config(&paths)
}

#[tauri::command]
pub fn save_config(
    paths: State<'_, AppPaths>,
    provider: String,
    api_key: String,
    model: String,
    endpoint_override: Option<String>,
) -> AppResult<AiConfig> {
    let endpoint_override = endpoint_override.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });

    storage::update_ai_config(
        &paths,
        AiConfig {
            provider,
            api_key,
            model,
            endpoint_override,
        },
    )
}

#[tauri::command]
pub fn get_videos(paths: State<'_, AppPaths>) -> AppResult<Vec<Video>> {
    storage::get_videos(&paths)
}

#[tauri::command]
pub fn get_video_detail(paths: State<'_, AppPaths>, id: i64) -> AppResult<Video> {
    storage::get_video(&paths, id)?.ok_or_else(|| "Video nicht gefunden".to_string())
}

#[tauri::command]
pub async fn add_video(paths: State<'_, AppPaths>, url: String) -> AppResult<Video> {
    add_video_impl(&paths, url).await
}

pub async fn add_video_impl(paths: &AppPaths, url: String) -> AppResult<Video> {
    let video_id = youtube::extract_video_id(&url)
        .ok_or_else(|| "Ungültige YouTube-URL oder Video-ID".to_string())?;
    if storage::video_exists(paths, &video_id)? {
        return Err("Video bereits in der Liste vorhanden".to_string());
    }

    let client = http_client()?;
    let info = youtube::fetch_video_info(&client, &video_id).await?;
    let thumbnail_data = youtube::download_thumbnail(&client, &video_id).await;

    let transcript = youtube::fetch_transcript(&client, &video_id).await.ok();
    let chapters = youtube::fetch_chapters(&client, &video_id).await;

    storage::insert_video(
        paths,
        NewVideo {
            video_id: video_id.clone(),
            url: youtube::video_url(&video_id),
            title: info.title,
            thumbnail_url: info.thumbnail_url,
            thumbnail_data,
            transcript,
            chapters,
        },
    )
}

#[tauri::command]
pub async fn refresh_transcript(paths: State<'_, AppPaths>, id: i64) -> AppResult<Video> {
    refresh_transcript_impl(&paths, id).await
}

pub async fn refresh_transcript_impl(paths: &AppPaths, id: i64) -> AppResult<Video> {
    let video = storage::get_video(paths, id)?.ok_or_else(|| "Video nicht gefunden".to_string())?;
    let client = http_client()?;
    let transcript = youtube::fetch_transcript(&client, &video.video_id).await?;
    let chapters = youtube::fetch_chapters(&client, &video.video_id).await;
    storage::update_transcript(paths, id, &transcript, chapters.as_deref())
}

#[tauri::command]
pub async fn summarize_video(
    paths: State<'_, AppPaths>,
    id: i64,
    system_prompt: String,
) -> AppResult<Video> {
    summarize_video_impl(&paths, id, system_prompt).await
}

pub async fn summarize_video_impl(
    paths: &AppPaths,
    id: i64,
    system_prompt: String,
) -> AppResult<Video> {
    let video = storage::get_video(paths, id)?.ok_or_else(|| "Video nicht gefunden".to_string())?;
    let transcript = video
        .transcript
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Kein Transkript vorhanden - bitte Video neu hinzufügen".to_string())?;
    let transcript_text = youtube::transcript_to_text(transcript);
    let chapters_json = video
        .chapters
        .as_ref()
        .and_then(|chapters| serde_json::to_string(chapters).ok());
    let ai_config = storage::get_ai_config(paths)?;
    let client = http_client()?;
    let prompt = system_prompt.trim();
    let summary = ai::summarize(
        &client,
        &ai_config,
        &transcript_text,
        chapters_json.as_deref(),
        if prompt.is_empty() {
            None
        } else {
            Some(prompt)
        },
    )
    .await?;

    storage::update_summary(paths, id, &summary)
}

#[tauri::command]
pub fn delete_video(paths: State<'_, AppPaths>, id: i64) -> AppResult<()> {
    storage::delete_video(&paths, id)
}

fn http_client() -> AppResult<Client> {
    Client::builder()
        .user_agent("Mozilla/5.0 YouTubeSummarizer/0.1")
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|err| format!("HTTP-Client konnte nicht erstellt werden: {err}"))
}
