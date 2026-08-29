# YouTube Summarizer

Desktop app for collecting YouTube videos, loading transcripts and creating AI summaries.

The app is implemented with Tauri 2, a TypeScript/Vite frontend and a Rust backend.

## Current Status

- Add YouTube videos by URL or video ID.
- Store videos in a local SQLite database, organize them in collections,
  search the library and filter by transcript/summary availability.
- Load video metadata, thumbnails, transcripts and chapters where available;
  refresh missing transcripts for existing videos.
- Configure recommended, custom and local OpenAI-compatible AI providers in
  the app; browse the models.dev catalog with context/pricing badges, pick
  the summary model per run and test individual models in a chat dialog.
- Generate Markdown summaries with a configurable dialog: prompt presets
  (built-ins plus custom templates), toggleable modules (tables, Mermaid
  diagrams, AI assessment, critical claim check, timestamps) and an editable
  prompt preview.
- Summaries stream live into the UI while they are generated.
- Every run is kept in a per-video summary history with a version dropdown.
- Summaries render Mermaid diagrams, and `[mm:ss]` timestamps are clickable
  and seek the embedded player to that position.
- Transcript, metadata and chapters are passed to the model behind
  untrusted-data delimiters to harden against prompt injection.
- Dev-only local automation API for functional testing by agents.

## Tech Stack

- Frontend: TypeScript, Vite, Tauri JavaScript API
- Backend: Rust, Tauri 2
- Storage: SQLite via `rusqlite`
- HTTP: `reqwest` with Rustls TLS

## Development

Install dependencies:

```bash
npm install
```

Run the desktop app in development mode:

```bash
npm run tauri dev
```

Run the frontend build:

```bash
npm run build
```

This only builds the web frontend into `dist/`; it does not create a desktop executable.

Build the release desktop app and Linux bundles:

```bash
npm run tauri -- build
```

If `cargo` is not in the shell `PATH`, load the Rust environment first:

```bash
. "$HOME/.cargo/env"
npm run tauri -- build
```

Release outputs are written to:

```text
src-tauri/target/release/youtube-summarizer
src-tauri/target/release/bundle/deb/YouTube Summarizer_0.1.0_amd64.deb
src-tauri/target/release/bundle/rpm/YouTube Summarizer-0.1.0-1.x86_64.rpm
src-tauri/target/release/bundle/appimage/YouTube Summarizer_0.1.0_amd64.AppImage
```

Run Rust tests:

```bash
cd src-tauri
cargo test
```

Run the ignored YouTube network transcript test when network access is intended:

```bash
cd src-tauri
cargo test fetches_transcript_from_innertube_caption_url -- --ignored
```

## Automation API

In development builds the app starts a local API for automated testing. The console prints the active URL:

```text
AUTOMATION_URL=http://127.0.0.1:<port>/api
```

Available endpoints:

- `GET /api/health`
- `GET /api/videos`
- `GET /api/video/{id}`
- `POST /api/add-video` with `{"url":"..."}`
- `POST /api/transcript/{id}`
- `POST /api/summarize/{id}` with `{"system_prompt":"...", "provider_id":"...", "model_id":"...", "timestamps":false, "options":"..."}` (all fields optional)
- `DELETE /api/video/{id}`

The API is debug-only and binds to `127.0.0.1`.

## Project Notes

- See `TODO.md` for the current working state and next tasks.
- See `AGENTS.md` for instructions aimed at AI coding agents.
