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
    pub api_key: String,
    pub model: String,
    pub endpoint_override: Option<String>,
    #[serde(default)]
    pub models: Vec<AiModel>,
    pub models_updated_at: Option<String>,
    pub last_error: Option<String>,
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
            api_key: String::new(),
            model: "gpt-oss:120b".to_string(),
            endpoint_override: None,
            models: Vec::new(),
            models_updated_at: None,
            last_error: None,
        },
        AiProviderConfig {
            id: "openrouter".to_string(),
            name: None,
            enabled: true,
            api_key: String::new(),
            model: "openai/gpt-oss-120b".to_string(),
            endpoint_override: None,
            models: Vec::new(),
            models_updated_at: None,
            last_error: None,
        },
        AiProviderConfig {
            id: "opencode_zen".to_string(),
            name: None,
            enabled: true,
            api_key: String::new(),
            model: "minimax-m2.5-free".to_string(),
            endpoint_override: None,
            models: Vec::new(),
            models_updated_at: None,
            last_error: None,
        },
        AiProviderConfig {
            id: "opencode_go".to_string(),
            name: None,
            enabled: true,
            api_key: String::new(),
            model: "qwen3.6-plus".to_string(),
            endpoint_override: None,
            models: Vec::new(),
            models_updated_at: None,
            last_error: None,
        },
    ]
}

fn default_provider_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub ai: AiConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ai: AiConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub time: String,
    pub start: f64,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSnippet {
    pub text: String,
    pub start: f64,
    pub time: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Video {
    pub id: i64,
    pub video_id: String,
    pub url: String,
    pub title: String,
    pub thumbnail_url: String,
    pub thumbnail: Option<String>,
    pub transcript: Option<String>,
    pub chapters: Option<Vec<Chapter>>,
    pub summary: Option<String>,
    pub summary_provider: Option<String>,
    pub summary_model: Option<String>,
    pub published_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewVideo {
    pub video_id: String,
    pub url: String,
    pub title: String,
    pub thumbnail_url: String,
    pub thumbnail_data: Option<Vec<u8>>,
    pub transcript: Option<String>,
    pub chapters: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VideoInfo {
    pub title: String,
    pub thumbnail_url: String,
    pub published_at: Option<String>,
}
