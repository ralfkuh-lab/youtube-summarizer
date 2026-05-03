# Working Notes

This is the shared working file for project state, TODOs and handoff notes between sessions.

## Project Goal

Build a cross-platform desktop app for Linux, Windows and macOS that can collect YouTube videos, load transcripts and create AI summaries.

## Current Direction

The app is now focused on the Tauri 2 implementation with a TypeScript frontend and Rust backend. The previous Python/PySide implementation has been removed.

## Done

- Created the Tauri 2 project structure.
- Moved the previous Python implementation aside during the rewrite.
- Removed the old Python implementation after confirming it is no longer needed.
- Implemented local SQLite storage for videos and AI settings.
- Implemented video add/delete/list/detail flows.
- Implemented YouTube metadata, thumbnail, transcript and chapter loading.
- Switched transcript loading to YouTube Innertube player data for better reliability.
- Added manual transcript refresh for existing videos without transcripts.
- Implemented AI summary generation through OpenAI-compatible chat completion endpoints.
- Added a dev-only local automation API for agent-driven functional testing.
- Added the full Tauri icon set required for Linux AppImage bundling.
- Adjusted the Video tab YouTube embed for Error 153 by sending an explicit referrer policy, adding player origin when available and keeping a direct YouTube fallback link.
- Reworked the frontend shell layout with the URL input in the top toolbar, a stable video-list sidebar, an optional chapter inspector and a Video tab that fits the player into the available space without using the normal content scrollbar.
- Replaced the basic AI settings form with a provider-focused configuration area, per-provider saved settings, cached model lists, automatic model refresh commands and a searchable model picker.
- Split AI settings into separate provider configuration and global model selection areas. The model list now spans all providers, supports name/provider/tag search and has a "free only" filter that includes all Ollama Cloud models because of the free usage allowance.
- Added OpenRouter as a recommended provider, support for multiple custom OpenAI-compatible providers, fixed settings headers/forms with only the model lists scrolling, removed heuristic `Low cost`/`Fast` model tags, and removed the six-model preview limit in provider details.
- Recommended provider order is Ollama Cloud, OpenRouter, OpenCode Zen, OpenCode Go.
- Added custom-provider deletion using the existing trash-button style used by video deletion.
- Removed the built-in default custom provider and replaced the small custom-provider plus button with an add-card at the end of the Custom/local list.
- Treat Ollama local as a user-added custom/local provider instead of a default provider; it can be added via an add-card and deleted like other custom providers.
- Recommended provider cards now include provider homepage links, and the model selection view shows the selected model in a fixed panel above the model list.
- Added per-provider enabled toggles. The model selection only includes models from enabled and configured providers, and the local config's old `ollama` provider entry was removed so Ollama local only appears after explicit add.
- Added model-refresh based provider status, optional model-specific chat tests, provider-nav status dots, custom/local API-key-required settings and an API-key reveal toggle in the provider settings.
- Kept the provider navigation stable across provider and All Models views, moved provider enable toggles into the provider cards and made provider-local model lists selectable.
- Replaced the provider-level chat test with per-model Test chat actions and a small prompt/response dialog.
- Turned the model test dialog into a small multi-turn chat without explicit response token limits.
- Unified the AI settings layout: provider form uses a two-column field-row grid, the global model list reuses the per-provider `.settings-model-row` structure inside the shared scrollable preview container, model selection updates buttons in place to preserve scroll position, and the chat test dialog now opens with "Hi" prefilled and selected.
- Render the provider model-refresh timestamp as a relative time (e.g. "vor 3 Tagen") with the absolute date as tooltip.
- Added an Ollama Cloud Plan (Free/Pro/Max) selector with selective probing: probe only on Free tier and only for models without a stored availability, plus a manual "Re-probe availability" button. Pro/Max suppresses Free / Subscription tags.
- Pulled the AI provider config out of the app's general modules: backend now lives under `src-tauri/src/ai_config/` (types, client, store), and the frontend UI moved to `src/ai-config.ts` with shared helpers in `src/dom-utils.ts`.
- Added sidebar video search plus transcript/summary availability filters with compact status chips.
- Replaced the Tauri app icon set with a generated video/transcript/sparkle icon that includes a light outer rim for dark taskbars.
- Verified:
  - `npm run build`
  - `cargo test`
  - `npm run tauri -- build`
  - Live automation flow for health, transcript loading, summarization and cleanup.

## Next TODOs

- Next app features:
  - Collections or playlists.
  - Import/export.
  - Batch summarization.
  - Refresh metadata/transcripts.
- Improve frontend polish and interaction states.
- Add better empty/error states for transcript and summary failures.
- Follow-up cleanup from the AI/provider settings changes:
  - Replace emoji trash buttons with a consistent icon approach when the frontend icon strategy is decided.
  - Consider splitting future broad UI commits more narrowly when they touch independent areas such as dependencies, link handling, Markdown rendering and settings UX.
- Add richer provider metadata such as pricing links, context limits and preferred summarization models.
- Add Windows and macOS packaging notes once tested on those platforms.
- Add release checklist once app behavior stabilizes.
- Review whether automation API responses should return compact video objects to avoid huge payloads from thumbnails/transcripts.
- Backlog: AI provider config reuse/refactor beyond this app. Current in-app separation is enough for now; only revisit the reusable backend crate/framework-agnostic component idea when there is a concrete second consumer. Details in [`docs/ai-config-refactor.md`](docs/ai-config-refactor.md).

## Known Notes

- The automation API is only available in debug builds and prints its URL as `AUTOMATION_URL=http://127.0.0.1:<port>/api`.
- Existing videos that were added before the transcript fix may need the `Transkript laden` button or `POST /api/transcript/{id}`.
- The ignored Rust test `fetches_transcript_from_innertube_caption_url` uses live YouTube network access.
- OpenCode Go settings are stored in the app configuration, not in this repository.

## Last Verified State

- Date: 2026-05-03
- Release build: `npm run tauri -- build` passed after replacing the app icon assets and produced Linux deb/rpm/AppImage bundles.
- Docs: README, AGENTS and TODO updated after removing the Python legacy implementation and checking the next TODO order.
- Build: `npm run build` passed after adding sidebar search/filter controls, removing the old Python implementation and updating docs/TODOs.
- Previous Rust tests: `cargo test` passed with 2 tests passed and 1 network test ignored.
- Node.js: development/build requires Node >=20 because of the current frontend dependency set; installed Tauri app does not require Node at runtime.
- Previous format check: `cargo fmt --check` passed.
- Automation API check: `GET /api/health`, `GET /api/providers`, `GET /api/config`, `POST /api/models/opencode_go` and `POST /api/models/opencode_zen` passed while the Tauri dev app was running.
- Checked OpenCode Zen chat completions with the saved key: paid model `kimi-k2.6` returns `CreditsError` for insufficient balance, while free model `minimax-m2.5-free` succeeds. App error handling now reports the billing issue instead of saying the API key is invalid.
- Automation API check for the new model selection data: `GET /api/config` confirmed shared OpenCode Go/Zen API keys, and `POST /api/models/ollama_cloud` refreshed 39 Ollama Cloud models with all 39 marked free.
- Local Ollama model refresh now filters out `:cloud` models so "Ollama local" only lists truly local models; automation check returned `gemma4:26b` and `gemma4:e4b` with no cloud entries.
- Previous release build: `npm run tauri -- build` passed on 2026-05-01 and produced Linux binary, deb, rpm and AppImage artifacts.
- Previous functional test: summary generation worked with stored OpenCode Go settings on 2026-05-01.
