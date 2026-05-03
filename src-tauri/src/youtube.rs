use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::models::{Chapter, TranscriptSnippet, VideoInfo};
use crate::storage::AppResult;

const LANGUAGES: &[&str] = &[
    "de", "en", "fr", "es", "it", "nl", "pl", "ru", "ja", "ko", "pt", "ar", "tr",
];

#[derive(Debug, Deserialize)]
struct OEmbedResponse {
    title: Option<String>,
}

pub fn extract_video_id(input: &str) -> Option<String> {
    let patterns = [
        r"(?:v=|/v/|youtu\.be/|/embed/|/shorts/)([A-Za-z0-9_-]{11})",
        r"^([A-Za-z0-9_-]{11})$",
    ];

    patterns.iter().find_map(|pattern| {
        Regex::new(pattern)
            .ok()
            .and_then(|re| re.captures(input))
            .and_then(|captures| captures.get(1))
            .map(|m| m.as_str().to_string())
    })
}

pub fn video_url(video_id: &str) -> String {
    format!("https://www.youtube.com/watch?v={video_id}")
}

pub fn thumbnail_url(video_id: &str) -> String {
    format!("https://i.ytimg.com/vi/{video_id}/hqdefault.jpg")
}

pub async fn fetch_video_info(client: &Client, video_id: &str) -> AppResult<VideoInfo> {
    let url = video_url(video_id);
    let oembed_url = format!("https://www.youtube.com/oembed?url={url}&format=json");
    let oembed_future = client.get(oembed_url).send();
    let publish_date_future = fetch_publish_date(client, video_id);
    let (oembed_response, published_at) = tokio::join!(oembed_future, publish_date_future);

    let data = oembed_response
        .map_err(|err| format!("YouTube-Metadaten konnten nicht geladen werden: {err}"))?
        .error_for_status()
        .map_err(|err| format!("YouTube-Metadaten konnten nicht geladen werden: {err}"))?
        .json::<OEmbedResponse>()
        .await
        .map_err(|err| format!("YouTube-Metadaten konnten nicht gelesen werden: {err}"))?;

    Ok(VideoInfo {
        title: data.title.unwrap_or_else(|| video_id.to_string()),
        thumbnail_url: thumbnail_url(video_id),
        published_at,
    })
}

pub async fn fetch_publish_date(client: &Client, video_id: &str) -> Option<String> {
    let html = fetch_watch_html(client, video_id).await.ok()?;
    let pattern = Regex::new(r#""publishDate"\s*:\s*"([0-9]{4}-[0-9]{2}-[0-9]{2})"#).ok()?;
    pattern
        .captures(&html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

pub async fn download_thumbnail(client: &Client, video_id: &str) -> Option<Vec<u8>> {
    let response = client.get(thumbnail_url(video_id)).send().await.ok()?;
    let response = response.error_for_status().ok()?;
    response.bytes().await.ok().map(|bytes| bytes.to_vec())
}

pub async fn fetch_transcript(client: &Client, video_id: &str) -> AppResult<String> {
    let html = fetch_watch_html(client, video_id).await?;
    let api_key = extract_innertube_api_key(&html)
        .ok_or_else(|| "YouTube-Innertube-API-Key wurde nicht gefunden".to_string())?;
    let player = fetch_innertube_player(client, video_id, &api_key).await?;

    let tracks = player
        .pointer("/captions/playerCaptionsTracklistRenderer/captionTracks")
        .and_then(Value::as_array)
        .ok_or_else(|| "Für dieses Video wurde kein Transkript gefunden".to_string())?;

    let track = select_caption_track(tracks)
        .ok_or_else(|| "Kein unterstütztes Transkript gefunden".to_string())?;
    let base_url = track
        .get("baseUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| "Transkript-URL fehlt".to_string())?;

    let transcript_url = with_json3_format(base_url)?;
    let payload = client
        .get(transcript_url)
        .send()
        .await
        .map_err(|err| format!("Transkript konnte nicht geladen werden: {err}"))?
        .error_for_status()
        .map_err(|err| format!("Transkript konnte nicht geladen werden: {err}"))?
        .json::<Value>()
        .await
        .map_err(|err| format!("Transkript konnte nicht gelesen werden: {err}"))?;

    let snippets = parse_json3_transcript(&payload);
    if snippets.is_empty() {
        return Err("Transkript ist leer".to_string());
    }

    serde_json::to_string(&snippets)
        .map_err(|err| format!("Transkript konnte nicht serialisiert werden: {err}"))
}

pub async fn fetch_chapters(client: &Client, video_id: &str) -> Option<String> {
    let html = fetch_watch_html(client, video_id).await.ok()?;

    for var_name in ["ytInitialData", "ytInitialPlayerResponse"] {
        if let Some(data) = extract_json_assignment(&html, var_name) {
            if let Some(chapters) = extract_chapters_from_data(&data) {
                return serde_json::to_string(&chapters).ok();
            }
        }
    }

    None
}

pub fn transcript_to_text(transcript_json: &str) -> String {
    serde_json::from_str::<Vec<TranscriptSnippet>>(transcript_json)
        .map(|snippets| {
            snippets
                .into_iter()
                .map(|snippet| snippet.text)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_else(|_| transcript_json.to_string())
}

fn select_caption_track<'a>(tracks: &'a [Value]) -> Option<&'a Value> {
    for lang in LANGUAGES {
        if let Some(track) = tracks
            .iter()
            .find(|track| language_matches(track.get("languageCode").and_then(Value::as_str), lang))
        {
            return Some(track);
        }
    }
    tracks.first()
}

fn with_json3_format(base_url: &str) -> AppResult<String> {
    let mut url =
        Url::parse(base_url).map_err(|err| format!("Transkript-URL ist ungültig: {err}"))?;
    let pairs = url
        .query_pairs()
        .filter(|(key, _)| key != "fmt")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();

    {
        let mut query = url.query_pairs_mut();
        query.clear();
        for (key, value) in pairs {
            query.append_pair(&key, &value);
        }
        query.append_pair("fmt", "json3");
    }

    Ok(url.to_string())
}

fn language_matches(language_code: Option<&str>, wanted: &str) -> bool {
    language_code
        .map(|code| code == wanted || code.starts_with(&format!("{wanted}-")))
        .unwrap_or(false)
}

fn extract_innertube_api_key(html: &str) -> Option<String> {
    Regex::new(r#""INNERTUBE_API_KEY":\s*"([a-zA-Z0-9_-]+)""#)
        .ok()?
        .captures(html)?
        .get(1)
        .map(|value| value.as_str().to_string())
}

#[derive(Serialize)]
struct InnertubePlayerRequest<'a> {
    context: InnertubeContext,
    #[serde(rename = "videoId")]
    video_id: &'a str,
}

#[derive(Serialize)]
struct InnertubeContext {
    client: InnertubeClient,
}

#[derive(Serialize)]
struct InnertubeClient {
    #[serde(rename = "clientName")]
    client_name: &'static str,
    #[serde(rename = "clientVersion")]
    client_version: &'static str,
}

async fn fetch_innertube_player(
    client: &Client,
    video_id: &str,
    api_key: &str,
) -> AppResult<Value> {
    let request = InnertubePlayerRequest {
        context: InnertubeContext {
            client: InnertubeClient {
                client_name: "ANDROID",
                client_version: "20.10.38",
            },
        },
        video_id,
    };

    client
        .post(format!(
            "https://www.youtube.com/youtubei/v1/player?key={api_key}"
        ))
        .json(&request)
        .send()
        .await
        .map_err(|err| format!("YouTube-Innertube-Daten konnten nicht geladen werden: {err}"))?
        .error_for_status()
        .map_err(|err| format!("YouTube-Innertube-Daten konnten nicht geladen werden: {err}"))?
        .json::<Value>()
        .await
        .map_err(|err| format!("YouTube-Innertube-Daten konnten nicht gelesen werden: {err}"))
}

async fn fetch_watch_html(client: &Client, video_id: &str) -> AppResult<String> {
    client
        .get(video_url(video_id))
        .header("Accept-Language", "en")
        .send()
        .await
        .map_err(|err| format!("YouTube-Seite konnte nicht geladen werden: {err}"))?
        .error_for_status()
        .map_err(|err| format!("YouTube-Seite konnte nicht geladen werden: {err}"))?
        .text()
        .await
        .map_err(|err| format!("YouTube-Seite konnte nicht gelesen werden: {err}"))
}

fn parse_json3_transcript(payload: &Value) -> Vec<TranscriptSnippet> {
    payload
        .get("events")
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .filter_map(|event| {
                    let start = event.get("tStartMs")?.as_f64()? / 1000.0;
                    let text = event
                        .get("segs")?
                        .as_array()?
                        .iter()
                        .filter_map(|seg| seg.get("utf8").and_then(Value::as_str))
                        .collect::<String>()
                        .replace('\n', " ")
                        .trim()
                        .to_string();
                    if text.is_empty() {
                        None
                    } else {
                        Some(TranscriptSnippet {
                            text,
                            start,
                            time: format_time(start),
                        })
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_chapters_from_data(data: &Value) -> Option<Vec<Chapter>> {
    let mut chapters = Vec::new();

    if let Some(items) = data
        .pointer("/playerOverlays/playerOverlayRenderer/decoratedPlayerBarRenderer/decoratedPlayerBarRenderer/playerBar/multiMarkersPlayerBarRenderer/markersMap/0/value/chapters")
        .and_then(Value::as_array)
    {
        for item in items {
            if let Some(chapter) = parse_chapter_renderer(item) {
                chapters.push(chapter);
            }
        }
    }

    if chapters.is_empty() {
        if let Some(contents) = data
            .pointer("/engagementPanels/0/engagementPanelSectionListRenderer/content/macroMarkersListRenderer/contents")
            .and_then(Value::as_array)
        {
            for item in contents {
                if let Some(chapter) = parse_macro_marker(item) {
                    chapters.push(chapter);
                }
            }
        }
    }

    if chapters.is_empty() {
        None
    } else {
        Some(chapters)
    }
}

fn parse_chapter_renderer(item: &Value) -> Option<Chapter> {
    let renderer = item.get("chapterRenderer")?;
    let start = renderer.get("timeRangeStartMillis")?.as_f64()? / 1000.0;
    let title = renderer.pointer("/title/simpleText")?.as_str()?.to_string();
    Some(Chapter {
        time: format_time(start),
        start,
        title,
    })
}

fn parse_macro_marker(item: &Value) -> Option<Chapter> {
    let renderer = item.get("macroMarkersListItemRenderer")?;
    let start = renderer
        .pointer("/onTap/watchEndpoint/startTimeSeconds")?
        .as_f64()?;
    let title = renderer.pointer("/title/simpleText")?.as_str()?.to_string();
    Some(Chapter {
        time: format_time(start),
        start,
        title,
    })
}

fn extract_json_assignment(html: &str, var_name: &str) -> Option<Value> {
    let marker = format!("{var_name} = ");
    let start = html.find(&marker)? + marker.len();
    let start = html[start..].find('{')? + start;
    let bytes = html.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;

    for i in start..bytes.len() {
        let ch = bytes[i] as char;
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&html[start..=i]).ok();
                }
            }
            _ => {}
        }
    }

    None
}

fn format_time(seconds: f64) -> String {
    let total = seconds.floor() as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json3_format_replaces_existing_fmt() {
        let url = with_json3_format("https://www.youtube.com/api/timedtext?v=abc&fmt=srv3&lang=en")
            .expect("valid URL");

        assert!(url.contains("fmt=json3"));
        assert!(!url.contains("fmt=srv3"));
    }

    #[test]
    fn language_match_accepts_regional_variants() {
        assert!(language_matches(Some("de-DE"), "de"));
        assert!(language_matches(Some("en"), "en"));
        assert!(!language_matches(Some("pt-BR"), "de"));
    }

    #[tokio::test]
    #[ignore = "requires YouTube network access"]
    async fn fetches_transcript_from_innertube_caption_url() {
        let client = Client::builder()
            .user_agent("Mozilla/5.0 YouTubeSummarizer/0.1")
            .build()
            .expect("HTTP client");

        let transcript = fetch_transcript(&client, "dQw4w9WgXcQ")
            .await
            .expect("transcript");

        let snippets =
            serde_json::from_str::<Vec<TranscriptSnippet>>(&transcript).expect("snippet JSON");
        assert!(snippets.len() > 10);
        assert!(snippets
            .iter()
            .any(|snippet| !snippet.text.trim().is_empty()));
    }
}
