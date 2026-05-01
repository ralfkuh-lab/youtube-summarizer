use reqwest::Client;
use serde_json::{json, Value};

use crate::models::AiConfig;
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
) -> AppResult<String> {
    let endpoint = endpoint(ai)?;
    let mut user_content =
        String::from("Please summarize the following YouTube video transcript:\n\n");
    user_content.push_str(transcript);

    if let Some(chapters) = chapters.filter(|raw| !raw.trim().is_empty()) {
        user_content.push_str("\n\nAvailable chapter markers as JSON:\n");
        user_content.push_str(chapters);
    }

    let mut request = client
        .post(endpoint)
        .header("Content-Type", "application/json");
    if !ai.api_key.trim().is_empty() {
        request = request.bearer_auth(ai.api_key.trim());
    }

    let payload = json!({
        "model": ai.model,
        "messages": [
            {"role": "system", "content": system_prompt.unwrap_or(DEFAULT_SYSTEM_PROMPT)},
            {"role": "user", "content": user_content}
        ],
        "temperature": 0.5
    });

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
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err("API-Key ungültig - bitte in den Einstellungen prüfen".to_string());
        }
        return Err(format!("KI-Anfrage fehlgeschlagen ({status}): {body}"));
    }

    let value: Value = serde_json::from_str(&body)
        .map_err(|err| format!("KI-Antwort ist kein gültiges JSON: {err}"))?;
    value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "KI hat eine leere oder unerwartete Antwort zurückgegeben".to_string())
}

fn endpoint(ai: &AiConfig) -> AppResult<&str> {
    if let Some(endpoint) = ai
        .endpoint_override
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(endpoint);
    }

    match ai.provider.as_str() {
        "opencode_zen" => Ok("https://opencode.ai/zen/v1/chat/completions"),
        "opencode_go" => Ok("https://opencode.ai/zen/go/v1/chat/completions"),
        "openrouter" => Ok("https://openrouter.ai/api/v1/chat/completions"),
        "ollama" => Ok("http://localhost:11434/v1/chat/completions"),
        _ => Err("Unbekannter KI-Provider".to_string()),
    }
}
