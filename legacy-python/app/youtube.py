import re
import urllib.request
import json
from youtube_transcript_api import YouTubeTranscriptApi


def extract_video_id(url: str) -> str | None:
    patterns = [
        r"(?:v=|/v/|youtu\.be/|/embed/|/shorts/)([A-Za-z0-9_-]{11})",
        r"^([A-Za-z0-9_-]{11})$",
    ]
    for pattern in patterns:
        match = re.search(pattern, url)
        if match:
            return match.group(1)
    return None


def get_video_url(video_id: str) -> str:
    return f"https://www.youtube.com/watch?v={video_id}"


def get_thumbnail_url(video_id: str) -> str:
    return f"https://i.ytimg.com/vi/{video_id}/hqdefault.jpg"


def fetch_video_info(video_id: str) -> dict:
    url = get_video_url(video_id)
    oembed_url = f"https://www.youtube.com/oembed?url={url}&format=json"
    with urllib.request.urlopen(oembed_url, timeout=10) as resp:
        data = json.loads(resp.read())
    return {
        "title": data.get("title", video_id),
        "thumbnail_url": get_thumbnail_url(video_id),
    }


def download_thumbnail(video_id: str) -> bytes:
    url = get_thumbnail_url(video_id)
    with urllib.request.urlopen(url, timeout=15) as resp:
        return resp.read()


def _format_time(seconds: float) -> str:
    h = int(seconds // 3600)
    m = int((seconds % 3600) // 60)
    s = int(seconds % 60)
    if h > 0:
        return f"{h}:{m:02d}:{s:02d}"
    return f"{m}:{s:02d}"


def fetch_transcript(video_id: str) -> str:
    api = YouTubeTranscriptApi()
    transcript = api.fetch(video_id, languages=["de", "en", "fr", "es", "it", "nl", "pl", "ru", "ja", "ko", "pt", "ar", "tr"])
    snippets = []
    for snippet in transcript:
        snippets.append({
            "text": snippet.text,
            "start": snippet.start,
            "time": _format_time(snippet.start),
        })
    return json.dumps(snippets, ensure_ascii=False)


def transcript_to_text(transcript_json: str | None) -> str:
    if not transcript_json:
        return ""
    try:
        snippets = json.loads(transcript_json)
    except (json.JSONDecodeError, TypeError):
        return transcript_json
    return "\n".join(s["text"] for s in snippets)


def _extract_json_from_html(html: str, var_name: str) -> dict | None:
    match = re.search(rf'{re.escape(var_name)}\s*=\s*\{{', html)
    if not match:
        return None

    start = match.end() - 1
    depth = 0
    end = start
    for i in range(start, len(html)):
        if html[i] == '{':
            depth += 1
        elif html[i] == '}':
            depth -= 1
            if depth == 0:
                end = i + 1
                break
    if depth != 0:
        return None

    try:
        return json.loads(html[start:end])
    except json.JSONDecodeError:
        return None


def _extract_chapters_from_data(data: dict) -> list | None:
    chapters_raw = None

    try:
        chapters_raw = data["playerOverlays"]["playerOverlayRenderer"]["decoratedPlayerBarRenderer"]["decoratedPlayerBarRenderer"]["playerBar"]["multiMarkersPlayerBarRenderer"]["markersMap"][0]["value"]["chapters"]
    except (KeyError, IndexError, TypeError):
        pass

    if not chapters_raw:
        try:
            chapters_raw = data["engagementPanels"][0]["engagementPanelSectionListRenderer"]["content"]["macroMarkersListRenderer"]["contents"]
        except (KeyError, IndexError, TypeError):
            pass

    if not chapters_raw:
        return None

    chapters = []
    for item in chapters_raw:
        try:
            c = item["macroMarkersListItemRenderer"]["onTap"]["watchEndpoint"]["startTimeSeconds"]
            title = item["macroMarkersListItemRenderer"]["title"]["simpleText"]
            chapters.append({"time": _format_time(int(c)), "start": int(c), "title": title})
        except (KeyError, TypeError):
            try:
                c = item["chapterRenderer"]
                chapters.append({
                    "time": _format_time(c["timeRangeStartMillis"] / 1000),
                    "start": c["timeRangeStartMillis"] / 1000,
                    "title": c["title"]["simpleText"],
                })
            except (KeyError, TypeError):
                continue

    return chapters if chapters else None


def fetch_chapters(video_id: str) -> str | None:
    url = get_video_url(video_id)
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0", "Accept-Language": "en"})
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            html = resp.read().decode("utf-8", errors="replace")
    except Exception:
        return None

    for var_name in ("ytInitialData", "ytInitialPlayerResponse"):
        data = _extract_json_from_html(html, var_name)
        if data is None:
            continue
        chapters = _extract_chapters_from_data(data)
        if chapters:
            return json.dumps(chapters, ensure_ascii=False)

    return None
