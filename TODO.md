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
`docs/spec-transcript-error.md`. Shipped and verified by the maintainer:
multi-device sync (`docs/spec-sync.md`, server
`https://yt-sync.srv1280390.hstgr.cloud`) and the local-agent handoff
(`docs/spec-agent-handoff.md`).

## Next TODOs

- Video-Chat ([Spec](docs/spec-video-chat.md)) — umgesetzt und reviewt; seit 2026-10-09 werden Tool-Ergebnisse früherer Runden beim Senden auf 2 000 Zeichen gekürzt (Delimiter bleiben geschlossen), und Modelle gelten als tool-fähig, außer der Katalog sagt `tool_call: false`. Offen:
  - [ ] Manuell in der installierten App prüfen: Live-Streaming, Tool-Aktivität während der Anfrage, „Stopp“, Video-/Chatwechsel während einer Anfrage (bisher nur per UI-Tests mit Mock und per Automation-API belegt).
- [ ] Oberfläche (2026-10-09: SVG-Icons statt Emoji, einheitliche Leerzustände, Fokus-/Hover-/Busy-Stile) in der installierten App unter WebKitGTK ansehen.
- Agent-Übergabe unter Windows: PowerShell-Vorlage tatsächlich ausführen (bisher nur als Zeichenkette getestet, siehe `AGENTS.md`).
- macOS: erster Build und Start nach [docs/release.md](docs/release.md) (dort die ungeprüften Punkte bestätigen).
- UI-Tests: vereinzelt 30-s-Start-Timeouts in `tests/ui/harness.mjs` (Seite lädt, App bootet nicht; Ursache vermutlich Vites Dependency-Optimizer, „504 Outdated Optimize Dep“, verstärkt durch parallele Läufe mit geteiltem `node_modules/.vite`). Betroffene Tests sind einzeln grün. Abhilfe prüfen: ein Vite-Server pro Testdatei statt pro Test oder `optimizeDeps.include`.
- Ideen:
  - Import/Export, mehrere Videos auf einmal zusammenfassen, Metadaten/Transkripte gesammelt neu laden (pro Video gibt es „Neu laden“).
  - Transkript-Ausweichwege: Übersetzung via `tlang` für `isTranslatable`-Spuren; weitere Innertube-Clients (WEB, TV_EMBEDDED) oder yt-dlp, wenn der ANDROID-Client keine Untertitel liefert.
  - Fehlerkarte im Zusammenfassungs-/Chat-Tab (Fehler stehen bisher nur in der Statuszeile).
  - Abhaken des Standardmodells lässt `defaultModel` stehen (Statuszeile zeigt es weiter) – entscheiden, ob es zurückgesetzt werden soll.
  - Niedrige Priorität: Playlist-URL-Import (öffentlich/nicht gelistet), später ggf. YouTube-OAuth.

## Known Notes

- The automation API is only available in debug builds and prints its URL as `AUTOMATION_URL=http://127.0.0.1:<port>/api`.
- The ignored Rust test `fetches_transcript_from_innertube_caption_url` uses live YouTube network access.
- Node >= 20 is required for development/builds; the installed app does not need Node.
- The app is installed as a deb package (`sudo dpkg -i youtube-summarizer.deb`), see AGENTS.md.

## Last Verified State

Nur der letzte Stand; ältere Einträge stehen in [docs/verification-log.md](docs/verification-log.md).

- 2026-09-24: Sync (Spec Revision 4.1) abgenommen: `src-tauri` `cargo test` (381 bestanden, 4 ignoriert), `sync-server` `cargo test` (35 + 4), `sync-proto` (9), `npm run test:ui` (115), `npm run tauri -- build` grün. Live-Test mit zwei Datenverzeichnissen gegen den VPS (Kopie der echten DB: 147 Videos identisch übertragen); danach Server-Daten zurückgesetzt. Installation per `sudo dpkg -i youtube-summarizer.deb` steht aus. Kein Dev-Server aktiv.
