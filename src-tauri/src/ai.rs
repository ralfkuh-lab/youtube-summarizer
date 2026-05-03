use reqwest::Client;
use serde_json::{json, Value};

use crate::models::{AiChatMessage, AiConfig, AiModel, AiProviderInfo};
use crate::storage::AppResult;

const DEFAULT_SYSTEM_PROMPT: &str = r#"You are a helpful assistant that summarizes YouTube video transcripts.
Provide a clear, structured summary in the same language as the transcript.
Include:
- A short overview (1-2 sentences)
- Key points as bullet points
- Main conclusions or takeaways

Format your response as Markdown."#;

pub async fn summarize(
    client: &Client,
    ai: &AiConfig,
    transcript: &str,
    chapters: Option<&str>,
    system_prompt: Option<&str>,
    title: Option<&str>,
    published_at: Option<&str>,
) -> AppResult<String> {
    let endpoint = endpoint(ai)?;
    let mut user_content =
        String::from("Please summarize the following YouTube video transcript.\n");
    if let Some(title) = title.filter(|value| !value.trim().is_empty()) {
        user_content.push_str(&format!("\nVideo title: {title}"));
    }
    if let Some(published) = published_at.filter(|value| !value.trim().is_empty()) {
        user_content.push_str(&format!("\nPublished on: {published}"));
    }
    user_content.push_str("\n\nTranscript:\n");
    user_content.push_str(transcript);

    if let Some(chapters) = chapters.filter(|raw| !raw.trim().is_empty()) {
        user_content.push_str("\n\nAvailable chapter markers as JSON:\n");
        user_content.push_str(chapters);
    }

    let mut request = client
        .post(&endpoint)
        .header("Content-Type", "application/json");
    if !ai.api_key.trim().is_empty() {
        request = request.bearer_auth(ai.api_key.trim());
    }

    let messages = json!([
            {"role": "system", "content": system_prompt.unwrap_or(DEFAULT_SYSTEM_PROMPT)},
            {"role": "user", "content": user_content}
    ]);

    let payload = if ai.provider == "ollama_cloud" {
        json!({
            "model": ai.model,
            "messages": messages,
            "stream": false
        })
    } else {
        json!({
            "model": ai.model,
            "messages": messages,
        "temperature": 0.5
        })
    };

    let response = request
        .json(&payload)
        .send()
        .await
        .map_err(|err| format!("KI-Anfrage fehlgeschlagen: {err}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| format!("KI-Antwort konnte nicht gelesen werden: {err}"))?;

    if !status.is_success() {
        return Err(api_error_message(status.as_u16(), &body));
    }

    parse_summary_response(ai, &body)
}

pub async fn fetch_models(
    client: &Client,
    ai: &AiConfig,
    provider_id: &str,
) -> AppResult<Vec<AiModel>> {
    let mut request = client.get(models_endpoint(ai, provider_id)?);
    if !ai.api_key.trim().is_empty() {
        request = request.bearer_auth(ai.api_key.trim());
    }

    let response = request
        .send()
        .await
        .map_err(|err| format!("Modelle konnten nicht geladen werden: {err}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| format!("Modellantwort konnte nicht gelesen werden: {err}"))?;

    if !status.is_success() {
        return Err(format!(
            "Modelle konnten nicht geladen werden ({status}): {body}"
        ));
    }

    let mut models = parse_models(provider_id, &body)?;
    if provider_id == "ollama_cloud" {
        probe_ollama_cloud_free(client, ai.api_key.trim(), &mut models).await;
    }
    Ok(models)
}

pub async fn test_chat(
    client: &Client,
    ai: &AiConfig,
    messages: &[AiChatMessage],
) -> AppResult<String> {
    if ai.model.trim().is_empty() {
        return Err("Bitte zuerst ein Modell auswählen".to_string());
    }
    if messages
        .iter()
        .all(|message| message.content.trim().is_empty())
    {
        return Err("Bitte zuerst eine Testnachricht eingeben".to_string());
    }
    let messages = messages
        .iter()
        .filter(|message| !message.content.trim().is_empty())
        .map(|message| {
            json!({
                "role": message.role,
                "content": message.content
            })
        })
        .collect::<Vec<_>>();

    let endpoint = endpoint(ai)?;
    let mut request = client
        .post(&endpoint)
        .header("Content-Type", "application/json");
    if !ai.api_key.trim().is_empty() {
        request = request.bearer_auth(ai.api_key.trim());
    }

    let payload = if ai.provider == "ollama_cloud" {
        json!({
            "model": ai.model,
            "messages": messages,
            "stream": false
        })
    } else {
        json!({
            "model": ai.model,
            "messages": messages,
            "temperature": 0
        })
    };

    let response = request
        .json(&payload)
        .send()
        .await
        .map_err(|err| format!("Verbindungstest fehlgeschlagen: {err}"))?;
    let status = response.status();
    let body = response.text().await.map_err(|err| {
        format!("Antwort des Verbindungstests konnte nicht gelesen werden: {err}")
    })?;

    if !status.is_success() {
        return Err(api_error_message(status.as_u16(), &body));
    }

    parse_summary_response(ai, &body)
}

async fn probe_ollama_cloud_free(client: &Client, api_key: &str, models: &mut [AiModel]) {
    use std::sync::Arc;
    use tokio::sync::Semaphore;
    use tokio::task::JoinSet;

    if api_key.is_empty() || models.is_empty() {
        return;
    }
    let permits = Arc::new(Semaphore::new(6));
    let mut tasks: JoinSet<(String, &'static str)> = JoinSet::new();
    for model in models.iter() {
        let id = model.id.clone();
        let client = client.clone();
        let key = api_key.to_string();
        let permits = Arc::clone(&permits);
        tasks.spawn(async move {
            let _permit = permits.acquire_owned().await.ok();
            let payload = json!({
                "model": id,
                "messages": [{"role": "user", "content": "ping"}],
                "stream": false,
                "options": {"num_predict": 1}
            });
            let response = client
                .post("https://ollama.com/api/chat")
                .bearer_auth(&key)
                .header("Content-Type", "application/json")
                .json(&payload)
                .send()
                .await;
            let availability = match response {
                Ok(r) if r.status().as_u16() == 403 => "subscription_required",
                Ok(r) if r.status().is_success() => "free",
                _ => "unknown",
            };
            (id, availability)
        });
    }
    let mut results = std::collections::HashMap::new();
    while let Some(joined) = tasks.join_next().await {
        if let Ok((id, availability)) = joined {
            results.insert(id, availability);
        }
    }
    for model in models.iter_mut() {
        match results.get(&model.id).copied() {
            Some("free") => {
                model.free = true;
                model.availability = Some("free".to_string());
            }
            Some("subscription_required") => {
                model.free = false;
                model.availability = Some("subscription_required".to_string());
            }
            _ => {
                model.free = false;
                model.availability = Some("unknown".to_string());
            }
        }
    }
}

pub fn provider_catalog() -> Vec<AiProviderInfo> {
    vec![
        AiProviderInfo {
            id: "ollama_cloud".to_string(),
            name: "Ollama Cloud".to_string(),
            description: "Cloud models through Ollama. Some models are usable within a free allowance, others require a paid subscription. Refreshing the model list probes each model and tags the freely usable ones.".to_string(),
            badge: "Easy".to_string(),
            homepage_url: Some("https://ollama.com".to_string()),
            default_endpoint: Some("https://ollama.com/api/chat".to_string()),
            requires_api_key: true,
            supports_model_refresh: true,
            endpoint_editable: false,
            recommended: true,
        },
        AiProviderInfo {
            id: "openrouter".to_string(),
            name: "OpenRouter".to_string(),
            description: "One API for many hosted OpenAI-compatible models. Some models may be free depending on OpenRouter availability.".to_string(),
            badge: "Multi-provider".to_string(),
            homepage_url: Some("https://openrouter.ai".to_string()),
            default_endpoint: Some("https://openrouter.ai/api/v1/chat/completions".to_string()),
            requires_api_key: true,
            supports_model_refresh: true,
            endpoint_editable: false,
            recommended: true,
        },
        AiProviderInfo {
            id: "opencode_zen".to_string(),
            name: "OpenCode Zen".to_string(),
            description: "Curated models from OpenCode, including some free models.".to_string(),
            badge: "Some free models".to_string(),
            homepage_url: Some("https://opencode.ai".to_string()),
            default_endpoint: Some("https://opencode.ai/zen/v1/chat/completions".to_string()),
            requires_api_key: true,
            supports_model_refresh: true,
            endpoint_editable: false,
            recommended: true,
        },
        AiProviderInfo {
            id: "opencode_go".to_string(),
            name: "OpenCode Go".to_string(),
            description: "Curated OpenCode models with an affordable subscription available.".to_string(),
            badge: "Subscription".to_string(),
            homepage_url: Some("https://opencode.ai".to_string()),
            default_endpoint: Some("https://opencode.ai/zen/go/v1/chat/completions".to_string()),
            requires_api_key: true,
            supports_model_refresh: true,
            endpoint_editable: false,
            recommended: true,
        },
        AiProviderInfo {
            id: "ollama".to_string(),
            name: "Ollama local".to_string(),
            description: "Local models through Ollama's OpenAI-compatible API.".to_string(),
            badge: "Local".to_string(),
            homepage_url: Some("https://ollama.com".to_string()),
            default_endpoint: Some("http://localhost:11434/v1/chat/completions".to_string()),
            requires_api_key: false,
            supports_model_refresh: true,
            endpoint_editable: true,
            recommended: false,
        },
        AiProviderInfo {
            id: "custom".to_string(),
            name: "Custom".to_string(),
            description: "OpenAI-compatible chat completions endpoint, for example LM Studio or llama.cpp.".to_string(),
            badge: "Flexible".to_string(),
            homepage_url: None,
            default_endpoint: None,
            requires_api_key: false,
            supports_model_refresh: true,
            endpoint_editable: true,
            recommended: false,
        },
    ]
}

fn endpoint(ai: &AiConfig) -> AppResult<String> {
    if let Some(endpoint) = ai
        .endpoint_override
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(endpoint.to_string());
    }

    match ai.provider.as_str() {
        "opencode_zen" => Ok("https://opencode.ai/zen/v1/chat/completions".to_string()),
        "opencode_go" => Ok("https://opencode.ai/zen/go/v1/chat/completions".to_string()),
        "openrouter" => Ok("https://openrouter.ai/api/v1/chat/completions".to_string()),
        "ollama_cloud" => Ok("https://ollama.com/api/chat".to_string()),
        "ollama" => Ok("http://localhost:11434/v1/chat/completions".to_string()),
        _ => Err("Unbekannter KI-Provider".to_string()),
    }
}

fn models_endpoint(ai: &AiConfig, provider_id: &str) -> AppResult<String> {
    match provider_id {
        "opencode_go" => Ok("https://opencode.ai/zen/go/v1/models".to_string()),
        "opencode_zen" => Ok("https://opencode.ai/zen/v1/models".to_string()),
        "openrouter" => Ok("https://openrouter.ai/api/v1/models".to_string()),
        "ollama_cloud" => Ok("https://ollama.com/api/tags".to_string()),
        "ollama" => Ok("http://localhost:11434/v1/models".to_string()),
        id if id.starts_with("custom") => ai
            .endpoint_override
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(derive_models_endpoint)
            .ok_or_else(|| "Bitte zuerst einen benutzerdefinierten Endpunkt eintragen".to_string()),
        _ => Err("Unbekannter KI-Provider".to_string()),
    }
}

fn derive_models_endpoint(endpoint: &str) -> String {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if let Some(base) = trimmed.strip_suffix("/chat/completions") {
        format!("{base}/models")
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/models")
    } else {
        format!("{trimmed}/models")
    }
}

fn parse_summary_response(ai: &AiConfig, body: &str) -> AppResult<String> {
    let value: Value = serde_json::from_str(body)
        .map_err(|err| format!("KI-Antwort ist kein gültiges JSON: {err}"))?;
    let content = if ai.provider == "ollama_cloud" {
        value.pointer("/message/content").and_then(Value::as_str)
    } else {
        value
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
    };

    content
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "KI hat eine leere oder unerwartete Antwort zurückgegeben".to_string())
}

fn api_error_message(status: u16, body: &str) -> String {
    let parsed = serde_json::from_str::<Value>(body).ok();
    let error = parsed.as_ref().and_then(|value| value.get("error"));
    let message = error
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
        .or_else(|| {
            parsed
                .as_ref()
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
        });
    let error_type = error
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    if error_type == "CreditsError"
        || message.is_some_and(|value| value.to_lowercase().contains("balance"))
    {
        return format!(
            "Nicht genug Guthaben für dieses Modell. {}",
            message.unwrap_or("Bitte Billing/Guthaben beim Anbieter prüfen.")
        );
    }

    if status == 401 || status == 403 {
        if let Some(message) = message {
            return format!("KI-Anbieter lehnt die Anfrage ab: {message}");
        }
        return "API-Key ungültig oder ohne Berechtigung - bitte in den Einstellungen prüfen"
            .to_string();
    }

    if let Some(message) = message {
        return format!("KI-Anfrage fehlgeschlagen ({status}): {message}");
    }

    format!("KI-Anfrage fehlgeschlagen ({status}): {body}")
}

fn parse_models(provider_id: &str, body: &str) -> AppResult<Vec<AiModel>> {
    let value: Value = serde_json::from_str(body)
        .map_err(|err| format!("Modellantwort ist kein gültiges JSON: {err}"))?;
    let items = if provider_id == "ollama_cloud" {
        value.pointer("/models").and_then(Value::as_array)
    } else {
        value.pointer("/data").and_then(Value::as_array)
    }
    .ok_or_else(|| "Modellantwort enthält keine Modellliste".to_string())?;

    let mut models = items
        .iter()
        .filter_map(|item| {
            let id = item
                .get("id")
                .or_else(|| item.get("model"))
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)?;
            if provider_id == "ollama" && id.ends_with(":cloud") {
                return None;
            }
            Some(AiModel {
                id: id.to_string(),
                name: model_name(id),
                tags: model_tags(provider_id, id),
                free: id.contains("free") || provider_id == "ollama",
                availability: default_model_availability(provider_id, id),
            })
        })
        .collect::<Vec<_>>();
    models.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(models)
}

fn default_model_availability(provider_id: &str, id: &str) -> Option<String> {
    if provider_id == "ollama" || id.contains("free") {
        Some("free".to_string())
    } else {
        None
    }
}

fn model_name(id: &str) -> String {
    id.replace(['-', '_', ':'], " ")
        .split_whitespace()
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn model_tags(provider_id: &str, id: &str) -> Vec<String> {
    let mut tags = Vec::new();
    if provider_id == "ollama" {
        tags.push("Local".to_string());
    }
    if id.contains("free") {
        tags.push("Free".to_string());
    }
    tags
}
