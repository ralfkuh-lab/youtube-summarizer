use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

use crate::models::{Chapter, TranscriptSnippet, VideoInfo};
use crate::storage::AppResult;

const LANGUAGES: &[&str] = &[
    "de", "en", "fr", "es", "it", "nl", "pl", "ru", "ja", "ko", "pt", "ar", "tr",
];

#[derive(Debug, Deserialize)]
struct OEmbedResponse {
    title: Option<String>,
}

fn video_id_patterns() -> &'static [Regex; 2] {
    static PATTERNS: OnceLock<[Regex; 2]> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            Regex::new(r"(?:v=|/v/|youtu\.be/|/embed/|/shorts/)([A-Za-z0-9_-]{11})").unwrap(),
            Regex::new(r"^([A-Za-z0-9_-]{11})$").unwrap(),
        ]
    })
}

pub fn extract_video_id(input: &str) -> Option<String> {
    video_id_patterns().iter().find_map(|re| {
        re.captures(input)
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

/// Loads oembed metadata (title + thumbnail). The publish date is parsed
/// separately from the shared watch HTML by the caller (see A5), so this call
/// only performs the oembed request and can run in parallel with the HTML fetch.
pub async fn fetch_video_info(client: &Client, video_id: &str) -> AppResult<VideoInfo> {
    let url = video_url(video_id);
    let oembed_url = format!("https://www.youtube.com/oembed?url={url}&format=json");
    let data = client
        .get(oembed_url)
        .send()
        .await
        .map_err(|err| format!("YouTube-Metadaten konnten nicht geladen werden: {err}"))?
        .error_for_status()
        .map_err(|err| format!("YouTube-Metadaten konnten nicht geladen werden: {err}"))?
        .json::<OEmbedResponse>()
        .await
        .map_err(|err| format!("YouTube-Metadaten konnten nicht gelesen werden: {err}"))?;

    Ok(VideoInfo {
        title: data.title.unwrap_or_else(|| video_id.to_string()),
        thumbnail_url: thumbnail_url(video_id),
        published_at: None,
    })
}

pub fn publish_date_from_html(html: &str) -> Option<String> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(r#""publishDate"\s*:\s*"([0-9]{4}-[0-9]{2}-[0-9]{2})"#).unwrap()
    });
    pattern
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

pub async fn download_thumbnail(client: &Client, video_id: &str) -> Option<Vec<u8>> {
    let response = client.get(thumbnail_url(video_id)).send().await.ok()?;
    let response = response.error_for_status().ok()?;
    response.bytes().await.ok().map(|bytes| bytes.to_vec())
}

pub async fn fetch_transcript(client: &Client, video_id: &str) -> AppResult<String> {
    let player = fetch_innertube_player(client, video_id).await?;

    check_playability(&player)?;

    let tracks = player
        .pointer("/captions/playerCaptionsTracklistRenderer/captionTracks")
        .and_then(Value::as_array)
        .filter(|tracks| !tracks.is_empty())
        .ok_or_else(|| "Für dieses Video wurde kein Transkript gefunden".to_string())?;

    let track = select_caption_track(tracks)
        .ok_or_else(|| "Kein unterstütztes Transkript gefunden".to_string())?;
    let base_url = track
        .get("baseUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| "Transkript-URL fehlt".to_string())?;

    let transcript_url = with_json3_format(base_url);
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

pub fn chapters_from_html(html: &str) -> Option<String> {
    for var_name in ["ytInitialData", "ytInitialPlayerResponse"] {
        if let Some(data) = extract_json_assignment(html, var_name) {
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

fn is_asr(track: &Value) -> bool {
    track.get("kind").and_then(Value::as_str) == Some("asr")
}

fn select_caption_track(tracks: &[Value]) -> Option<&Value> {
    for lang in LANGUAGES {
        let same_language = |track: &&Value| {
            language_matches(track.get("languageCode").and_then(Value::as_str), lang)
        };
        if let Some(track) = tracks
            .iter()
            .find(|track| same_language(track) && !is_asr(track))
        {
            return Some(track);
        }
        if let Some(track) = tracks
            .iter()
            .find(|track| same_language(track) && is_asr(track))
        {
            return Some(track);
        }
    }

    tracks
        .iter()
        .find(|track| !is_asr(track))
        .or_else(|| tracks.first())
}

/// Rejects videos that YouTube marks as not playable, using the honest status
/// (and reason) from the player response before we look for caption tracks.
fn check_playability(player: &Value) -> Result<(), String> {
    let Some(status) = player
        .pointer("/playabilityStatus/status")
        .and_then(Value::as_str)
    else {
        return Ok(());
    };
    if status == "OK" {
        return Ok(());
    }
    let reason = player
        .pointer("/playabilityStatus/reason")
        .and_then(Value::as_str);
    Err(match reason {
        Some(reason) => format!("Video nicht abrufbar ({status}): {reason}"),
        None => format!("Video nicht abrufbar ({status})"),
    })
}

/// Ensures the caption URL requests `fmt=json3` while keeping the existing
/// (signed) query byte-identical. The `baseUrl` from `captionTracks` carries
/// `sig`/`sparams`/`pot`; re-encoding it via `query_pairs_mut` could alter the
/// percent-encoding and break the signature, so we only replace the `fmt`
/// segment in place (or append it) and leave every other byte — including empty
/// segments and any trailing fragment — untouched.
fn with_json3_format(base_url: &str) -> String {
    // Split off a fragment first; fmt=json3 belongs in the query, before it.
    let (without_fragment, fragment) = match base_url.split_once('#') {
        Some((head, frag)) => (head, Some(frag)),
        None => (base_url, None),
    };

    let (base, query) = match without_fragment.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (without_fragment, None),
    };

    let new_query = match query {
        None => "fmt=json3".to_string(),
        Some(query) => {
            let mut replaced = false;
            let mut segments: Vec<&str> = query
                .split('&')
                .map(|segment| {
                    if segment == "fmt" || segment.starts_with("fmt=") {
                        replaced = true;
                        "fmt=json3"
                    } else {
                        segment
                    }
                })
                .collect();
            if !replaced {
                segments.push("fmt=json3");
            }
            segments.join("&")
        }
    };

    match fragment {
        Some(fragment) => format!("{base}?{new_query}#{fragment}"),
        None => format!("{base}?{new_query}"),
    }
}

fn language_matches(language_code: Option<&str>, wanted: &str) -> bool {
    language_code
        .map(|code| code == wanted || code.starts_with(&format!("{wanted}-")))
        .unwrap_or(false)
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

async fn fetch_innertube_player(client: &Client, video_id: &str) -> AppResult<Value> {
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
        .post("https://www.youtube.com/youtubei/v1/player")
        .header(
            "User-Agent",
            "com.google.android.youtube/20.10.38 (Linux; U; Android 14) gzip",
        )
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

pub async fn fetch_watch_html(client: &Client, video_id: &str) -> AppResult<String> {
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
    fn json3_format_replaces_existing_fmt_in_place() {
        // The fmt segment keeps its position in the query; nothing is reordered.
        let url = with_json3_format("https://www.youtube.com/api/timedtext?v=abc&fmt=srv3&lang=en");
        assert_eq!(
            url,
            "https://www.youtube.com/api/timedtext?v=abc&fmt=json3&lang=en"
        );
    }

    #[test]
    fn json3_format_appends_when_query_empty() {
        let url = with_json3_format("https://www.youtube.com/api/timedtext");
        assert_eq!(url, "https://www.youtube.com/api/timedtext?fmt=json3");
    }

    #[test]
    fn json3_format_preserves_percent_encoding() {
        // A signed URL must survive byte-identical except for the fmt segment;
        // re-encoding sparams=ip%2Cipbits would break the signature.
        let url = with_json3_format(
            "https://www.youtube.com/api/timedtext?v=abc&sparams=ip%2Cipbits&sig=XYZ",
        );
        assert_eq!(
            url,
            "https://www.youtube.com/api/timedtext?v=abc&sparams=ip%2Cipbits&sig=XYZ&fmt=json3"
        );
    }

    #[test]
    fn json3_format_keeps_empty_segments() {
        // Empty segments (a&&b, trailing &) must not be normalized away.
        let url = with_json3_format("https://www.youtube.com/api/timedtext?a&&b&");
        assert_eq!(url, "https://www.youtube.com/api/timedtext?a&&b&&fmt=json3");
    }

    #[test]
    fn json3_format_places_fmt_before_fragment() {
        let url = with_json3_format("https://www.youtube.com/api/timedtext?v=abc#frag");
        assert_eq!(
            url,
            "https://www.youtube.com/api/timedtext?v=abc&fmt=json3#frag"
        );
    }

    #[test]
    fn caption_track_prefers_manual_over_asr_same_language() {
        let tracks = serde_json::json!([
            {"languageCode": "en", "kind": "asr", "baseUrl": "en-asr"},
            {"languageCode": "en", "baseUrl": "en-manual"},
        ]);
        let track = select_caption_track(tracks.as_array().unwrap()).expect("track");
        assert_eq!(
            track.get("baseUrl").and_then(Value::as_str),
            Some("en-manual")
        );
    }

    #[test]
    fn caption_track_language_priority_beats_manual_over_asr() {
        // de is higher priority than en, so de-ASR wins over en-manual.
        let tracks = serde_json::json!([
            {"languageCode": "en", "baseUrl": "en-manual"},
            {"languageCode": "de", "kind": "asr", "baseUrl": "de-asr"},
        ]);
        let track = select_caption_track(tracks.as_array().unwrap()).expect("track");
        assert_eq!(track.get("baseUrl").and_then(Value::as_str), Some("de-asr"));
    }

    #[test]
    fn caption_track_fallback_without_language_prefers_manual() {
        // No language in LANGUAGES matches; the manual track wins over ASR.
        let tracks = serde_json::json!([
            {"languageCode": "zh", "kind": "asr", "baseUrl": "zh-asr"},
            {"languageCode": "zh", "baseUrl": "zh-manual"},
        ]);
        let track = select_caption_track(tracks.as_array().unwrap()).expect("track");
        assert_eq!(
            track.get("baseUrl").and_then(Value::as_str),
            Some("zh-manual")
        );
    }

    #[test]
    fn playability_ok_passes() {
        let player = serde_json::json!({"playabilityStatus": {"status": "OK"}});
        assert!(check_playability(&player).is_ok());
    }

    #[test]
    fn playability_missing_status_passes() {
        let player = serde_json::json!({"videoDetails": {}});
        assert!(check_playability(&player).is_ok());
    }

    #[test]
    fn playability_error_with_reason() {
        let player = serde_json::json!({
            "playabilityStatus": {"status": "LOGIN_REQUIRED", "reason": "Melde dich an"}
        });
        assert_eq!(
            check_playability(&player),
            Err("Video nicht abrufbar (LOGIN_REQUIRED): Melde dich an".to_string())
        );
    }

    #[test]
    fn playability_error_without_reason() {
        let player = serde_json::json!({"playabilityStatus": {"status": "ERROR"}});
        assert_eq!(
            check_playability(&player),
            Err("Video nicht abrufbar (ERROR)".to_string())
        );
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
