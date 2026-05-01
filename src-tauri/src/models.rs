use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub provider: String,
    pub api_key: String,
    pub model: String,
    pub endpoint_override: Option<String>,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: "opencode_go".to_string(),
            api_key: String::new(),
            model: "qwen3.5-plus".to_string(),
            endpoint_override: None,
        }
    }
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
}

#[derive(Debug, Clone)]
pub struct VideoInfo {
    pub title: String,
    pub thumbnail_url: String,
}
