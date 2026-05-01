# Working Notes

This is the shared working file for project state, TODOs and handoff notes between sessions.

## Project Goal

Build a cross-platform desktop app for Linux, Windows and macOS that can collect YouTube videos, load transcripts and create AI summaries.

## Current Direction

The app is being rewritten from Python/PySide to Tauri 2 with a TypeScript frontend and Rust backend. The old Python implementation remains in `legacy-python/` until the new app reaches the desired functional level.

## Done

- Created the Tauri 2 project structure.
- Moved the previous Python implementation to `legacy-python/`.
- Implemented local SQLite storage for videos and AI settings.
- Implemented video add/delete/list/detail flows.
- Implemented YouTube metadata, thumbnail, transcript and chapter loading.
- Switched transcript loading to YouTube Innertube player data for better reliability.
- Added manual transcript refresh for existing videos without transcripts.
- Implemented AI summary generation through OpenAI-compatible chat completion endpoints.
- Added a dev-only local automation API for agent-driven functional testing.
- Added the full Tauri icon set required for Linux AppImage bundling.
- Verified:
  - `npm run build`
  - `cargo test`
  - `npm run tauri -- build`
  - Live automation flow for health, transcript loading, summarization and cleanup.

## Next TODOs

- Improve frontend polish and interaction states.
- Add better empty/error states for transcript and summary failures.
- Decide whether the video list should include compact status icons instead of plain `T`/`Z` markers.
- Add Windows and macOS packaging notes once tested on those platforms.
- Add release checklist once app behavior stabilizes.
- Review whether automation API responses should return compact video objects to avoid huge payloads from thumbnails/transcripts.
- Decide when `legacy-python/` can be deleted.

## Known Notes

- The automation API is only available in debug builds and prints its URL as `AUTOMATION_URL=http://127.0.0.1:<port>/api`.
- Existing videos that were added before the transcript fix may need the `Transkript laden` button or `POST /api/transcript/{id}`.
- The ignored Rust test `fetches_transcript_from_innertube_caption_url` uses live YouTube network access.
- OpenCode Go settings are stored in the app configuration, not in this repository.

## Last Verified State

- Date: 2026-05-01
- Build: `npm run build` passed.
- Release build: `npm run tauri -- build` passed and produced Linux binary, deb, rpm and AppImage artifacts.
- Rust tests: `cargo test` passed with 2 tests passed and 1 network test ignored.
- Functional test: summary generation worked with stored OpenCode Go settings.
