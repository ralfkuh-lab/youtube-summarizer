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
- Fixed the installed release app's YouTube Error 153 by serving production assets through Tauri's localhost plugin so embeds get an HTTP origin/referrer while IPC remains allowed for the app-local URL.
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
- Implemented local collections with create/rename/delete, multi-collection video assignment, collection counts and combined collection/search/status filtering in the sidebar.
- Fixed summaries showing raw Markdown source: some models (e.g. deepseek-v4-flash via OpenCode Go) wrap their whole reply in a single ```markdown ... ``` fence, which marked renders as one `<pre><code>` block. Now stripped in `parse_summary_response` (backend, so the DB stays clean) with a defensive frontend strip in `markdownToHtml`; both only unwrap a fence that spans the entire text and contains no inner fence. Cleaned the one already-affected DB row.
- Ported the AI provider/model configuration from folio 1:1 (spec: `docs/spec-ai-port.md`): models.dev catalog (embedded snapshot + `ai-catalog.json` cache, refresh only on user click), `ai.json` (enabled providers, per-provider model whitelist, custom providers, defaultModel), `auth.json` with 0600 perms for API keys (never in logs/automation/UI), OpenAI-compatible client with SSE streaming (60s chunk timeout) + JSON fallback. Settings modal rebuilt to folio's tab scheme ("KI-Anbieter" / "KI-Modelle") incl. custom-provider dialog, per-model whitelist toggles, default-model selection and the kept per-model chat test. One-time best-effort migration from the old `config.json` ai block (keys → auth.json, custom endpoints normalized, old selected pair → defaultModel); `config.json` stays untouched on disk. Old `src-tauri/src/ai_config/` module and hardcoded provider catalog removed; automation API returns ai.json (keyless) and catalog short list. Dropped deliberately: Ollama Cloud plan probing, "free only" filter, provider status dots (catalog metadata replaces them).
- Verified:
  - `npm run build`
  - `cargo test`
  - `npm run tauri -- build`
  - Live automation flow for health, transcript loading, summarization and cleanup.
- Refactored the transcript/metadata fetch in `youtube.rs` for robustness and
  fewer requests (spec: `docs/spec-transcript-refactor.md`, notes:
  `docs/impl-notes-transcript-refactor.md`), outer API unchanged:
  - Innertube player call now runs without watch HTML and without the (ignored)
    `key` query param, using a matching ANDROID client User-Agent header.
  - Checks `playabilityStatus` and returns an honest error (status + reason)
    before the generic "no transcript" message.
  - Track selection prefers manual subtitles over ASR per language priority.
  - `fmt=json3` is appended via string ops so the signed caption URL stays
    byte-identical (no more query re-encoding that could break the signature).
  - Watch HTML is fetched at most once per flow (add: publish date + chapters;
    refresh: chapters only) instead of up to three times.

- Added a model picker to the summarize dialog: the dropdown lists all
  whitelisted models of enabled providers (reusing `populateModelPicker` from
  the settings UI), preselects the configured default model and remembers the
  last choice in `localStorage` alongside detail level, language and chapters.
  The choice applies to the single run only and does not change the global
  default model. Backend `summarize_video`/`summarize_video_impl` take optional
  `providerId`/`modelId`; an explicit selection is validated against provider
  enablement and the model whitelist, no selection keeps the previous
  default-model behavior. The automation API accepts `provider_id`/`model_id` in
  the `POST /api/summarize/{id}` body.
- Fixed shifted click targets on Wayland compositors with fractional display
  scaling (`src-tauri/src/lib.rs`, `force_x11_backend_on_wayland`). Symptom: the
  GTK title bar and everything toward the right edge (settings gear, tab
  buttons) was drawn but not clickable, with the active area compressed toward
  the window's left. Measured under Hyprland at scale 1.25: Hyprland reports the
  window as 758x830 logical (948x1038 physical), tao reports `scale_factor = 2`
  with a never-existing inner size of 2400x1426 and `outer = 0x0`, and WebKit
  lays out with 945 CSS pixels (the physical width) while GTK delivers pointer
  events in the logical 758 space. GTK3/WebKitGTK cannot do fractional scaling
  and receives the rounded-up integer scale 2 from the compositor. Forcing
  XWayland (`GDK_BACKEND=x11`) realigns all layers (`scale_factor = 1`,
  `inner = outer = 948`); the title bar disappears there, which suits a tiling
  WM. Guarded so it only applies on Linux + Wayland + an available `DISPLAY`
  (otherwise GTK would fail to start instead of merely mis-scaling), with
  `YOUTUBE_SUMMARIZER_KEEP_WAYLAND=1` as an opt-out. Ruled out by measurement
  first: the `viewport` meta tag, `GDK_SCALE`, process environment (identical to
  folio's), tauri/wry/tao versions, the `tauri-plugin-localhost` load path
  (switching to folio's `tauri://localhost` origin reproduced the bug exactly),
  a compensating `set_zoom`, `set_size(LogicalSize)`, and starting the window
  with `visible: false` plus a later `show()`. Why folio is unaffected on the
  same machine is still unexplained - its frontend is embedded in the installed
  binary, so it could not be instrumented without a full rebuild.
- Escape now closes the open dialog (summarize, settings, collection). The
  global handler clicks the dialog's existing close button instead of hiding the
  modal itself, so the key takes exactly the same path as a mouse click. The
  confirmation dialog and the custom-provider form keep their own Escape
  handling and are skipped by the global handler - otherwise an Escape in the
  upper dialog would have closed the one underneath as well. The collection
  dialog previously reacted to Escape only while the name field had focus; that
  special case was removed in favor of the global handler. Verified headless
  (vite + playwright-core + mocked `window.__TAURI_INTERNALS__`): all three
  dialogs close, Escape without an open dialog does nothing, no console errors.
- Made the missing-codec case explainable instead of silent. On Linux the
  embedded YouTube player only showed "Your browser can't play this video":
  WebKitGTK decodes video through the system's GStreamer, and this machine had
  neither `gst-libav` nor `gst-plugins-good` installed - no video decoder at
  all (`avdec_h264`, `vp9dec`, `vp8dec`, `av1dec` all absent). Two changes:
  - The Video tab now probes `canPlayType` and `MediaSource.isTypeSupported`
    for the codecs YouTube ships (H.264, VP9, VP8) and, if none of them is
    supported, shows a notice naming the cause, the install commands for
    Arch and Debian, and the YouTube fallback link. Deliberately conservative:
    the notice only appears when not a single codec is reported, and only on
    Linux - WebView2 and WKWebView bring their own decoders.
  - `bundle.linux.deb/rpm` now list the GStreamer plugins under `recommends`
    (not `depends`): the app runs fine without them, only the embedded player
    stays silent, and a hard dependency would make the package uninstallable on
    distributions that do not ship H.264 themselves.
  The `#tabVideo` grid got a third row for the notice, otherwise the player
  kept sizing against the full panel height and pushed the notice out of view.

## Next TODOs

- Collections/playlists roadmap:
  - Add playlist URL import next, without user login, for public/unlisted YouTube playlists.
  - Consider optional YouTube account OAuth later for importing the user's own playlists once the local collection model and import UX are stable.
- Next app features after local collections:
  - Import/export.
  - Batch summarization.
  - Refresh metadata/transcripts.
- Improve frontend polish and interaction states.
- Add better empty/error states for transcript and summary failures.
- Follow-up cleanup from the AI/provider settings changes:
  - Replace emoji trash buttons with a consistent icon approach when the frontend icon strategy is decided.
  - Consider splitting future broad UI commits more narrowly when they touch independent areas such as dependencies, link handling, Markdown rendering and settings UX.
- Transcript fetch follow-ups (out of scope of the `youtube.rs` refactor, kept as ideas):
  - Translation fallback via `tlang` for `isTranslatable` caption tracks.
  - Fallback chain over additional Innertube clients (WEB, TV_EMBEDDED) or yt-dlp
    when the ANDROID player response yields no usable captions.
  - Surface transcript fetch failures transparently in the UI so users know what
    is going on. Real-world case (2026-07-16): YouTube's bot check rejects known
    VPN exit IPs (e.g. Mullvad) with `LOGIN_REQUIRED: Sign in to confirm you're
    not a bot` — the backend now produces this honest error, but `add_video`
    swallows it (soft `.ok()`) and the frontend shows a generic "no transcript
    found" message. Ideas: propagate the transcript error message into the
    add-video result/toast, show it on the video detail page, and add a hint
    that a VPN may be the cause (workaround: run the app outside the tunnel,
    e.g. via `mullvad-exclude`).
- Add richer provider metadata such as pricing links, context limits and preferred summarization models.
- Add Windows and macOS packaging notes once tested on those platforms.
- Add release checklist once app behavior stabilizes.
- Review whether automation API responses should return compact video objects to avoid huge payloads from thumbnails/transcripts.
- Backlog: AI provider config reuse/refactor beyond this app. Resolved differently on 2026-07-09: instead of extracting a shared crate, folio's newer implementation was ported back into this app (see `docs/spec-ai-port.md`); `docs/ai-config-refactor.md` is historical context only.
- Follow-ups from the folio AI port: optional UI streaming of summaries (client already streams via SSE), vitest/jsdom setup for the settings UI like folio, richer catalog metadata display (pricing links, context limits).

## Known Notes

- The automation API is only available in debug builds and prints its URL as `AUTOMATION_URL=http://127.0.0.1:<port>/api`.
- Existing videos that were added before the transcript fix may need the `Transkript laden` button or `POST /api/transcript/{id}`.
- The ignored Rust test `fetches_transcript_from_innertube_caption_url` uses live YouTube network access.
- OpenCode Go settings are stored in the app configuration, not in this repository.

## Last Verified State

- Date: 2026-08-26 (codec notice + package recommends)
- Confirmed on the maintainer's machine: after installing the GStreamer plugins
  the embedded YouTube player works. The missing decoders were the whole cause -
  the localhost plugin added earlier against YouTube error 153 was unrelated to
  this failure.
- `npm run build` green. Verified headless in both states (vite +
  playwright-core, codec probes stubbed out for the failure case): with codecs
  present the notice stays hidden, without them it appears, no console errors,
  and a screenshot confirmed the notice sits above the player instead of being
  clipped. The headless run also caught a real bug before it shipped: the codec
  probe ran during event wiring while its module constant was still in the
  temporal dead zone, which left the whole video list empty.
- Date: 2026-08-25 (Escape closes dialogs)
- `npm run build` green, Escape behavior verified headless for all three
  dialogs. The Wayland scaling fix was confirmed by the user in the installed
  app: all buttons work again.
- Date: 2026-08-25 (Wayland scaling fix)
- `cargo fmt`, `cargo test` (45 passed, 1 network test ignored) and
  `npm run build` green. Fix verified live: the app now reports
  `xwayland: True` in `hyprctl clients`, and the user confirmed the settings
  gear and tab buttons are clickable again. The two alternatives were tested in
  the same session and rejected by the user's own click test: unchanged
  (`scale_factor = 2`) and `set_zoom(1.25)` both stayed broken.
- `npm run tauri -- build` rebuilt the release binary plus deb and rpm with the
  fix compiled in (verified: the guard's env var name is present in the binary);
  the AppImage step keeps failing with `failed to run linuxdeploy` (agent
  sandbox cannot mount the linuxdeploy AppImage, unrelated to the code). The
  root symlinks point at the fresh artifacts. Installing the new build is still
  pending on the user's side. Not committed.
- Date: 2026-08-25
- Summarize-dialog model picker: `cargo fmt`, `cargo test` (45 passed incl. 6 new
  `resolve_summary_model` tests, 1 network test ignored) and `npm run build` are
  green. UI verified headless (vite + playwright-core + mocked
  `window.__TAURI_INTERNALS__`): the picker lists only models of enabled
  providers, preselects the default model, and the chosen model is restored on
  reopening the dialog; no console errors. `npm run tauri -- build` compiled the
  release binary and bundled deb + rpm; the AppImage step failed with `failed to
  run linuxdeploy` (agent sandbox cannot mount the linuxdeploy AppImage - rerun
  outside the sandbox if an AppImage is needed). The convenience symlinks
  `youtube-summarizer-release` and `youtube-summarizer.deb` point at the fresh
  artifacts. No dev server or Tauri process left running. Not committed.
- Date: 2026-07-16
- Transcript/metadata refactor (`youtube.rs`, `commands.rs`): all four spec gates
  green from `src-tauri/`: `cargo fmt` (clean), `cargo test` (33 passed, 1 network
  test ignored), network test `fetches_transcript_from_innertube_caption_url
  --ignored` passed (confirms the key-less Innertube call still works), and
  `npm run build` from the repo root. Not committed. No dev/Tauri process left
  running. `npm run tauri -- build` not re-run (no user-visible feature change).
- Date: 2026-07-09
- Settings UI rework after failed review: the first two AI-settings UI passes were rejected (free rebuild instead of folio parity, then cyclic CSS custom properties `--bg: var(--bg)` that invalidated the whole palette, panels visible despite `hidden`, duplicated rule blocks). Final state: single clean `src/settings-ai.css` in this app's design language, ~513 lines of dead legacy settings CSS purged from `styles.css`, provider sorting enabled → keyed/configured → rest (each group alphabetical, `providerRank` as in folio). Verified headless via vite + playwright-core + mocked `window.__TAURI_INTERNALS__` (screenshots + DOM order checks: sorting, single visible panel, search filter, default-model dropdown, badges, custom dialog).
- folio AI port: `cargo test` (28 passed incl. moved fence-strip tests + new ai module tests), `cargo fmt --check`, `cargo build` with 0 warnings and `npm run build` all green. Live functional test via automation API against the running dev app: migration ran on the real config (6 providers incl. 2 custom, keys → auth.json 0600, defaultModel `opencode-go`/`deepseek-v4-flash`), `/api/config` exposes no keys, `POST /api/summarize/60` produced a real summary through the new SSE client with no wrapping fence. A `npm run tauri dev` instance may still be running from that test session.
- Date: 2026-06-17
- Markdown fence fix: `cargo test` (5 new `strip_wrapping_code_fence` unit tests plus existing suite) and `cargo fmt --check` passed; `npm run build` passed. Dev app (`npm run tauri dev`) was running during the change; existing DB row id=35 cleaned in place (backup at `videos.db.bak-20260617`). Reload/restart needed for the in-memory frontend list to pick up the cleaned row, though the frontend strip already renders it correctly via HMR.
- Date: 2026-05-03
- Release build: `npm run tauri -- build` passed after switching the installed app to localhost asset serving for YouTube embeds.
- Build/tests: `npm run build`, `cargo test` and `cargo fmt --check` passed after implementing local collections.
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
