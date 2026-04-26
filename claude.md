# YouTube Summarizer – Projektplan

## Ziel
Desktop-App, die YouTube-Video-Transkripte extrahiert und per KI zusammenfasst.

## Tech-Stack

| Bereich          | Wahl                      | Begründung                                    |
|------------------|---------------------------|-----------------------------------------------|
| Desktop-Rahmen   | PySide6 + QWebEngineView  | Native Fenster, Chromium-Webview, pip-install |
| Backend          | Python 3.9+               | Cross-platform, reichhaltiges Ökosystem       |
| DB               | SQLite via SQLAlchemy     | Dateibasiert, portabel, keine Installation    |
| Frontend (UI)    | HTML/CSS/JS (Vanilla)     | Läuft in QWebEngineView, via QWebChannel      |
| Youtube          | youtube-transcript-api + oEmbed | Transkripte ohne API-Key, Metadaten via oEmbed |
| KI-Client        | httpx + OpenAI-Format     | Alle Provider verwenden /v1/chat/completions  |

## KI-Provider (alle OpenAI-kompatibel)

| Provider       | Endpoint                                          | Auth          |
|----------------|---------------------------------------------------|---------------|
| OpenCode Zen   | https://opencode.ai/zen/v1/chat/completions       | Bearer api_key|
| OpenCode Go    | https://opencode.ai/zen/go/v1/chat/completions    | Bearer api_key|
| OpenRouter     | https://openrouter.ai/api/v1/chat/completions     | Bearer api_key|
| Ollama         | http://localhost:11434/v1/chat/completions        | Optional      |

## Ordnerstruktur

youtube-summarizer/
├── main.py                  # QApplication + QMainWindow + QWebEngineView
├── requirements.txt
├── config.example.json
├── config.json              # Nutzer-Konfiguration (gitignored)
├── app/
│   ├── __init__.py
│   ├── config.py            # JSON-Config-Loader
│   ├── database.py          # SQLAlchemy-Setup
│   ├── models.py            # Video-ORM-Modell
│   ├── youtube.py           # Metadaten + Transkript
│   ├── ai_client.py         # Einheitlicher KI-Client
│   ├── bridge.py            # QWebChannel-Bridge (Python↔JS)
│   └── www/
│       ├── index.html       # SPA: Video-Liste + Detailansicht
│       ├── style.css        # Styling (Dark Theme)
│       └── app.js           # Frontend-Logik + QWebChannel-Integration

## Datenmodell (videos-Tabelle)

| Feld          | Typ              | Beschreibung                    |
|---------------|------------------|---------------------------------|
| id            | INTEGER PK       | Auto-ID                         |
| video_id      | TEXT UNIQUE      | YouTube Video-ID                |
| url           | TEXT             | Komplette YouTube-URL           |
| title         | TEXT             | Video-Titel                     |
| thumbnail_url | TEXT             | Vorschaubild-URL                |
| transcript    | TEXT (nullable)  | Rohtranskript                   |
| summary       | TEXT (nullable)  | KI-Zusammenfassung              |
| created_at    | DATETIME         | Erstellungsdatum                |
| updated_at    | DATETIME         | Letzte Änderung                 |

## UI-Layout

- Linke Sidebar (30%): Video-Liste mit Thumbnails + Titeln
- Rechter Bereich (70%): Detailansicht mit Tabs (Transkript / Zusammenfassung)
- Oben: Eingabefeld für YouTube-URL + Add-Button
- Unten: Statusleiste (Provider, Modell)

## Kommunikation Python ↔ JavaScript

QWebChannel:
- JS → Python: bridge.add_video(url), bridge.summarize(video_id), bridge.delete_video(id)
- Python → JS: transcript_ready, summary_ready, error, video_added

## Konfiguration (config.json)

{
  "ai": {
    "provider": "opencode_go",
    "api_key": "sk-...",
    "model": "qwen3.5-plus",
    "endpoint_override": null
  }
}

## Start (Nutzerperspektive)

./start.sh
# Prüft Abhängigkeiten, bietet Installation an, startet die App

## Modell-Empfehlungen für Zusammenfassungen

- OpenCode Go: qwen3.5-plus (günstig), minimax-m2.5 (free), kimi-k2.6 (stark)
- OpenCode Zen: big-pickle (free), minimax-m2.5-free (free), glm-5 (günstig)
- OpenRouter: beliebige Modelle
- Ollama: lokale Modelle (llama3, mistral, etc.)
