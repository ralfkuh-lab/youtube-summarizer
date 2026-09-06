use reqwest::Client;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

use crate::ai::auth::AuthStore;
use crate::ai::catalog::{self as ai_catalog, CatalogResult};
use crate::ai::client::{self as ai_client, ChatMessage};
use crate::ai::config::{AiConfigError, AiConfigService};
use crate::ai::types::{AiConfig, AuthStatus, Catalog, CustomProviderDefinition};
use crate::models::{Collection, NewVideo, Summary, Video};
use crate::storage::{self, AppPaths, AppResult};
use crate::summarize::{self, SummaryTarget};
use crate::summary_presets::{self, SummaryPreset};
use crate::youtube;

// ============================================================================
// New AI commands (ported from folio per spec-ai-port.md)
// ============================================================================

#[tauri::command]
pub async fn ai_catalog_get(paths: State<'_, AppPaths>) -> Result<CatalogResult, String> {
    Ok(ai_catalog::load(&paths))
}

#[tauri::command]
pub async fn ai_catalog_refresh(
    paths: State<'_, AppPaths>,
    http: State<'_, reqwest::Client>,
) -> Result<CatalogResult, String> {
    let result = ai_catalog::refresh(&http, &paths)
        .await
        .map_err(|e| e.to_string())?;
    eprintln!(
        "AI catalog refreshed: providers={}, updated_at={}",
        result.catalog.len(),
        result.updated_at
    );
    Ok(result)
}

#[tauri::command]
pub async fn ai_config_get(
    _paths: State<'_, AppPaths>,
    cfg: State<'_, std::sync::Mutex<AiConfigService>>,
) -> Result<AiConfig, String> {
    let guard = cfg
        .lock()
        .map_err(|_| "AI config lock poisoned".to_string())?;
    Ok(guard.data())
}

#[tauri::command]
pub async fn ai_provider_enable(
    provider_id: String,
    enabled: bool,
    _paths: State<'_, AppPaths>,
    cfg: State<'_, std::sync::Mutex<AiConfigService>>,
) -> Result<AiConfig, String> {
    let result = mutate_ai_config_state(&cfg, |service| {
        service.provider_enable(provider_id.clone(), enabled)
    })?;
    Ok(result)
}

#[tauri::command]
pub async fn ai_model_toggle(
    provider_id: String,
    model_id: String,
    on: bool,
    _paths: State<'_, AppPaths>,
    cfg: State<'_, std::sync::Mutex<AiConfigService>>,
) -> Result<AiConfig, String> {
    let result = mutate_ai_config_state(&cfg, |service| {
        service.model_toggle(provider_id.clone(), model_id.clone(), on)
    })?;
    Ok(result)
}

#[tauri::command]
pub async fn ai_custom_upsert(
    definition: CustomProviderDefinition,
    _paths: State<'_, AppPaths>,
    cfg: State<'_, std::sync::Mutex<AiConfigService>>,
) -> Result<AiConfig, String> {
    let result = mutate_ai_config_state(&cfg, |service| service.custom_upsert(definition))?;
    Ok(result)
}

#[tauri::command]
pub async fn ai_custom_delete(
    id: String,
    _paths: State<'_, AppPaths>,
    cfg: State<'_, std::sync::Mutex<AiConfigService>>,
) -> Result<AiConfig, String> {
    let result = mutate_ai_config_state(&cfg, |service| service.custom_delete(&id))?;
    Ok(result)
}

#[tauri::command]
pub async fn ai_custom_models_fetch(
    provider_id: String,
    _paths: State<'_, AppPaths>,
    cfg: State<'_, std::sync::Mutex<AiConfigService>>,
    auth: State<'_, std::sync::Mutex<AuthStore>>,
    http: State<'_, reqwest::Client>,
) -> Result<AiConfig, String> {
    let (base_url, key) = {
        let config = ai_config_data_from_state(&cfg)?;
        let provider = config
            .provider
            .get(&provider_id)
            .ok_or_else(|| format!("Custom-Provider '{provider_id}' wurde nicht gefunden"))?;
        if !provider.custom {
            return Err(format!("Provider '{provider_id}' ist kein Custom-Provider"));
        }
        let base_url = provider
            .options
            .as_ref()
            .map(|o| o.base_url.clone())
            .filter(|u| !u.trim().is_empty())
            .ok_or_else(|| format!("Custom-Provider '{provider_id}' hat keine Basis-URL"))?;
        let key = lock_ai_auth_from_state(&auth)?.get_key(&provider_id);
        (base_url, key)
    };

    let url = custom_models_url(&base_url)?;
    let mut request = http.get(url).timeout(std::time::Duration::from_secs(15));
    if let Some(k) = key.as_deref() {
        request = request.bearer_auth(k);
    }
    let resp = request
        .send()
        .await
        .map_err(|e| format!("Modelle von '{provider_id}' nicht abrufbar: {e}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Antwort lesen fehlgeschlagen: {e}"))?;
    if !status.is_success() {
        return Err(http_error(&provider_id, status, &body, key.as_deref()));
    }
    let model_ids = parse_custom_models(&body)
        .map_err(|e| format!("Ungültige Modellliste von '{provider_id}': {e}"))?;

    let result =
        mutate_ai_config_state(&cfg, |s| s.custom_models_replace(&provider_id, model_ids))?;
    Ok(result)
}

#[tauri::command]
pub async fn ai_default_model_set(
    provider_id: Option<String>,
    model_id: Option<String>,
    _paths: State<'_, AppPaths>,
    cfg: State<'_, std::sync::Mutex<AiConfigService>>,
) -> Result<AiConfig, String> {
    let result = mutate_ai_config_state(&cfg, |service| {
        service.default_model_set(provider_id, model_id)
    })?;
    Ok(result)
}

#[tauri::command]
pub async fn ai_auth_set(
    provider_id: String,
    key: String,
    _paths: State<'_, AppPaths>,
    auth: State<'_, std::sync::Mutex<AuthStore>>,
) -> Result<AuthStatus, String> {
    let mut guard = lock_ai_auth_from_state(&auth)?;
    guard
        .set(provider_id.clone(), key)
        .map_err(|e| e.to_string())?;
    let status = guard.status();
    drop(guard);
    // IMPORTANT: never log key
    eprintln!("AI auth set for provider (no key in log)");
    Ok(status)
}

#[tauri::command]
pub async fn ai_auth_remove(
    provider_id: String,
    _paths: State<'_, AppPaths>,
    auth: State<'_, std::sync::Mutex<AuthStore>>,
) -> Result<AuthStatus, String> {
    let mut guard = lock_ai_auth_from_state(&auth)?;
    guard.remove(&provider_id).map_err(|e| e.to_string())?;
    let status = guard.status();
    drop(guard);
    eprintln!("AI auth removed for provider (no key)");
    Ok(status)
}

#[tauri::command]
pub async fn ai_auth_status(
    _paths: State<'_, AppPaths>,
    auth: State<'_, std::sync::Mutex<AuthStore>>,
) -> Result<AuthStatus, String> {
    Ok(lock_ai_auth_from_state(&auth)?.status())
}

#[tauri::command]
pub async fn ai_model_chat_test(
    paths: State<'_, AppPaths>,
    cfg: State<'_, std::sync::Mutex<AiConfigService>>,
    auth: State<'_, std::sync::Mutex<AuthStore>>,
    http: State<'_, reqwest::Client>,
    provider_id: String,
    model_id: String,
    messages: Vec<ChatMessage>, // {role, content} from frontend
) -> AppResult<String> {
    let config = ai_config_data_from_state(&cfg)?;
    let provider_cfg = config
        .provider
        .get(&provider_id)
        .ok_or_else(|| "KI-Provider nicht gefunden".to_string())?;
    if !provider_cfg.enabled {
        return Err("KI-Provider ist nicht aktiviert".to_string());
    }
    if !provider_cfg.whitelist.iter().any(|m| m == &model_id) {
        return Err("Modell nicht in Whitelist".to_string());
    }
    let base_url = provider_base_url(
        &config,
        &ai_catalog::load(paths.inner()).catalog,
        &provider_id,
    )?;
    let key = lock_ai_auth_from_state(&auth)?.get_key(&provider_id);

    ai_client::chat_stream(
        &http,
        &base_url,
        key.as_deref(),
        &model_id,
        &messages,
        |_| {},
    )
    .await
    .map_err(|e| e.to_string())
}

// Internal helpers for new commands (no keys in results)

// use managed state via mutate_ai_config_state / from_state (F11)

fn ai_config_data_from_state(cfg: &std::sync::Mutex<AiConfigService>) -> Result<AiConfig, String> {
    let guard = cfg
        .lock()
        .map_err(|_| "AI config lock poisoned".to_string())?;
    Ok(guard.data())
}

fn mutate_ai_config_state(
    cfg: &std::sync::Mutex<AiConfigService>,
    mutation: impl FnOnce(&mut AiConfigService) -> Result<(), AiConfigError>,
) -> Result<AiConfig, String> {
    let mut service = cfg
        .lock()
        .map_err(|_| "AI config lock poisoned".to_string())?;
    mutation(&mut service).map_err(|e| e.to_string())?;
    Ok(service.data())
}

fn lock_ai_auth_from_state(
    auth: &std::sync::Mutex<AuthStore>,
) -> Result<std::sync::MutexGuard<'_, AuthStore>, String> {
    auth.lock().map_err(|_| "AI auth lock poisoned".to_string())
}

pub(crate) fn provider_base_url(
    config: &AiConfig,
    catalog: &Catalog,
    provider_id: &str,
) -> Result<String, String> {
    let configured = config.provider.get(provider_id);
    let endpoint = if configured.is_some_and(|p| p.custom) {
        configured
            .and_then(|p| p.options.as_ref())
            .map(|o| o.base_url.trim())
            .filter(|u| !u.is_empty())
    } else {
        catalog
            .get(provider_id)
            .and_then(|p| p.api.as_deref())
            .map(str::trim)
            .filter(|u| !u.is_empty())
    };
    endpoint
        .map(str::to_string)
        .ok_or_else(|| format!("Provider '{provider_id}' hat keinen bekannten Endpoint."))
}

fn custom_models_url(base_url: &str) -> Result<reqwest::Url, String> {
    let mut url =
        reqwest::Url::parse(base_url.trim()).map_err(|e| format!("Ungültige baseURL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("baseURL muss HTTP(S) sein".to_string());
    }
    // Tolerate baseURLs that include the full chat path (e.g. migrated endpoint overrides)
    let p = url
        .path()
        .trim_end_matches('/')
        .trim_end_matches("/chat/completions")
        .to_string();
    if !p.ends_with("/models") {
        url.set_path(&format!("{p}/models"));
    } else {
        url.set_path(&p);
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn parse_custom_models(body: &str) -> Result<Vec<String>, serde_json::Error> {
    #[derive(serde::Deserialize)]
    struct Resp {
        data: Vec<IdItem>,
    }
    #[derive(serde::Deserialize)]
    struct IdItem {
        id: String,
    }
    let r: Resp = serde_json::from_str(body)?;
    Ok(r.data
        .into_iter()
        .map(|i| i.id.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

fn http_error(
    provider_id: &str,
    status: reqwest::StatusCode,
    body: &str,
    api_key: Option<&str>,
) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let redacted = match api_key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => compact.replace(k, "[REDACTED]"),
        None => compact,
    };
    let msg = redacted.chars().take(300).collect::<String>();
    if msg.is_empty() {
        format!("Provider '{provider_id}' antwortete mit HTTP-Status {status}")
    } else {
        format!("Provider '{provider_id}' antwortete mit HTTP-Status {status}: {msg}")
    }
}

#[tauri::command]
pub fn get_videos(paths: State<'_, AppPaths>) -> AppResult<Vec<Video>> {
    storage::get_videos(&paths)
}

#[tauri::command]
pub fn get_collections(paths: State<'_, AppPaths>) -> AppResult<Vec<Collection>> {
    storage::get_collections(&paths)
}

#[tauri::command]
pub fn create_collection(paths: State<'_, AppPaths>, name: String) -> AppResult<Collection> {
    storage::create_collection(&paths, &name)
}

#[tauri::command]
pub fn update_collection(
    paths: State<'_, AppPaths>,
    id: i64,
    name: String,
) -> AppResult<Collection> {
    storage::update_collection(&paths, id, &name)
}

#[tauri::command]
pub fn delete_collection(paths: State<'_, AppPaths>, id: i64) -> AppResult<()> {
    storage::delete_collection(&paths, id)
}

#[tauri::command]
pub fn set_video_collections(
    paths: State<'_, AppPaths>,
    video_id: i64,
    collection_ids: Vec<i64>,
) -> AppResult<Video> {
    storage::set_video_collections(&paths, video_id, collection_ids)
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
    // Fetch oembed (title) and the watch HTML in parallel; the watch HTML is
    // loaded at most once and provides both publish date and chapters. A failed
    // HTML fetch leaves both None without failing add_video (oembed stays hard).
    let (info, html) = tokio::join!(
        youtube::fetch_video_info(&client, &video_id),
        youtube::fetch_watch_html(&client, &video_id),
    );
    let mut info = info?;
    let html = html.ok();
    info.published_at = html.as_deref().and_then(youtube::publish_date_from_html);

    let thumbnail_data = youtube::download_thumbnail(&client, &video_id).await;

    let (transcript, transcript_error) = match youtube::fetch_transcript(&client, &video_id).await {
        Ok(transcript) => (Some(transcript), None),
        Err(error) => (None, Some(error)),
    };
    let chapters = html.as_deref().and_then(youtube::chapters_from_html);
    let description = html.as_deref().and_then(youtube::description_from_html);

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
            description,
            transcript_error,
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
    let transcript = match youtube::fetch_transcript(&client, &video.video_id).await {
        Ok(transcript) => transcript,
        Err(error) => {
            if video.transcript.is_none() {
                if let Err(storage_error) = storage::set_transcript_error(paths, id, &error) {
                    eprintln!("Transkript-Fehler für Video {id} konnte nicht gespeichert werden: {storage_error}");
                }
            }
            return Err(error);
        }
    };
    // Only overwrite chapters and description when the watch HTML actually
    // loaded. If the fetch fails, keep the video's existing values instead of
    // clearing them.
    let (chapters, description) = match youtube::fetch_watch_html(&client, &video.video_id).await {
        Ok(html) => (
            youtube::chapters_from_html(&html),
            youtube::description_from_html(&html),
        ),
        Err(_) => (
            video
                .chapters
                .as_ref()
                .and_then(|chapters| serde_json::to_string(chapters).ok()),
            video.description.clone(),
        ),
    };
    storage::update_transcript(
        paths,
        id,
        &transcript,
        chapters.as_deref(),
        description.as_deref(),
    )
}

#[tauri::command]
pub async fn summarize_video(
    app: AppHandle,
    paths: State<'_, AppPaths>,
    cfg: State<'_, std::sync::Mutex<AiConfigService>>,
    auth: State<'_, std::sync::Mutex<AuthStore>>,
    id: i64,
    system_prompt: String,
    provider_id: Option<String>,
    model_id: Option<String>,
    timestamps: Option<bool>,
    options: Option<String>,
    http: State<'_, reqwest::Client>,
) -> AppResult<Video> {
    let target = {
        let ai = ai_config_data_from_state(&cfg)?;
        let catalog = ai_catalog::load(paths.inner()).catalog;
        let (selected, base_url) =
            summarize::resolve_summary_target(&ai, &catalog, provider_id, model_id)?;
        let key = lock_ai_auth_from_state(&auth)?.get_key(&selected.provider);
        let provider_label = summarize::provider_label(&ai, &catalog, &selected.provider);
        SummaryTarget {
            provider_label,
            model: selected.model,
            base_url,
            api_key: key,
        }
    };
    let mut last_emit = None;
    let mut emit_error_logged = false;
    summarize::summarize_video_impl(
        &paths,
        &http,
        id,
        system_prompt,
        target,
        timestamps,
        options,
        |accumulated| {
            let now = Instant::now();
            if last_emit
                .is_some_and(|last: Instant| now.duration_since(last) < Duration::from_millis(150))
            {
                return;
            }
            last_emit = Some(now);
            if let Err(error) = app.emit(
                "ai:summarize_stream",
                serde_json::json!({
                    "videoId": id,
                    "text": accumulated,
                    "chars": accumulated.chars().count(),
                }),
            ) {
                if !emit_error_logged {
                    eprintln!("ai:summarize_stream emit failed: {error}");
                    emit_error_logged = true;
                }
            }
        },
    )
    .await
}

#[tauri::command]
pub fn delete_video(paths: State<'_, AppPaths>, id: i64) -> AppResult<()> {
    storage::delete_video(&paths, id)
}

#[tauri::command]
pub fn summary_presets_list(paths: State<'_, AppPaths>) -> AppResult<Vec<SummaryPreset>> {
    summary_presets::list(&paths)
}

#[tauri::command]
pub fn summary_preset_save(
    paths: State<'_, AppPaths>,
    preset: SummaryPreset,
) -> AppResult<SummaryPreset> {
    summary_presets::save(&paths, preset)
}

#[tauri::command]
pub fn summary_preset_delete(paths: State<'_, AppPaths>, id: String) -> AppResult<()> {
    summary_presets::delete(&paths, &id)
}

#[tauri::command]
pub fn get_summaries(paths: State<'_, AppPaths>, video_id: i64) -> AppResult<Vec<Summary>> {
    storage::get_summaries(&paths, video_id)
}

#[tauri::command]
pub fn delete_summary(paths: State<'_, AppPaths>, id: i64) -> AppResult<()> {
    storage::delete_summary(&paths, id)
}

fn http_client() -> AppResult<Client> {
    Client::builder()
        .user_agent("Mozilla/5.0 YouTubeSummarizer/0.1")
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|err| format!("HTTP-Client konnte nicht erstellt werden: {err}"))
}
