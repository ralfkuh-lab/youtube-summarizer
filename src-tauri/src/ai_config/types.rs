use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub provider: String,
    pub api_key: String,
    pub model: String,
    pub endpoint_override: Option<String>,
    #[serde(default)]
    pub providers: Vec<AiProviderConfig>,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: "opencode_go".to_string(),
            api_key: String::new(),
            model: "qwen3.5-plus".to_string(),
            endpoint_override: None,
            providers: default_ai_provider_configs(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProviderConfig {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "default_provider_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub api_key_required: bool,
    pub api_key: String,
    pub model: String,
    pub endpoint_override: Option<String>,
    #[serde(default)]
    pub models: Vec<AiModel>,
    pub models_updated_at: Option<String>,
    pub last_error: Option<String>,
    #[serde(default)]
    pub account_tier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiModel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub free: bool,
    #[serde(default)]
    pub availability: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiProviderInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub badge: String,
    pub homepage_url: Option<String>,
    pub default_endpoint: Option<String>,
    pub requires_api_key: bool,
    pub supports_model_refresh: bool,
    pub endpoint_editable: bool,
    pub recommended: bool,
}

pub fn default_ai_provider_configs() -> Vec<AiProviderConfig> {
    vec![
        AiProviderConfig {
            id: "ollama_cloud".to_string(),
            name: None,
            enabled: true,
            api_key_required: true,
            api_key: String::new(),
            model: "gpt-oss:120b".to_string(),
            endpoint_override: None,
            models: Vec::new(),
            models_updated_at: None,
            last_error: None,
            account_tier: None,
        },
        AiProviderConfig {
            id: "openrouter".to_string(),
            name: None,
            enabled: true,
            api_key_required: true,
            api_key: String::new(),
            model: "openai/gpt-oss-120b".to_string(),
            endpoint_override: None,
            models: Vec::new(),
            models_updated_at: None,
            last_error: None,
            account_tier: None,
        },
        AiProviderConfig {
            id: "opencode_zen".to_string(),
            name: None,
            enabled: true,
            api_key_required: true,
            api_key: String::new(),
            model: "minimax-m2.5-free".to_string(),
            endpoint_override: None,
            models: Vec::new(),
            models_updated_at: None,
            last_error: None,
            account_tier: None,
        },
        AiProviderConfig {
            id: "opencode_go".to_string(),
            name: None,
            enabled: true,
            api_key_required: true,
            api_key: String::new(),
            model: "qwen3.6-plus".to_string(),
            endpoint_override: None,
            models: Vec::new(),
            models_updated_at: None,
            last_error: None,
            account_tier: None,
        },
    ]
}

fn default_provider_enabled() -> bool {
    true
}
