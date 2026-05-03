use super::types::{default_ai_provider_configs, AiConfig, AiModel, AiProviderConfig};
use crate::storage::{load_config, save_config, AppPaths, AppResult};

pub fn get_ai_config(paths: &AppPaths) -> AppResult<AiConfig> {
    let mut config = load_config(paths)?;
    config.ai = normalize_ai_config(config.ai);
    save_config(paths, &config)?;
    Ok(config.ai)
}

pub fn update_ai_config(paths: &AppPaths, ai: AiConfig) -> AppResult<AiConfig> {
    let mut config = load_config(paths)?;
    config.ai = normalize_ai_config(ai);
    save_config(paths, &config)?;
    Ok(config.ai)
}

#[allow(clippy::too_many_arguments)]
pub fn update_provider_config(
    paths: &AppPaths,
    provider_id: String,
    name: Option<String>,
    enabled: bool,
    api_key_required: Option<bool>,
    api_key: String,
    model: String,
    endpoint_override: Option<String>,
    activate: bool,
    account_tier: Option<String>,
) -> AppResult<AiConfig> {
    let mut config = load_config(paths)?;
    let mut ai = normalize_ai_config(config.ai);
    let existing_models = provider_config(&ai, &provider_id)
        .map(|provider| provider.models.clone())
        .unwrap_or_default();
    let existing_models_updated_at =
        provider_config(&ai, &provider_id).and_then(|provider| provider.models_updated_at.clone());
    let existing_name =
        provider_config(&ai, &provider_id).and_then(|provider| provider.name.clone());
    let existing_api_key_required =
        provider_config(&ai, &provider_id).map(|provider| provider.api_key_required);
    let existing_account_tier =
        provider_config(&ai, &provider_id).and_then(|provider| provider.account_tier.clone());
    upsert_provider(
        &mut ai,
        AiProviderConfig {
            id: provider_id.clone(),
            name: normalize_name(name).or(existing_name),
            enabled,
            api_key_required: api_key_required
                .or(existing_api_key_required)
                .unwrap_or(false),
            api_key: api_key.clone(),
            model: model.clone(),
            endpoint_override: normalize_endpoint(endpoint_override.clone()),
            models: existing_models,
            models_updated_at: existing_models_updated_at,
            last_error: None,
            account_tier: account_tier.or(existing_account_tier),
        },
    );
    sync_shared_provider_secrets(&mut ai, &provider_id);
    if activate || ai.provider == provider_id {
        ai.provider = provider_id;
        if let Some(active) = ai
            .providers
            .iter()
            .find(|provider| provider.id == ai.provider)
        {
            ai.api_key = active.api_key.clone();
            ai.model = active.model.clone();
            ai.endpoint_override = active.endpoint_override.clone();
        }
    }
    config.ai = normalize_ai_config(ai);
    save_config(paths, &config)?;
    Ok(config.ai)
}

pub fn add_custom_provider(paths: &AppPaths, local_ollama: bool) -> AppResult<AiConfig> {
    let mut config = load_config(paths)?;
    let mut ai = normalize_ai_config(config.ai);
    if local_ollama {
        upsert_provider(
            &mut ai,
            AiProviderConfig {
                id: "ollama".to_string(),
                name: Some("Ollama local".to_string()),
                enabled: true,
                api_key_required: false,
                api_key: String::new(),
                model: "llama3.2".to_string(),
                endpoint_override: Some("http://localhost:11434/v1/chat/completions".to_string()),
                models: Vec::new(),
                models_updated_at: None,
                last_error: None,
                account_tier: None,
            },
        );
    } else {
        let next_number = ai
            .providers
            .iter()
            .filter(|provider| provider.id.starts_with("custom"))
            .count()
            + 1;
        let id = format!("custom_{next_number}");
        upsert_provider(
            &mut ai,
            AiProviderConfig {
                id,
                name: Some(format!("Custom {next_number}")),
                enabled: true,
                api_key_required: false,
                api_key: String::new(),
                model: String::new(),
                endpoint_override: Some("http://localhost:1234/v1/chat/completions".to_string()),
                models: Vec::new(),
                models_updated_at: None,
                last_error: None,
                account_tier: None,
            },
        );
    }
    config.ai = normalize_ai_config(ai);
    save_config(paths, &config)?;
    Ok(config.ai)
}

pub fn delete_custom_provider(paths: &AppPaths, provider_id: &str) -> AppResult<AiConfig> {
    if !is_user_managed_provider(provider_id) {
        return Err("Only custom/local providers can be deleted".to_string());
    }

    let mut config = load_config(paths)?;
    let mut ai = normalize_ai_config(config.ai);
    let original_len = ai.providers.len();
    ai.providers.retain(|provider| provider.id != provider_id);
    if ai.providers.len() == original_len {
        return Err("Provider not found".to_string());
    }

    if ai.provider == provider_id {
        ai.provider = "ollama_cloud".to_string();
    }

    config.ai = normalize_ai_config(ai);
    save_config(paths, &config)?;
    Ok(config.ai)
}

fn is_user_managed_provider(provider_id: &str) -> bool {
    provider_id == "ollama" || provider_id.starts_with("custom")
}

pub fn update_provider_models(
    paths: &AppPaths,
    provider_id: &str,
    models: Vec<AiModel>,
    updated_at: String,
    last_error: Option<String>,
) -> AppResult<AiConfig> {
    let mut config = load_config(paths)?;
    let mut ai = normalize_ai_config(config.ai);
    if let Some(provider) = ai
        .providers
        .iter_mut()
        .find(|provider| provider.id == provider_id)
    {
        let selected_model = if provider.model.trim().is_empty()
            || !models.iter().any(|model| model.id == provider.model)
        {
            preferred_model_id(&models)
        } else {
            Some(provider.model.clone())
        };

        provider.models = models;
        if let Some(model) = selected_model {
            provider.model = model;
        }
        provider.models_updated_at = Some(updated_at);
        provider.last_error = last_error;
    }
    if ai.provider == provider_id {
        if let Some(active) = ai
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
        {
            ai.model = active.model.clone();
        }
    }
    config.ai = normalize_ai_config(ai);
    save_config(paths, &config)?;
    Ok(config.ai)
}

fn preferred_model_id(models: &[AiModel]) -> Option<String> {
    models
        .iter()
        .find(|model| model.free)
        .or_else(|| models.first())
        .map(|model| model.id.clone())
}

pub fn set_provider_error(
    paths: &AppPaths,
    provider_id: &str,
    error: String,
) -> AppResult<AiConfig> {
    set_provider_last_error(paths, provider_id, Some(error))
}

fn set_provider_last_error(
    paths: &AppPaths,
    provider_id: &str,
    error: Option<String>,
) -> AppResult<AiConfig> {
    let mut config = load_config(paths)?;
    let mut ai = normalize_ai_config(config.ai);
    if let Some(provider) = ai
        .providers
        .iter_mut()
        .find(|provider| provider.id == provider_id)
    {
        provider.last_error = error;
    }
    config.ai = normalize_ai_config(ai);
    save_config(paths, &config)?;
    Ok(config.ai)
}

pub fn normalize_ai_config(mut ai: AiConfig) -> AiConfig {
    let defaults = default_ai_provider_configs();
    if ai.providers.is_empty() {
        ai.providers = defaults.clone();
    }

    for default_provider in defaults {
        if !ai
            .providers
            .iter()
            .any(|provider| provider.id == default_provider.id)
        {
            ai.providers.push(default_provider);
        }
    }

    if let Some(active) = ai
        .providers
        .iter_mut()
        .find(|provider| provider.id == ai.provider)
    {
        if !ai.api_key.is_empty() {
            active.api_key = ai.api_key.clone();
        }
        if !ai.model.is_empty() {
            active.model = ai.model.clone();
        }
        if ai.endpoint_override.is_some() {
            active.endpoint_override = ai.endpoint_override.clone();
        }
    }

    if let Some(active) = ai
        .providers
        .iter()
        .find(|provider| provider.id == ai.provider)
    {
        ai.api_key = active.api_key.clone();
        ai.model = active.model.clone();
        ai.endpoint_override = active.endpoint_override.clone();
    }

    ai
}

pub fn provider_config<'a>(ai: &'a AiConfig, provider_id: &str) -> Option<&'a AiProviderConfig> {
    ai.providers
        .iter()
        .find(|provider| provider.id == provider_id)
}

fn upsert_provider(ai: &mut AiConfig, next: AiProviderConfig) {
    if let Some(provider) = ai
        .providers
        .iter_mut()
        .find(|provider| provider.id == next.id)
    {
        *provider = next;
    } else {
        ai.providers.push(next);
    }
}

fn sync_shared_provider_secrets(ai: &mut AiConfig, provider_id: &str) {
    if !provider_id.starts_with("opencode_") {
        return;
    }

    let shared_key = ai
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .map(|provider| provider.api_key.clone())
        .unwrap_or_default();

    for provider in ai
        .providers
        .iter_mut()
        .filter(|provider| provider.id.starts_with("opencode_"))
    {
        provider.api_key = shared_key.clone();
    }
}

fn normalize_endpoint(endpoint_override: Option<String>) -> Option<String> {
    endpoint_override.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn normalize_name(name: Option<String>) -> Option<String> {
    name.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}
