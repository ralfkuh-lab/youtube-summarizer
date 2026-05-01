# YouTube Summarizer – Projektplan

## Ziel
Desktop-App, die YouTube-Video-Transkripte extrahiert und per KI zusammenfasst.

## Tech-Stack

| Bereich          | Wahl                      | Begründung                                    |
|------------------|---------------------------|-----------------------------------------------|
| Desktop-Rahmen   | PySide6 + QWebEngineView  | Native Fenster, Chromium-Webview, pip-install |
| Backend          | Python 3.9+               | Cross-platform, reichhaltiges Ökosystem       |
| DB               | SQLite via SQLAlchemy     | Dateibasiert, portabel, keine Installation    |
| Frontend (UI)    | HTML/CSS/JS (Vanilla)     | Lädt via lokalem HTTP-Server (Origin: http://127.0.0.1) |
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
├── main.py                  # Einstieg: QApplication, lokaler HTTP-Server, QWebEngineView
├── start.sh / start.bat     # Startscript mit Abhängigkeits-Check und Auto-Installation
├── requirements.txt          # PySide6, youtube-transcript-api, SQLAlchemy, httpx
├── config.example.json
├── config.json              # Nutzer-Konfiguration (gitignored)
├── .gitignore
├── claude.md
├── app/
│   ├── __init__.py
│   ├── config.py            # JSON-Config-Loader mit Auto-Endpoints
│   ├── database.py          # SQLAlchemy-Setup + automatische Migration
│   ├── models.py            # Video-ORM-Modell (inkl. Thumbnail-BLOB, Chapters)
│   ├── youtube.py           # URL-Parsing, oEmbed, Thumbnail-Download, Transkript (JSON), Kapitel
│   ├── ai_client.py         # KI-Client (konfigurierbarer System-Prompt)
│   ├── bridge.py            # QWebChannel-Bridge (Threading für Transkript+KI)
│   └── www/
│       ├── index.html       # SPA: Sidebar + Detail + Tabs + Kapitel-Panel + Modals
│       ├── style.css        # Dark Theme + Markdown-Styles + Kapitel-Panel
│       └── app.js           # QWebChannel-Integration, Markdown-Renderer, Video-Player

## Datenmodell (videos-Tabelle)

| Feld           | Typ              | Beschreibung                                    |
|----------------|------------------|-------------------------------------------------|
| id             | INTEGER PK       | Auto-ID                                         |
| video_id       | TEXT UNIQUE      | YouTube Video-ID                                |
| url            | TEXT             | Komplette YouTube-URL                           |
| title          | TEXT             | Video-Titel (via oEmbed)                        |
| thumbnail_url  | TEXT             | Vorschaubild-URL (Referenz)                     |
| thumbnail_data | BLOB (nullable)  | Vorschaubild als JPEG (offline-fähig)           |
| transcript     | TEXT (nullable)  | Transkript als JSON [{text, start, time}, ...]  |
| chapters       | TEXT (nullable)  | Kapitel als JSON [{time, start, title}, ...]    |
| summary        | TEXT (nullable)  | KI-Zusammenfassung (Markdown)                   |
| created_at     | DATETIME         | Erstellungsdatum                                |
| updated_at     | DATETIME         | Letzte Änderung                                 |

## UI-Layout

- **Header**: App-Titel + Settings-Button (⚙)
- **Add-Bar**: YouTube-URL-Eingabe + Hinzufügen-Button
- **Main-Bereich**:
  - Linke Sidebar (320px): Video-Liste mit Thumbnails (Base64), Titeln, Status-Indikatoren (T/Z)
  - Rechter Detail-Bereich (flex):
    - Detail-Header: Thumbnail, Titel, URL-Link, Löschen-Button
    - Tab-Bar: Transkript | Zusammenfassung | Video + Zusammenfassen-Button
    - Tab-Inhalt: Transkript (Zeitstempel), Zusammenfassung (Markdown), Video (iframe)
    - Kapitel-Panel (rechts, 220px): Bei vorhandenen Kapiteln, klickbar zum Video-Sprung
- **Statusleiste**: Aktueller Status + Provider/Modell-Info
- **Modals**: KI-Einstellungen, Zusammenfassungs-Dialog (Detailgrad, Sprache, Prompt-Editor)

## Kommunikation Python ↔ JavaScript

QWebChannel über lokalen HTTP-Server (Origin: http://127.0.0.1:{port}):

JS → Python (Slots):
- bridge.add_video(url)
- bridge.summarize_video(id, system_prompt)
- bridge.delete_video(id)
- bridge.get_videos(), bridge.get_video_detail(id)
- bridge.get_config(), bridge.save_config(provider, api_key, model)

Python → JS (Signale):
- videos_loaded, video_detail_loaded, video_added, video_deleted
- transcript_loaded, chapters_loaded, summary_loaded
- config_loaded, status_update, error

## Konfiguration (config.json)

```json
{
  "ai": {
    "provider": "opencode_go",
    "api_key": "sk-...",
    "model": "qwen3.5-plus",
    "endpoint_override": null
  }
}
```

`endpoint_override` für manuelle Provider. Bei null wird der Endpoint automatisch aus `DEFAULT_ENDPOINTS` gewählt.

## Features

- Video hinzufügen via YouTube-URL (Unterstützung für watch, youtu.be, Shorts, Embed)
- Transkript-Extraktion mit Zeitstempeln (JSON-Format, mehrsprachig)
- Thumbnail-Download und lokale Speicherung als BLOB (offline-fähig)
- Video-Kapitel-Extraktion aus YouTube Player Response
- Kapitel-Panel mit Klick-zum-Springen im Video
- Kapitel-Marker inline im Transkript
- Video-Player eingebettet via youtube-nocookie.com
- KI-Zusammenfassung mit konfigurierbarem Prompt
  - Detailgrad: Kurz / Mittel / Ausführlich
  - Sprache: Original / Deutsch / Englisch / Französisch / Spanisch / Italienisch
  - Optionale Kapitel-Gliederung
  - Freie Prompt-Bearbeitung
- Markdown-Rendering für Zusammenfassungen
- SQLite-Datenbank mit automatischer Schema-Migration
- Plattform-Autoerkennung (Wayland > XCB > Offscreen)
- Startscript mit Abhängigkeits-Check und interaktiver Installation

## Start (Nutzerperspektive)

```bash
./start.sh
# Prüft Abhängigkeiten (Python, System-Pakete, Python-Pakete)
# Bietet Installation an, kopiert config.example.json → config.json
# Startet die App (lokaler HTTP-Server + Qt-WebView)
```

## Modell-Empfehlungen für Zusammenfassungen

- OpenCode Go: qwen3.5-plus (günstig), minimax-m2.5 (free), kimi-k2.6 (stark)
- OpenCode Zen: big-pickle (free), minimax-m2.5-free (free), glm-5 (günstig)
- OpenRouter: beliebige Modelle
- Ollama: lokale Modelle (llama3, mistral, etc.)
