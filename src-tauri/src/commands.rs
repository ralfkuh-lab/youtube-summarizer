use reqwest::Client;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

use crate::ai::auth::AuthStore;
use crate::ai::catalog::{self as ai_catalog, CatalogResult};
use crate::ai::client::{self as ai_client, ChatMessage};
use crate::ai::config::{AiConfigError, AiConfigService};
use crate::ai::types::{AiConfig, AiModelRef, AuthStatus, Catalog, CustomProviderDefinition};
use crate::models::{Collection, NewVideo, Video};
use crate::storage::{self, AppPaths, AppResult};
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

fn provider_base_url(
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

// Migration from old config.json (best effort, once)
// Order: keys first (with error handling), THEN atomic ai.json write (done marker).
// Customs: any id not in the 4 known hosted ones (incl. "ollama") treated as custom.
pub(crate) fn ensure_migrated(paths: &AppPaths) {
    let ai_path = storage::ai_json_path(paths);
    if ai_path.exists() {
        return; // already migrated or fresh
    }
    // try read old config.json for ai block (raw, no type)
    let old_cfg_text = match std::fs::read_to_string(&paths.config_path) {
        Ok(t) => t,
        Err(_) => {
            // even no config, write marker
            let _ = crate::ai::config::save_json_atomic(
                &ai_path,
                &crate::ai::types::AiConfig::default(),
            );
            return;
        }
    };
    let old: serde_json::Value = match serde_json::from_str(&old_cfg_text) {
        Ok(v) => v,
        Err(_) => {
            let _ = crate::ai::config::save_json_atomic(
                &ai_path,
                &crate::ai::types::AiConfig::default(),
            );
            return;
        }
    };
    let old_ai = match old.get("ai") {
        Some(a) if a.is_object() => a,
        _ => {
            let _ = crate::ai::config::save_json_atomic(
                &ai_path,
                &crate::ai::types::AiConfig::default(),
            );
            return;
        }
    };

    let known_hosted: [&str; 4] = ["ollama_cloud", "openrouter", "opencode_zen", "opencode_go"];
    let is_hosted = |id: &str| known_hosted.contains(&id);

    // Build minimal old shape
    let old_provider = old_ai
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let old_model = old_ai
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let old_key = old_ai
        .get("api_key")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let old_endpoint = old_ai
        .get("endpoint_override")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut providers_map: BTreeMap<String, crate::ai::types::AiProviderConfig> = BTreeMap::new();
    let mut default_model: Option<AiModelRef> = None;
    let mut migrated_ids: Vec<String> = vec![];

    let catalog = ai_catalog::load(paths).catalog;

    // active provider -> provider entry (custom or mapped), even without key
    if !old_provider.is_empty() {
        let is_custom = !is_hosted(&old_provider);
        let mapped = if is_custom {
            old_provider.clone()
        } else {
            map_old_provider_id(&old_provider).unwrap_or_else(|| old_provider.clone())
        };
        if catalog.contains_key(&mapped) || is_custom {
            let mut pcfg = crate::ai::types::AiProviderConfig {
                enabled: true,
                custom: is_custom,
                ..Default::default()
            };
            if is_custom {
                if let Some(ep) = old_endpoint.clone().filter(|e| !e.trim().is_empty()) {
                    pcfg.options = Some(crate::ai::types::AiProviderOptions {
                        base_url: strip_chat_suffix(&ep),
                    });
                }
            }
            if !old_model.is_empty() {
                pcfg.whitelist = vec![old_model.clone()];
                default_model = Some(AiModelRef {
                    provider: mapped.clone(),
                    model: old_model.clone(),
                });
            }
            providers_map.insert(mapped.clone(), pcfg);
            migrated_ids.push(mapped.clone());
        }
    }

    // other providers from list: add entry regardless of key (b), custom if not hosted
    if let Some(provs) = old_ai.get("providers").and_then(|v| v.as_array()) {
        for prov in provs {
            let pid = prov
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if pid.is_empty() {
                continue;
            }
            let is_custom = !is_hosted(&pid);
            let mapped = if is_custom {
                pid.clone()
            } else {
                map_old_provider_id(&pid).unwrap_or_else(|| pid.clone())
            };
            if catalog.contains_key(&mapped) || is_custom {
                let entry = providers_map.entry(mapped.clone()).or_insert_with(|| {
                    crate::ai::types::AiProviderConfig {
                        enabled: true,
                        custom: is_custom,
                        ..Default::default()
                    }
                });
                entry.enabled = true;
                if is_custom {
                    if let Some(ep) = prov
                        .get("endpoint_override")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                    {
                        entry.options = Some(crate::ai::types::AiProviderOptions {
                            base_url: strip_chat_suffix(ep),
                        });
                    }
                }
                if !migrated_ids.contains(&mapped) {
                    migrated_ids.push(mapped);
                }
            }
            // keys handled separately below
        }
    }

    // FIRST: migrate keys (a) - proper error handling, never swallow silently for keys
    let mut auth_store = AuthStore::load(paths);
    if !old_key.trim().is_empty() {
        if let Some(m) = (if is_hosted(&old_provider) {
            map_old_provider_id(&old_provider)
        } else {
            None
        })
        .or_else(|| {
            if !is_hosted(&old_provider) {
                Some(old_provider.clone())
            } else {
                None
            }
        }) {
            if let Err(e) = auth_store.set(m.clone(), old_key.clone()) {
                eprintln!(
                    "AI migration: auth key set error for provider (id redacted): {}",
                    e
                );
            }
        }
    }
    if let Some(provs) = old_ai.get("providers").and_then(|v| v.as_array()) {
        for prov in provs {
            let pid = prov
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let pkey = prov
                .get("api_key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if pkey.trim().is_empty() {
                continue;
            }
            let target = if is_hosted(&pid) {
                map_old_provider_id(&pid).unwrap_or(pid.clone())
            } else {
                pid.clone()
            };
            if let Err(e) = auth_store.set(target, pkey) {
                eprintln!(
                    "AI migration: auth key set error for listed provider (id redacted): {}",
                    e
                );
            }
        }
    }

    // if nothing at all, still write marker (d)
    if providers_map.is_empty() && old_key.trim().is_empty() {
        let _ =
            crate::ai::config::save_json_atomic(&ai_path, &crate::ai::types::AiConfig::default());
        return;
    }

    let new_ai = crate::ai::types::AiConfig {
        provider: providers_map,
        default_model,
        translate: Default::default(),
    };

    // THEN atomic ai.json (a, d) -- this marks done
    if let Err(e) = crate::ai::config::save_json_atomic(&ai_path, &new_ai) {
        eprintln!("AI migration: failed to write ai.json marker: {}", e);
    }

    eprintln!("AI migration completed for providers: {:?}", migrated_ids);
}

fn map_old_provider_id(old: &str) -> Option<String> {
    match old {
        "opencode_zen" => Some("opencode".into()),
        "opencode_go" => Some("opencode-go".into()),
        "ollama_cloud" => Some("ollama-cloud".into()),
        "openrouter" => Some("openrouter".into()),
        _ => None,
    }
}

// Old endpoint overrides stored the full chat URL; the new baseURL schema expects the API root.
fn strip_chat_suffix(endpoint: &str) -> String {
    endpoint
        .trim()
        .trim_end_matches('/')
        .trim_end_matches("/chat/completions")
        .to_string()
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

    let transcript = youtube::fetch_transcript(&client, &video_id).await.ok();
    let chapters = html.as_deref().and_then(youtube::chapters_from_html);

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
    // Only overwrite chapters when the watch HTML actually loaded. If the fetch
    // fails, keep the video's existing chapters instead of clearing them.
    let chapters = match youtube::fetch_watch_html(&client, &video.video_id).await {
        Ok(html) => youtube::chapters_from_html(&html),
        Err(_) => video
            .chapters
            .as_ref()
            .and_then(|chapters| serde_json::to_string(chapters).ok()),
    };
    storage::update_transcript(paths, id, &transcript, chapters.as_deref())
}

#[tauri::command]
pub async fn summarize_video(
    app: AppHandle,
    paths: State<'_, AppPaths>,
    id: i64,
    system_prompt: String,
    provider_id: Option<String>,
    model_id: Option<String>,
    http: State<'_, reqwest::Client>,
) -> AppResult<Video> {
    let mut last_emit = None;
    let mut emit_error_logged = false;
    summarize_video_impl(
        &paths,
        &http,
        id,
        system_prompt,
        provider_id,
        model_id,
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

pub async fn summarize_video_impl(
    paths: &AppPaths,
    http: &reqwest::Client,
    id: i64,
    system_prompt: String,
    provider_id: Option<String>,
    model_id: Option<String>,
    on_delta: impl FnMut(&str),
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

    let ai = AiConfigService::load(paths).data();
    let selected = resolve_summary_model(&ai, provider_id, model_id)?;
    // resolve
    let catalog = ai_catalog::load(paths).catalog;
    let base_url = provider_base_url(&ai, &catalog, &selected.provider)?;
    let key = AuthStore::load(paths).get_key(&selected.provider);

    // build prompt (unchanged logic from old DEFAULT + user)
    let prompt = system_prompt.trim();
    let sys = if prompt.is_empty() {
        DEFAULT_SYSTEM_PROMPT.to_string()
    } else {
        prompt.to_string()
    };

    let mut user_content =
        String::from("Please summarize the following YouTube video transcript.\n");
    if !video.title.trim().is_empty() {
        user_content.push_str(&format!("\nVideo title: {}", video.title));
    }
    if let Some(published) = video
        .published_at
        .as_deref()
        .filter(|v| !v.trim().is_empty())
    {
        user_content.push_str(&format!("\nPublished on: {}", published));
    }
    user_content.push_str("\n\nTranscript:\n");
    user_content.push_str(&transcript_text);
    if let Some(ch) = chapters_json.as_deref().filter(|v| !v.trim().is_empty()) {
        user_content.push_str("\n\nAvailable chapter markers as JSON:\n");
        user_content.push_str(ch);
    }

    let messages = vec![ChatMessage::system(sys), ChatMessage::user(user_content)];

    let raw = ai_client::chat_stream(
        http,
        &base_url,
        key.as_deref(),
        &selected.model,
        &messages,
        on_delta,
    )
    .await
    .map_err(|e| format!("KI-Anfrage fehlgeschlagen: {}", e))?;

    let summary = strip_wrapping_code_fence(&raw);

    let provider_label = ai
        .provider
        .get(&selected.provider)
        .and_then(|p| p.name.clone())
        .or_else(|| catalog.get(&selected.provider).and_then(|p| p.name.clone()))
        .unwrap_or_else(|| selected.provider.clone());

    storage::update_summary(
        paths,
        id,
        &summary,
        Some(&provider_label),
        Some(&selected.model),
    )
}

/// Waehlt das Modell fuer einen Zusammenfassungslauf. Ohne explizite Auswahl
/// gilt weiterhin das Default-Modell aus den Einstellungen; eine explizite
/// Auswahl wird gegen die Provider-/Modell-Freischaltung geprueft, damit der
/// Aufruf nicht an einem laengst deaktivierten Modell haengen bleibt.
fn resolve_summary_model(
    ai: &AiConfig,
    provider_id: Option<String>,
    model_id: Option<String>,
) -> AppResult<AiModelRef> {
    let provider_id = provider_id.filter(|value| !value.trim().is_empty());
    let model_id = model_id.filter(|value| !value.trim().is_empty());
    let requested = match (provider_id, model_id) {
        (Some(provider), Some(model)) => Some(AiModelRef { provider, model }),
        (None, None) => None,
        _ => {
            return Err("Unvollständige Modellauswahl - Anbieter und Modell angeben".to_string());
        }
    };

    let Some(selected) = requested else {
        return ai.default_model.clone().ok_or_else(|| {
            "Kein defaultModel gesetzt - bitte in Einstellungen KI-Modell auswählen".to_string()
        });
    };

    let provider = ai
        .provider
        .get(&selected.provider)
        .ok_or_else(|| format!("KI-Provider '{}' nicht gefunden", selected.provider))?;
    if !provider.enabled {
        return Err(format!(
            "KI-Provider '{}' ist nicht aktiviert",
            selected.provider
        ));
    }
    if !provider.whitelist.iter().any(|id| id == &selected.model) {
        return Err(format!(
            "Modell '{}' ist für '{}' nicht aktiviert",
            selected.model, selected.provider
        ));
    }
    Ok(selected)
}

// Unchanged summary prompt + parse/strip (moved here to keep behavior + tests intact)
const DEFAULT_SYSTEM_PROMPT: &str = r#"You are a helpful assistant that summarizes YouTube video transcripts.
Provide a clear, structured summary in the same language as the transcript.
Include:
- A short overview (1-2 sentences)
- Key points as bullet points
- Main conclusions or takeaways

Format your response as Markdown."#;

fn strip_wrapping_code_fence(text: &str) -> String {
    let trimmed = text.trim();
    // keep the regex from old (regex crate is dep)
    let re = match regex::Regex::new(r"(?s)^```([^\n]*)\n(.*?)\n?```$") {
        Ok(r) => r,
        Err(_) => return trimmed.to_string(),
    };
    let Some(caps) = re.captures(trimmed) else {
        return trimmed.to_string();
    };
    let info = caps[1].trim().to_lowercase();
    if !info.is_empty() && info != "markdown" && info != "md" {
        return trimmed.to_string();
    }
    let inner = &caps[2];
    if inner
        .lines()
        .any(|line| line.trim_start().starts_with("```"))
    {
        return trimmed.to_string();
    }
    inner.to_string()
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

#[cfg(test)]
mod tests {
    use super::{resolve_summary_model, strip_wrapping_code_fence};
    use crate::ai::types::{AiConfig, AiModelRef, AiProviderConfig};

    fn config_with_enabled_model() -> AiConfig {
        let mut config = AiConfig::default();
        config.provider.insert(
            "openrouter".into(),
            AiProviderConfig {
                enabled: true,
                whitelist: vec!["fast".into(), "smart".into()],
                ..Default::default()
            },
        );
        config.default_model = Some(AiModelRef {
            provider: "openrouter".into(),
            model: "fast".into(),
        });
        config
    }

    #[test]
    fn falls_back_to_default_model_without_selection() {
        let selected = resolve_summary_model(&config_with_enabled_model(), None, None).unwrap();
        assert_eq!(selected.model, "fast");
    }

    #[test]
    fn uses_explicit_selection_over_default_model() {
        let selected = resolve_summary_model(
            &config_with_enabled_model(),
            Some("openrouter".into()),
            Some("smart".into()),
        )
        .unwrap();
        assert_eq!(selected.provider, "openrouter");
        assert_eq!(selected.model, "smart");
    }

    #[test]
    fn rejects_model_that_is_not_whitelisted() {
        let error = resolve_summary_model(
            &config_with_enabled_model(),
            Some("openrouter".into()),
            Some("unknown".into()),
        )
        .unwrap_err();
        assert!(error.contains("nicht aktiviert"), "unexpected: {error}");
    }

    #[test]
    fn rejects_selection_from_disabled_provider() {
        let mut config = config_with_enabled_model();
        config.provider.get_mut("openrouter").unwrap().enabled = false;
        let error = resolve_summary_model(&config, Some("openrouter".into()), Some("fast".into()))
            .unwrap_err();
        assert!(error.contains("nicht aktiviert"), "unexpected: {error}");
    }

    #[test]
    fn rejects_incomplete_selection() {
        let error = resolve_summary_model(
            &config_with_enabled_model(),
            Some("openrouter".into()),
            None,
        )
        .unwrap_err();
        assert!(error.contains("Unvollständige"), "unexpected: {error}");
    }

    #[test]
    fn treats_blank_selection_as_no_selection() {
        let selected = resolve_summary_model(
            &config_with_enabled_model(),
            Some("  ".into()),
            Some(String::new()),
        )
        .unwrap();
        assert_eq!(selected.model, "fast");
    }

    #[test]
    fn strips_markdown_wrapping_fence() {
        let input = "```markdown\n# Title\n\nBody **bold**.\n```";
        assert_eq!(
            strip_wrapping_code_fence(input),
            "# Title\n\nBody **bold**."
        );
    }

    #[test]
    fn strips_bare_wrapping_fence() {
        let input = "```\n# Title\n\nBody\n```";
        assert_eq!(strip_wrapping_code_fence(input), "# Title\n\nBody");
    }

    #[test]
    fn leaves_plain_markdown_untouched() {
        let input = "# Title\n\nBody **bold**.";
        assert_eq!(strip_wrapping_code_fence(input), input);
    }

    #[test]
    fn keeps_embedded_code_block() {
        // A summary wrapped in a fence but containing its own code block must
        // not be unwrapped, otherwise the inner block would break.
        let input = "```markdown\nHere is code:\n```python\nx = 1\n```\ndone\n```";
        assert_eq!(strip_wrapping_code_fence(input), input);
    }

    #[test]
    fn keeps_standalone_language_block() {
        // A genuine, non-markdown single code block is not a wrapper.
        let input = "```python\nprint(1)\n```";
        assert_eq!(strip_wrapping_code_fence(input), input);
    }
}
