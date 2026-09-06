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
modules). Since 2026-09-05 transcript fetch failures are persisted per video
in `videos.transcript_error` and displayed across the UI (status message on add,
dedicated error card with VPN guidance on `LOGIN_REQUIRED` in the transcript
tab, and detail tooltip on the 'T' status chip) — spec:
`docs/spec-transcript-error.md`.

## Next TODOs

- [ ] Code-Review vom 2026-09-06 abarbeiten: [Befunde und Abhilfen](docs/code-review-2026-09-06.md).
  - [x] 1: Race Conditions beim Videowechsel und Transkript-Neuladen beheben (Etappe 1).
  - [x] 2: Gemeinsamen KI-Konfigurationszustand verwenden und Speicheränderungen bei Schreibfehlern verhindern (Etappe 2).
  - [x] 3: Unvollständig beendete KI-Streams erkennen (Etappe 1).
  - [x] 4: Schlanke Video-Listenobjekte und gesammelte Sammlungsabfragen einführen (Etappe 2).
  - [x] 5: Standardmodell genauso wie explizite Modellauswahl validieren (Etappe 1).
  - [ ] Modulgrenzen in `src/main.ts` und `commands.rs` refactoren; gezielte Regressionstests ergänzen (Etappe 3).
- Collections/playlists roadmap:
  - Add playlist URL import next, without user login, for public/unlisted YouTube playlists.
  - Consider optional YouTube account OAuth later for importing the user's own playlists once the local collection model and import UX are stable.
- Next app features: import/export, batch summarization, refresh metadata/transcripts.
- Improve frontend polish, interaction states and empty/error states.
- Transcript fetch ideas: translation fallback via `tlang` for
  `isTranslatable` tracks; fallback chain over additional Innertube clients
  (WEB, TV_EMBEDDED) or yt-dlp when the ANDROID player response yields no
  usable captions.
- Replace emoji trash buttons with a consistent icon approach when the
  frontend icon strategy is decided.
- vitest/jsdom setup for the settings UI like folio.
- Add Windows and macOS packaging notes once tested on those platforms.
- Add release checklist once app behavior stabilizes.

## Known Notes

- The automation API is only available in debug builds and prints its URL as `AUTOMATION_URL=http://127.0.0.1:<port>/api`.
- The ignored Rust test `fetches_transcript_from_innertube_caption_url` uses live YouTube network access.
- Node >= 20 is required for development/builds; the installed app does not need Node.
- The app is installed as a deb package (`sudo dpkg -i youtube-summarizer.deb`), see AGENTS.md.

## Last Verified State

- 2026-09-06: Etappe 2 umgesetzt (`SummaryTarget` entkoppelt
  `summarize_video_impl` von den Konfigurationsspeichern, Setter in
  `AiConfigService`/`AuthStore` übernehmen erst nach erfolgreichem Save,
  Sammlungszuordnungen in einer Abfrage, schlanke Listenobjekte mit
  `has_transcript`/`has_summary`; `openSummaryDialog` hydriert schlanke Objekte
  nach). `cargo fmt`, `cargo test` (94 bestanden, 1 Netzwerktest ignoriert),
  `npm run build` und `npm run test:ui` (5 bestanden) grün. Kein Dev-Server aktiv.
- 2026-09-06: Etappe 1 umgesetzt (Race Conditions, Standardmodell-Validierung,
  Stream-Abschluss, UI-Testharness, gleiche Guards in deleteActiveVideo und
  updateActiveVideoCollections). `cargo fmt`, `cargo test` (87 bestanden, 1
  Netzwerktest ignoriert), `npm run build` und `npm run test:ui` (3 bestanden) grün.
  UI-Regressionstests vor Fix rot und nach Fix grün belegt. Kein Dev-Server aktiv.
- 2026-09-06: Code-Review dokumentiert in `docs/code-review-2026-09-06.md`;
  Abarbeitung oben erfasst. `npm run build` erfolgreich, `cargo test` mit
  80 bestandenen Tests und einem ignorierten Netzwerktest. Race Condition beim
  Videowechsel mit verzögerten Antworten reproduziert. Keine Implementierungsänderungen;
  im Review und bei der Dokumentation keinen Dev-Server oder Tauri-Prozess gestartet.
- Date: 2026-09-05 (transcript error visibility feature, spec: `docs/spec-transcript-error.md`)
- `cargo fmt`, `cargo test` (80 passed, 1 network test ignored) and
  `npm run build` green. Transcript load failures are stored in `videos.transcript_error`
  on add and refresh (when no transcript is present), cleared upon successful
  transcript update, and surfaced in the add status line, transcript tab (with VPN
  actionable hint on `LOGIN_REQUIRED`), and T-chip tooltip. No dev server or Tauri
  process left running.
- Previous: 2026-08-30 (video description feature + seek links + player sizing) —
  75 passed, 1 ignored; npm run build green.
- Older verified states were trimmed 2026-08-29; see the git history of this
  file if needed.
