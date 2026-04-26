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


def fetch_transcript(video_id: str) -> str:
    api = YouTubeTranscriptApi()
    transcript = api.fetch(video_id, languages=["de", "en", "fr", "es", "it", "nl", "pl", "ru", "ja", "ko", "pt", "ar", "tr"])
    lines = [snippet.text for snippet in transcript]
    return "\n".join(lines)
