# YouTube Summarizer

Desktop app for collecting YouTube videos, loading transcripts and creating AI summaries.

The current implementation is a Tauri 2 app with a TypeScript/Vite frontend and a Rust backend. The previous Python version is kept in `legacy-python/` for reference until the new app has fully reached feature parity.

## Current Status

- Add YouTube videos by URL or video ID.
- Store videos in a local SQLite database.
- Load video metadata, thumbnails, transcripts and chapters where available.
- Refresh missing transcripts for existing videos.
- Configure AI provider, API key, model and optional endpoint override in the app.
- Generate Markdown summaries from transcripts.
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
- `POST /api/summarize/{id}` with `{"system_prompt":"..."}`
- `DELETE /api/video/{id}`

The API is debug-only and binds to `127.0.0.1`.

## Project Notes

- See `TODO.md` for the current working state and next tasks.
- See `AGENTS.md` for instructions aimed at AI coding agents.
- Do not remove `legacy-python/` yet; it is the reference implementation until the Tauri app is complete.
