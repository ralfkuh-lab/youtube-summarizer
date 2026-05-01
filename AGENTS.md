# Agent Instructions

This repository is a Tauri 2 rewrite of a previous Python YouTube summarizer. Work in the Tauri app by default. The legacy Python code in `legacy-python/` is reference material only unless the user explicitly asks to change it.

## Important Files

- `src/main.ts`: frontend UI and Tauri command calls.
- `src/styles.css`: frontend styling.
- `src-tauri/src/commands.rs`: Tauri command layer and shared command implementations.
- `src-tauri/src/youtube.rs`: YouTube metadata, transcript and chapter fetching.
- `src-tauri/src/ai.rs`: AI provider request handling.
- `src-tauri/src/storage.rs`: config and SQLite persistence.
- `src-tauri/src/automation.rs`: debug-only local automation API for functional tests.
- `TODO.md`: current collaboration state, open tasks and session handoff notes.

## Commands

Use these from the repository root unless noted otherwise:

```bash
npm run build
npm run tauri dev
```

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

- Keep changes scoped to the Tauri rewrite unless asked otherwise.
- Preserve `legacy-python/` until the user confirms deletion.
- Do not commit API keys, local databases or generated build output.
- Prefer existing patterns in the app over introducing new frameworks.
- When changing transcript, AI or storage behavior, run `npm run build` and `cargo test`.
- If testing the running app, use the dev-only automation API printed by `npm run tauri dev`.

## Current Architecture

The frontend invokes Tauri commands for all application actions. The backend stores app data in the Tauri app data directory, not in the repository root. YouTube transcripts are fetched through the Innertube player endpoint because direct web caption URLs can fail for some videos. Summaries are sent to OpenAI-compatible chat completion endpoints.

## Session Handoff

Before ending a substantial coding session:

- Update `TODO.md` with completed work, open issues and useful test results.
- Mention whether a dev server or Tauri process is still running.
- Keep final user summaries short and concrete.
