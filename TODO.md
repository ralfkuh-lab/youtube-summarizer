# Working Notes

This is the shared working file for project state, TODOs and handoff notes between sessions.

## Project Goal

Build a cross-platform desktop app for Linux, Windows and macOS that can collect YouTube videos, load transcripts and create AI summaries.

## Current State

The app is the Tauri 2 implementation (TypeScript frontend, Rust backend);
the old Python implementation is long gone. Feature history lives in the git
log and in `docs/` (specs). Highlights of what is shipped: video library with
collections/search/filters, Innertube transcript loading, AI provider/model
configuration ported from folio, live-streaming summaries, and the extended
summarize dialog (prompt presets + modules, summary history, Mermaid,
clickable timestamps, prompt-injection hardening) — spec:
`docs/spec-summary-dialog.md`. Since 2026-08-30 the video description is
stored (`videos.description`, from the watch HTML's player response), shown
as a collapsible block under the detail title and passed to the summarizer
as an untrusted DESCRIPTION prompt block. Timestamps in the description are
seek links into the video tab, and the video player sizes itself against
the actual panel space via container queries (cqw/cqh; the grid rows are
explicitly assigned because the hidden codec notice creates no grid item).
The transcript button is always visible ("Neu laden" once a transcript
exists) so old videos can backfill chapters and description. A "links"
module in the summarize dialog asks the model for a 'Ressourcen' section
built from helpful description links (opt-in, persisted like the other
modules).

## Next TODOs

- Collections/playlists roadmap:
  - Add playlist URL import next, without user login, for public/unlisted YouTube playlists.
  - Consider optional YouTube account OAuth later for importing the user's own playlists once the local collection model and import UX are stable.
- Next app features: import/export, batch summarization, refresh metadata/transcripts.
- Surface transcript fetch failures transparently in the UI. Real-world case
  (2026-07-16): YouTube's bot check rejects known VPN exit IPs (e.g. Mullvad)
  with `LOGIN_REQUIRED: Sign in to confirm you're not a bot` — the backend
  produces this honest error, but `add_video` swallows it (soft `.ok()`) and
  the frontend shows a generic "no transcript found" message. Ideas: propagate
  the error into the add-video result/toast, show it on the video detail page,
  hint that a VPN may be the cause (workaround: `mullvad-exclude`).
- Improve frontend polish, interaction states and empty/error states.
- Transcript fetch ideas: translation fallback via `tlang` for
  `isTranslatable` tracks; fallback chain over additional Innertube clients
  (WEB, TV_EMBEDDED) or yt-dlp when the ANDROID player response yields no
  usable captions.
- Replace emoji trash buttons with a consistent icon approach when the
  frontend icon strategy is decided.
- vitest/jsdom setup for the settings UI like folio.
- Review whether automation API responses should return compact video objects
  to avoid huge payloads from thumbnails/transcripts.
- Add Windows and macOS packaging notes once tested on those platforms.
- Add release checklist once app behavior stabilizes.

## Known Notes

- The automation API is only available in debug builds and prints its URL as `AUTOMATION_URL=http://127.0.0.1:<port>/api`.
- The ignored Rust test `fetches_transcript_from_innertube_caption_url` uses live YouTube network access.
- Node >= 20 is required for development/builds; the installed app does not need Node.
- The app is installed as a deb package (`sudo dpkg -i youtube-summarizer.deb`), see AGENTS.md.

## Last Verified State

- Date: 2026-08-30 (video description feature + seek links + player sizing)
- `cargo fmt`, `cargo test` (75 passed, 1 network test ignored) and
  `npm run build` green. Existing videos get the description only after
  "Transkript laden" (refresh re-reads the watch HTML); new videos store it
  on add. Headless UI check (playwright-core + Tauri mock): description
  block, URL/timestamp links, tab switch with start/autoplay params and
  player fit/aspect verified via screenshots and bounding boxes. No dev
  server or Tauri process left running.
- Previous: 2026-08-29 (extended summarize dialog) — all gates green after
  the cross-review fix round, confirmed live by the maintainer.
- Older verified states were trimmed 2026-08-29; see the git history of this
  file if needed.
