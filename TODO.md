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

- Video-Chat ([Spec](docs/spec-video-chat.md)) — Etappen 1–3, Websuche und Live-Anzeige sind umgesetzt und reviewt. Offen:
  - [ ] Manuell in der installierten App prüfen: Live-Streaming, Tool-Aktivität während der Anfrage, „Stopp“, Video-/Chatwechsel während einer Anfrage (bisher nur per UI-Tests mit Mock und per Automation-API belegt).
  - [ ] Später erwägen: Obergrenze für gespeicherte Tool-Ergebnisse (eine voll ausgereizte Recherche-Runde speichert ~240 000 Zeichen, die jede Folgefrage mitsendet); Checkbox „unterstützt Tool-Calling“ für Custom-Modelle (ohne Katalog-Flag bleibt die Websuche ausgegraut).
- Video an lokalen Agenten übergeben ([Spec](docs/spec-agent-handoff.md)) — Etappe 1 umgesetzt, reviewt und am 2026-09-20 vom Maintainer nativ geprüft (Zwischenablage unter WebKitGTK, Agentenstart). Die App startet keinen Prozess; das Kommando geht nur in die Zwischenablage. Offen:
  - [x] Kontextauswahl im Übergabe-Dialog (2026-09-20, Spec Revision 3): Transkript an/aus, Zusammenfassungs-Versionen und einzelne Chats pro Übergabe, Zeichenzahl der Kontextdatei, gemerkte Auswahl je Video; die Einstellungen im Tab „Agent“ sind nur noch Vorbelegung (neu: „Transkript aufnehmen“). Leere Versionen und Chats ohne exportierbare Nachricht sind nicht wählbar; Recherche-Ergebnisse werden bewusst nicht exportiert. Reviewt von Gemini und GLM 5.3 Flash, Nachprüfung durch Gemini.
  - [x] Eigennamen aus Auto-Transkripten (2026-09-20): fester Zusatz `summarize::PROPER_NAMES_NOTE` im Systemprompt von Zusammenfassung und Chat (gilt auch für eigene Presets) und Hinweiszeile im Kopf der Kontextdatei.
  - [ ] Manuell in der installierten App prüfen (WebKitGTK): Bereich „Kontext“ im Übergabe-Dialog, auch per Tastatur; Wirkung des Eigennamen-Hinweises an einem Video mit falsch transkribierten Namen.
  - [ ] Windows: PowerShell-Vorlage ausführen (bisher nur als Zeichenkette getestet, siehe `AGENTS.md`). Stufe 2 (Direktstart im Terminal) nur bei Bedarf.
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
- Add macOS packaging notes once tested there (Windows: siehe Abschnitt „Windows“ in `AGENTS.md`).
- Add release checklist once app behavior stabilizes.

## Known Notes

- The automation API is only available in debug builds and prints its URL as `AUTOMATION_URL=http://127.0.0.1:<port>/api`.
- The ignored Rust test `fetches_transcript_from_innertube_caption_url` uses live YouTube network access.
- Node >= 20 is required for development/builds; the installed app does not need Node.
- The app is installed as a deb package (`sudo dpkg -i youtube-summarizer.deb`), see AGENTS.md.

## Last Verified State

Nur der letzte Stand; ältere Einträge stehen in [docs/verification-log.md](docs/verification-log.md).

- 2026-09-20: Kontextauswahl (Spec Revision 3) und Eigennamen-Hinweis vom Orchestrator abgenommen:
  `cargo fmt --check`, `cargo test` (299 bestanden, 4 ignoriert), `npm run build`, `npm run test:ui`
  (103 bestanden), `npm run tauri -- build` grün. Nach dem ersten nativen Test des Maintainers wurde der
  Bereich „Kontext“ neu aufgebaut (keine Radios, Haken und Text in einer Zeile; per Screenshot geprüft). Nativer Backend-Durchlauf über die
  Automation-API mit einer Datenkopie; der Dialog selbst ist nur per UI-Test mit Mock belegt.
  Installation per `sudo dpkg -i youtube-summarizer.deb` steht aus. Kein Dev-Server aktiv.
