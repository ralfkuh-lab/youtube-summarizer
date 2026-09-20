# Agent Instructions

This repository is a Tauri 2 YouTube summarizer desktop app. Work in the Tauri app by default.

## Important Files

- `src/main.ts`: frontend entry point and bootstrap (DOM init, top bar event wiring, modal escape handling).
- `src/library.ts`: video list, filtering, searching, video selection/deletion, and collection management.
- `src/detail.ts`: video detail view, transcript display, chapter navigation, seek handling, and video player embedding.
- `src/summary-view.ts`: summary tab view, version history, markdown rendering, timestamp linking, and Mermaid diagram rendering.
- `src/summary-dialog.ts`: summary creation dialog, settings persistence, preset management, and prompt composition.
- `src/state.ts`: shared frontend application state object (`state`), active video getters, and status/busy helpers.
- `src/template.ts`: HTML shell template.
- `src/types.ts`: shared TypeScript type and interface definitions.
- `src/utils.ts`: pure utility helpers (formatting, search normalization, collection comparison).
- `src/ai-config.ts`: settings UI for the "KI-Anbieter" / "KI-Modelle" tabs.
- `src/agent-handoff.ts`: "An Agent übergeben" button, handoff dialog, clipboard copy (with textarea/`execCommand` fallback), and "Ordner öffnen".
- `src/agent-settings.ts`: settings UI for the "Agent" tab (workdir, shell, active template, custom templates, prompt, live preview); shares its cached config view with the handoff dialog.
- `src/styles.css`: frontend styling.
- `src-tauri/src/commands.rs`: Tauri command layer delegating to domain modules.
- `src-tauri/src/summarize.rs`: AI summary target resolution, prompt building, untrusted content delimiters, and streaming summary orchestration.
- `src-tauri/src/agent_handoff.rs` (with `agent_handoff/{config,quote,resolve,context,selection}.rs`, tests in `agent_handoff/tests.rs`, `agent_handoff/tests/context_tests.rs`, `agent_handoff/tests/selection_tests.rs` and `agent_handoff/tests/fixtures.rs`): local-agent handoff — `agent.json` config and validation, shell quoting, one-pass command resolution, slug/path building, per-handoff context selection (transcript, summary versions, chats), context file rendering and atomic writing, and the `agent_config_get` / `agent_config_set` / `agent_prepare` / `agent_preview` commands. See `docs/spec-agent-handoff.md`.
- `src-tauri/src/ai/migration.rs`: legacy AI config migration.
- `src-tauri/src/ai/`: AI provider/model config (models.dev catalog, ai.json, auth.json) and the OpenAI-compatible chat client; ported from folio, see `docs/spec-ai-port.md`.
- `src-tauri/src/youtube.rs`: YouTube metadata, transcript and chapter fetching.
- `src-tauri/src/storage.rs`: config and SQLite persistence.
- `src-tauri/src/automation.rs`: debug-only local automation API for functional tests.
- `TODO.md`: current collaboration state, open tasks and session handoff notes (kept lean).
- `docs/verification-log.md`: history of verified states and test results; not needed for normal work.

## Commands

Use these from the repository root unless noted otherwise:

```bash
npm run build
npm run test:ui
npm run tauri dev
```

UI-Tests mit `npm run test:ui` suchen einen installierten Chromium-basierten Browser (Linux: Chromium/Chrome, Windows und macOS: Chrome, Edge, Chromium), überschreibbar per `CHROMIUM_PATH`. Jede Testdatei startet einen eigenen Vite-Server ab Port 5199 und weicht bei belegtem Port aus; `UI_TEST_PORT` erzwingt einen festen Port.

Use these from `src-tauri/`:

```bash
cargo test
cargo fmt
```

Network-dependent transcript test:

```bash
cargo test fetches_transcript_from_innertube_caption_url -- --ignored
```

## Working Rules

- Keep changes scoped to the Tauri app unless asked otherwise.
- Do not commit API keys, local databases or generated build output.
- Prefer existing patterns in the app over introducing new frameworks.
- When changing transcript, AI or storage behavior, run `npm run build` and `cargo test`.
- After finishing a feature, run `npm run tauri -- build` so the project-root symlinks (`youtube-summarizer-release`, `youtube-summarizer.deb`) point to current artifacts.
- On the maintainer's Linux machine the app is installed as a deb package (dpkg name `you-tube-summarizer`, binary at `/usr/bin/youtube-summarizer`, since 2026-08-25; the earlier `~/.local/bin` plain-copy install no longer exists). After a release build, the installed app is updated with `sudo dpkg -i youtube-summarizer.deb` (project-root symlink). Agents cannot run sudo — ask the maintainer to run it. The app launches through the packaged desktop entry; the former user-local `mullvad-exclude` wrapper was removed on 2026-09-19, so with an active VPN YouTube may answer `LOGIN_REQUIRED`.
- If testing the running app, use the dev-only automation API printed by `npm run tauri dev`.

## Windows

- Prerequisites: Node >= 20, Rust (MSVC toolchain) and the Tauri CLI from `npm install`.
- `npm run tauri -- build` writes the installers to `src-tauri/target/release/bundle/nsis/` and `src-tauri/target/release/bundle/msi/`. A local shortcut `youtube-summarizer-setup.lnk` in the project root is ignored by git, like the Linux symlinks.
- `.gitattributes` forces LF line endings; do not commit CRLF files.
- `src-tauri/gen/schemas/` is generated per platform on every build and is not tracked.
- The PowerShell form of the built-in agent templates (`src-tauri/src/agent_handoff/config.rs`) is tested only as a string (quoting and command shape), not executed on Windows; that run is still open.

## Current Architecture

The frontend invokes Tauri commands for all application actions. The backend stores app data in the Tauri app data directory, not in the repository root. YouTube transcripts are fetched through the Innertube player endpoint because direct web caption URLs can fail for some videos. Summaries are sent to OpenAI-compatible chat completion endpoints.

## Session Handoff

Before ending a substantial coding session:

- Update `TODO.md` with completed work and open issues. Keep it lean (every agent reads it at start): collapse finished items to one line, keep only the latest entry under "Last Verified State" and prepend the detailed test results to `docs/verification-log.md`.
- Mention whether a dev server or Tauri process is still running.
- Keep final user summaries short and concrete.
