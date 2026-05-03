use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::storage::{self, AppPaths, AppResult};
use crate::{ai, commands};

#[derive(Debug, Deserialize)]
struct AddVideoRequest {
    url: String,
}

#[derive(Debug, Deserialize)]
struct SummarizeRequest {
    system_prompt: Option<String>,
}

pub fn start(paths: AppPaths) {
    thread::spawn(move || {
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(err) => {
                eprintln!("Automation API konnte nicht gestartet werden: {err}");
                return;
            }
        };

        let addr = match listener.local_addr() {
            Ok(addr) => addr,
            Err(err) => {
                eprintln!("Automation API Adresse konnte nicht gelesen werden: {err}");
                return;
            }
        };

        println!("AUTOMATION_URL=http://{addr}/api");

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let paths = paths.clone();
                    thread::spawn(move || {
                        if let Err(err) = handle_connection(stream, &paths) {
                            eprintln!("Automation API Fehler: {err}");
                        }
                    });
                }
                Err(err) => eprintln!("Automation API Verbindung fehlgeschlagen: {err}"),
            }
        }
    });
}

fn handle_connection(mut stream: TcpStream, paths: &AppPaths) -> AppResult<()> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|err| format!("Read timeout konnte nicht gesetzt werden: {err}"))?;

    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|err| format!("Stream konnte nicht geklont werden: {err}"))?,
    );
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|err| format!("Request-Zeile konnte nicht gelesen werden: {err}"))?;

    let parts = request_line.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 2 {
        return write_json(&mut stream, 400, &json!({"error": "Invalid request"}));
    }

    let method = parts[0];
    let path = parts[1].trim_end_matches('/').to_string();
    let headers = read_headers(&mut reader)?;
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader
            .read_exact(&mut body)
            .map_err(|err| format!("Request-Body konnte nicht gelesen werden: {err}"))?;
    }

    route(&mut stream, paths, method, &path, &body)
}

fn read_headers(reader: &mut BufReader<TcpStream>) -> AppResult<HashMap<String, String>> {
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|err| format!("Header konnten nicht gelesen werden: {err}"))?;
        let line = line.trim();
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    Ok(headers)
}

fn route(
    stream: &mut TcpStream,
    paths: &AppPaths,
    method: &str,
    path: &str,
    body: &[u8],
) -> AppResult<()> {
    match (method, path) {
        ("GET", "/api/health") => write_json(stream, 200, &json!({"status": "ok"})),
        ("GET", "/api/config") => write_result(stream, storage::get_ai_config(paths)),
        ("GET", "/api/providers") => write_json(stream, 200, &ai::provider_catalog()),
        _ if method == "POST" && path.starts_with("/api/models/") => {
            let provider_id = path
                .strip_prefix("/api/models/")
                .ok_or_else(|| "Ungültiger Pfad".to_string())?;
            let runtime = tokio::runtime::Runtime::new()
                .map_err(|err| format!("Runtime konnte nicht erstellt werden: {err}"))?;
            write_result(
                stream,
                runtime.block_on(refresh_models_impl(paths, provider_id)),
            )
        }
        ("GET", "/api/videos") => write_result(stream, storage::get_videos(paths)),
        ("POST", "/api/add-video") => {
            let request = parse_body::<AddVideoRequest>(body)?;
            let runtime = tokio::runtime::Runtime::new()
                .map_err(|err| format!("Runtime konnte nicht erstellt werden: {err}"))?;
            write_result(
                stream,
                runtime.block_on(commands::add_video_impl(paths, request.url)),
            )
        }
        _ if method == "GET" && path.starts_with("/api/video/") => {
            let id = parse_id(path, "/api/video/")?;
            write_result(
                stream,
                storage::get_video(paths, id)?.ok_or_else(|| "Video nicht gefunden".to_string()),
            )
        }
        _ if method == "DELETE" && path.starts_with("/api/video/") => {
            let id = parse_id(path, "/api/video/")?;
            write_result(
                stream,
                storage::delete_video(paths, id).map(|_| json!({"status": "ok"})),
            )
        }
        _ if method == "POST" && path.starts_with("/api/transcript/") => {
            let id = parse_id(path, "/api/transcript/")?;
            let runtime = tokio::runtime::Runtime::new()
                .map_err(|err| format!("Runtime konnte nicht erstellt werden: {err}"))?;
            write_result(
                stream,
                runtime.block_on(commands::refresh_transcript_impl(paths, id)),
            )
        }
        _ if method == "POST" && path.starts_with("/api/summarize/") => {
            let id = parse_id(path, "/api/summarize/")?;
            let request = parse_body::<SummarizeRequest>(body)?;
            let runtime = tokio::runtime::Runtime::new()
                .map_err(|err| format!("Runtime konnte nicht erstellt werden: {err}"))?;
            write_result(
                stream,
                runtime.block_on(commands::summarize_video_impl(
                    paths,
                    id,
                    request.system_prompt.unwrap_or_default(),
                )),
            )
        }
        _ => write_json(stream, 404, &json!({"error": "Not found"})),
    }
}

async fn refresh_models_impl(
    paths: &AppPaths,
    provider_id: &str,
) -> AppResult<crate::models::AiConfig> {
    let config = storage::get_ai_config(paths)?;
    let provider = storage::provider_config(&config, provider_id)
        .ok_or_else(|| "KI-Anbieter nicht gefunden".to_string())?;
    let mut request_config = config.clone();
    request_config.provider = provider.id.clone();
    request_config.api_key = provider.api_key.clone();
    request_config.model = provider.model.clone();
    request_config.endpoint_override = provider.endpoint_override.clone();
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 YouTubeSummarizer/0.1")
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|err| format!("HTTP-Client konnte nicht erstellt werden: {err}"))?;
    let existing_models = provider.models.clone();
    let account_tier = provider
        .account_tier
        .clone()
        .unwrap_or_else(|| "free".to_string());
    let models = ai::fetch_models(
        &client,
        &request_config,
        provider_id,
        &existing_models,
        &account_tier,
        false,
    )
    .await?;
    storage::update_provider_models(
        paths,
        provider_id,
        models,
        chrono::Utc::now().to_rfc3339(),
        None,
    )
}

fn parse_body<T: for<'de> Deserialize<'de>>(body: &[u8]) -> AppResult<T> {
    serde_json::from_slice(body).map_err(|err| format!("Request-Body ist ungültig: {err}"))
}

fn parse_id(path: &str, prefix: &str) -> AppResult<i64> {
    path.strip_prefix(prefix)
        .ok_or_else(|| "Ungültiger Pfad".to_string())?
        .parse::<i64>()
        .map_err(|_| "Ungültige Video-ID".to_string())
}

fn write_result<T: serde::Serialize>(
    stream: &mut TcpStream,
    result: AppResult<T>,
) -> AppResult<()> {
    match result {
        Ok(value) => write_json(stream, 200, &value),
        Err(err) => write_json(stream, 400, &json!({"error": err})),
    }
}

fn write_json<T: serde::Serialize>(
    stream: &mut TcpStream,
    status: u16,
    value: &T,
) -> AppResult<()> {
    let body = serde_json::to_vec(value)
        .map_err(|err| format!("JSON konnte nicht erzeugt werden: {err}"))?;
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .map_err(|err| format!("Response-Header konnte nicht geschrieben werden: {err}"))?;
    stream
        .write_all(&body)
        .map_err(|err| format!("Response-Body konnte nicht geschrieben werden: {err}"))
}
