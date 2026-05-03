use reqwest::Client;
use tauri::State;

use crate::ai;
use crate::models::{AiChatMessage, AiConfig, AiProviderInfo, NewVideo, Video};
use crate::storage::{self, AppPaths, AppResult};
use crate::youtube;

#[tauri::command]
pub fn get_config(paths: State<'_, AppPaths>) -> AppResult<AiConfig> {
    storage::get_ai_config(&paths)
}

#[tauri::command]
pub fn get_ai_providers() -> Vec<AiProviderInfo> {
    ai::provider_catalog()
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
            providers: Vec::new(),
        },
    )
}

#[tauri::command]
pub fn save_provider_config(
    paths: State<'_, AppPaths>,
    provider_id: String,
    name: Option<String>,
    enabled: Option<bool>,
    api_key_required: Option<bool>,
    api_key: String,
    model: String,
    endpoint_override: Option<String>,
    activate: Option<bool>,
    account_tier: Option<String>,
) -> AppResult<AiConfig> {
    storage::update_provider_config(
        &paths,
        provider_id,
        name,
        enabled.unwrap_or(true),
        api_key_required,
        api_key,
        model,
        endpoint_override,
        activate.unwrap_or(false),
        account_tier,
    )
}

#[tauri::command]
pub fn add_custom_provider(
    paths: State<'_, AppPaths>,
    local_ollama: Option<bool>,
) -> AppResult<AiConfig> {
    storage::add_custom_provider(&paths, local_ollama.unwrap_or(false))
}

#[tauri::command]
pub fn delete_custom_provider(
    paths: State<'_, AppPaths>,
    provider_id: String,
) -> AppResult<AiConfig> {
    storage::delete_custom_provider(&paths, &provider_id)
}

#[tauri::command]
pub async fn refresh_provider_models(
    paths: State<'_, AppPaths>,
    provider_id: String,
    force_reprobe: Option<bool>,
) -> AppResult<AiConfig> {
    let config = storage::get_ai_config(&paths)?;
    let provider = storage::provider_config(&config, &provider_id)
        .ok_or_else(|| "KI-Anbieter nicht gefunden".to_string())?;
    let mut request_config = config.clone();
    request_config.provider = provider.id.clone();
    request_config.api_key = provider.api_key.clone();
    request_config.model = provider.model.clone();
    request_config.endpoint_override = provider.endpoint_override.clone();
    let existing_models = provider.models.clone();
    let account_tier = provider
        .account_tier
        .clone()
        .unwrap_or_else(|| "free".to_string());

    let client = http_client()?;
    match ai::fetch_models(
        &client,
        &request_config,
        &provider_id,
        &existing_models,
        &account_tier,
        force_reprobe.unwrap_or(false),
    )
    .await
    {
        Ok(models) => storage::update_provider_models(
            &paths,
            &provider_id,
            models,
            chrono::Utc::now().to_rfc3339(),
            None,
        ),
        Err(error) => {
            let _ = storage::set_provider_error(&paths, &provider_id, error.clone());
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn test_provider_model_chat(
    paths: State<'_, AppPaths>,
    provider_id: String,
    model_id: String,
    messages: Vec<AiChatMessage>,
) -> AppResult<String> {
    let config = storage::get_ai_config(&paths)?;
    let provider = storage::provider_config(&config, &provider_id)
        .ok_or_else(|| "KI-Anbieter nicht gefunden".to_string())?;
    let mut request_config = config.clone();
    request_config.provider = provider.id.clone();
    request_config.api_key = provider.api_key.clone();
    request_config.model = model_id;
    request_config.endpoint_override = provider.endpoint_override.clone();

    let client = http_client()?;
    ai::test_chat(&client, &request_config, &messages).await
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
            published_at: info.published_at,
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
        Some(video.title.as_str()),
        video.published_at.as_deref(),
    )
    .await?;

    let provider_label = storage::provider_config(&ai_config, &ai_config.provider)
        .and_then(|p| p.name.clone())
        .or_else(|| {
            ai::provider_catalog()
                .into_iter()
                .find(|info| info.id == ai_config.provider)
                .map(|info| info.name)
        })
        .unwrap_or_else(|| ai_config.provider.clone());
    storage::update_summary(
        paths,
        id,
        &summary,
        Some(&provider_label),
        Some(&ai_config.model),
    )
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
