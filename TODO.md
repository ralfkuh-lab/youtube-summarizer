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

- [ ] Video-Chat ([Spec](docs/spec-video-chat.md)):
  - [x] Etappe 1: Chat über ein Video (Backend, Chat-Tab, Tests).
  - [x] Etappe 2a/2b: Websuche per Tool-Calling (SearXNG + Seitenabruf mit Adresssperre, Tool-Schleife, Einstellungs-Tab „Websuche“, Tool-Aktivität im Chat). Reviewt von Grok, Gemini und Opus (Codex fiel wegen Limit aus).
  - [ ] Manuell in der installierten App prüfen: Live-Streaming, Tool-Aktivität während der Anfrage, „Stopp“, Video-/Chatwechsel während einer Anfrage (bisher nur per UI-Tests mit Mock und per Automation-API belegt).
  - [ ] Etappe 3: Kontext-Wähler (Transkript an/aus, Auswahl der Zusammenfassungs-Versionen pro Chat).
  - [ ] Später erwägen: Obergrenze für gespeicherte Tool-Ergebnisse (eine voll ausgereizte Recherche-Runde speichert ~240 000 Zeichen, die jede Folgefrage mitsendet); Checkbox „unterstützt Tool-Calling“ für Custom-Modelle (ohne Katalog-Flag bleibt die Websuche ausgegraut).
- [ ] Video an lokalen Agenten übergeben ([Spec-Entwurf](docs/spec-agent-handoff.md)): Kontextdatei exportieren, Kommando in die Zwischenablage; Spec-Review läuft.
- [x] Code-Review vom 2026-09-06 abarbeiten: [Befunde und Abhilfen](docs/code-review-2026-09-06.md).
  - [x] 1: Race Conditions beim Videowechsel und Transkript-Neuladen beheben (Etappe 1).
  - [x] 2: Gemeinsamen KI-Konfigurationszustand verwenden und Speicheränderungen bei Schreibfehlern verhindern (Etappe 2).
  - [x] 3: Unvollständig beendete KI-Streams erkennen (Etappe 1).
  - [x] 4: Schlanke Video-Listenobjekte und gesammelte Sammlungsabfragen einführen (Etappe 2).
  - [x] 5: Standardmodell genauso wie explizite Modellauswahl validieren (Etappe 1).
  - [x] Modulgrenzen in `src/main.ts` und `commands.rs` refactoren; gezielte Regressionstests ergänzen (Etappe 3).
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

- 2026-09-19: Nachbesserung zum Praxistest-Paket (N1/N2): `looks_like_tool_markup` entfernt
  vorab Markdown-Codeblöcke und Inline-Code und erkennt Markup nur noch am Textanfang, als
  reines `name`/`arguments`-JSON oder bei `|DSML|` mit < 200 Zeichen Vorspann (keine
  Fehlalarme mehr bei Erklärtexten über Tool-Calling); Aufklapp-Marker der Tool-Schritte
  sichtbar (▶/▼, Pointer, Hover; Fehler-Schritte ohne Marker/Pointer). Tests: L18 erweitert,
  U50 neu. Gates: `cargo fmt --check` sauber, `cargo test` (222 bestanden, 4 ignoriert),
  `npm run build`, `npm run test:ui` (58 bestanden) und `npm run tauri -- build` grün.
  Mutationsbelege M13/M14 im Bericht `.herd/impl-2c-bericht.md` (Abschnitt „Nachbesserung“).
  Installation per `sudo dpkg -i youtube-summarizer.deb` steht beim Maintainer aus.

- 2026-09-19: Korrekturpaket aus dem ersten Praxistest (F0–F3): Kontext-Button ohne
  doppeltes Präfix, Schlussanfrage der Websuche mit `tool_choice: "none"` + nicht
  gespeicherter Abschluss-Nachricht (Rückfall ohne `tools` bei 400/422) und
  Sicherheitsnetz `looks_like_tool_markup` (genau eine Wiederholung, sonst
  `Das Modell hat nach der Recherche keine Antwort geliefert …`), Budget im
  `WEB_SEARCH_PROMPT_ADDENDUM` und Schlusszeile an der letzten Tool-Nachricht der Runde 5;
  Tool-Schritte als je ein `<details>` mit Label im `<summary>` plus Gruppenzeile
  `Recherche · N Schritte`; Websuche-Einstellungen mit Platzhalter „z. B. …“ und
  Hinweis „Bitte zuerst eine SearXNG-URL eintragen“ statt Backend-Fehler. Neue Tests
  L13–L18, U45–U49. Gates: `cargo fmt --check` sauber, `cargo test` (222 bestanden,
  4 ignoriert, ohne Warnungen), `npm run build`, `npm run test:ui` (57 bestanden) und
  `npm run tauri -- build` grün. Mutationsbeleg M13 (ohne Markup-Erkennung) → L13/L14/L17
  rot. Bericht: `.herd/impl-2c-bericht.md`. Installation per
  `sudo dpkg -i youtube-summarizer.deb` steht beim Maintainer aus.

- 2026-09-19: Korrekturen Etappe 3 (E1–E3): Versionen im Kontext-Popover sind jetzt
  **Checkboxen** (Mehrfachauswahl bis 5, „Neueste“/„Keine“ als Radios, letzte Checkbox
  abgewählt → „Neueste“; Hinweis „Höchstens 5 Zusammenfassungen“), kompakte
  Button-Beschriftung mit vollem Text im `title` (`Kontext: Transkript + 2 Zus.`), Popover
  öffnet linksbündig unter dem Kontext-Button und bleibt im Viewport, Laufregister nach
  `src-tauri/src/chat_runs.rs` ausgelagert (`chat.rs` jetzt 502 Zeilen). Tests U42–U45.
  Gates: `cargo fmt --check` sauber, `cargo test` (216 bestanden, 4 ignoriert, ohne
  Warnungen), `npm run build`, `npm run test:ui` (52 bestanden) und `npm run tauri -- build`
  grün. Mutationsbeleg M13 (Radio statt Checkbox) → U42/U43 rot. Bericht:
  `.herd/impl-3-korrekturen-bericht.md`. Installation per
  `sudo dpkg -i youtube-summarizer.deb` steht beim Maintainer aus.

- 2026-09-19: Video-Chat Etappe 3 (Kontext-Wähler): Spalte `chats.context_options`
  (nachgerüstet per `ensure_table_column`, X12), `ChatContextOptions` in Modell/Chat,
  `ChatContext::resolve` (gewählte Versionen älteste zuerst, fremde/gelöschte IDs entfallen,
  max. 5, `NO_TRANSCRIPT_ADDENDUM`, `Kein Kontext gewählt …`), eindeutige Delimiter je
  Geschwisterblock, Optionen werden mit der Runde gespeichert (X10) und `chat_context_set`;
  Frontend-Popover `#chatContextBtn`/`#chatContextMenu` (`src/chat-context.ts`), U5
  präzisiert (Eingabe ohne Transkript nutzbar, wenn eine Zusammenfassung existiert). Tests
  X1–X12 und U35–U41. Gates: `cargo fmt --check` sauber, `cargo test` (216 bestanden,
  4 ignoriert, ohne Warnungen), `npm run build`, `npm run test:ui` (48 bestanden) und
  `npm run tauri -- build` grün. Mutationsbelege M11 (X3/X11) und M12 (X10) in
  `/tmp/yts-mut-3-ds`. Bericht: `.herd/impl-3-bericht.md`. Installation per
  `sudo dpkg -i youtube-summarizer.deb` steht beim Maintainer aus.

- 2026-09-19: Abnahme Etappe 2 Video-Chat (Websuche). Nativer Durchlauf mit
  `npm run tauri dev` (isoliertes `XDG_DATA_HOME`, danach gelöscht) über die
  Automation-API gegen OpenRouter `deepseek/deepseek-v4.1-flash` und das lokale SearXNG
  (`http://127.0.0.1:8080`): fünf Tool-Runden (6 Suchen, 5 Seitenabrufe), Schlussanfrage
  ohne `tools`, Antwort trennt Video- und Web-Aussagen mit verlinkten Quellen;
  Folgefrage ohne Websuche auf den Tool-Verlauf funktioniert. Der native Lauf fand vor den
  beiden 2b-Korrekturpaketen statt; danach nur Gates: `cargo fmt --check`, `cargo test`
  (203 bestanden, 4 ignoriert), `npm run build`, `npm run test:ui` (41 bestanden),
  `npm run tauri -- build` erfolgreich. Nicht nativ geprüft: Chat-Tab selbst (Streaming,
  Tool-Aktivität, Stopp). Kein Dev-Server aktiv.
- 2026-09-19: Video-Chat Korrekturpaket 2 zu Etappe 2b (Opus/Grok): Schluss-Tags lösen
  keinen Gerüst-Skip mehr aus (D1), unabgeschlossene Gerüst-Elemente behalten ihren Inhalt
  (nur Rohtext wird verworfen, D2), entfernte Tags trennen Wörter (Inline-Ausnahmen, D3),
  rohe Kontextteile werden einmal je `chat_send_impl` vorgehalten und nur entliehen (D4,
  `ExtraParts`), Frontend kürzt Labels nach Codepunkten (D5). Gates: `cargo fmt --check`
  sauber, `cargo test` (203 bestanden, 4 ignoriert, ohne Warnungen), `npm run build`,
  `npm run test:ui` (41 bestanden) und `npm run tauri -- build` grün. Mutationsbelege im
  Bericht `.herd/impl-2b-korrekturen-2-bericht.md`. Installation per
  `sudo dpkg -i youtube-summarizer.deb` steht beim Maintainer aus.

- 2026-09-19: Video-Chat Korrekturpaket Etappe 2b (Grok/Gemini/Opus/Orchestrator):
  Kontextblöcke werden vor **jeder** Provider-Anfrage neu gebaut (Tool-Ergebnisse der Runde
  fließen in die Delimiter ein, Transkript einmal pro Runde), feste Modell-Fehlertexte ohne
  Fremdtext (+200-Zeichen-Kürzung und `=`-Neutralisierung), Abbruch mitten im Tool-Aufruf
  (250-ms-Abfrage), eindeutige `tool_call`-IDs, Events nur für ausgeführte Aufrufe
  (`kind` um `other`), klare Meldung bei leerer Schlussantwort; `websearch.rs` in
  `fetch`/`search`/`tools` aufgeteilt; HTML-Extraktor mit `>`-Rückfall, leeren Kommentaren,
  Seitengerüst-Skip und linearen Laufzeittests; Frontend: Aktivitätszeilen aktualisieren
  statt verdoppeln, Tool-Schritte an ihrer Assistant-Nachricht (über `tool_call_id`) mit
  lesbaren Kopfzeilen und bereinigtem Inhalt. Gates: `cargo fmt --check` sauber, `cargo test`
  (197 bestanden, 3 ignoriert, ohne Warnungen), `npm run build` und `npm run test:ui`
  (40 bestanden) grün, `npm run tauri -- build` erfolgreich. Mutationsbelege M6/M7/M8 und
  M10 in `/tmp/yts-mut-2b-ds`. Bericht: `.herd/impl-2b-korrekturen-bericht.md`.
  Installation per `sudo dpkg -i youtube-summarizer.deb` steht beim Maintainer aus.

- 2026-09-19: Video-Chat Etappe 2b (Websuche am Chat): Frontend-Modulschnitt
  (`src/chat-state.ts`, `src/chat-render.ts`), `websearch.json` mit Commands
  `web_search_config_get/set/test`, Tool-Schleife in `chat.rs` (max. 5 Runden, max. 4
  Calls, Schlussanfrage ohne `tools`, feste Fehlertexte, `WEB RESULT`-Verpackung,
  Commit nach Erfolg), Event `ai:chat_tool`, `webSearch` in `chat_send` und in
  `POST /api/chat/<id>`, Einstellungs-Tab „Websuche“ (`src/websearch-settings.ts`),
  Chat-Schalter `#chatWebSearch`, Tool-Aktivität und eingeklappte Tool-Schritte im
  Verlauf. Tests L1–L10 (+L8b/L10b) und U25–U31. Gates: `cargo fmt --check` sauber,
  `cargo test` (192 bestanden, 3 ignoriert), `npm run build` und `npm run test:ui`
  (37 bestanden) grün, `npm run tauri -- build` erfolgreich. Mutationsbelege M6 (L1),
  M7 (L6), M8 (L5) in `/tmp/yts-mut-2b`. Bericht: `.herd/impl-2b-bericht.md`.
  Installation per `sudo dpkg -i youtube-summarizer.deb` steht beim Maintainer aus;
  kein Dev-Server oder Tauri-Prozess gestartet.

- 2026-09-19: Video-Chat Korrekturpaket 2 zu Etappe 2a (Nachprüfungen Opus/Grok):
  `html_to_text` ist jetzt ein einziger Vorwärtsdurchlauf (vorab letztes `>`, letztes
  Kommentarende, letzte Abschluss-Tags; anführungsbewusstes Tag-Ende; Rohtext ohne
  Abschluss wird bis zum nächsten `<` verworfen), Extraktion läuft per `spawn_blocking`
  (Q1); der Kindprozess-Helfer der Proxy-Tests prüft die Ausgabe („test result: ok.
  1 passed“), damit ein Tippfehler im Testnamen nicht fälschlich grün ist (Q2); der
  SearXNG-Client läuft **immer** ohne System-Proxy (Q3); leere `tool_calls` werden
  weggelassen (Q4); S12 prüft Content-Types case-insensitiv, S13b belegt den Abbruch
  mitten im Stream (Q5). Gates: `cargo fmt --check` sauber, `cargo test` (174 bestanden,
  3 ignoriert), `npm run build` und `npm run test:ui` (30 bestanden) grün. Mutationsbeleg
  M9 (naive Variante → Laufzeittest rot) in `/tmp/yts-mut-2a`. Bericht:
  `.herd/impl-2a-korrekturen-2-bericht.md`. Kein Dev-Server oder Tauri-Prozess gestartet.

- 2026-09-19: Video-Chat Korrekturpaket 2a (Reviews Grok/Gemini/Opus): `fetch_page` und die
  lokale Suche ohne System-Proxy (`no_proxy`) – `HTTP_PROXY` konnte Resolver-Filter und
  Adressprüfung umgehen (K1, Test S17 über einen Kindprozess); NAT64-Regeln korrigiert
  (`64:ff9b:1::/48` komplett gesperrt, `64:ff9b::/96` exakt 96 Bit, IPv4-translated, `fec0::/10`)
  samt Präfixgrenzen (K2/K3); `web_search` ohne automatische Redirects mit eigenem Fehlertext
  und `no_proxy` für lokale Instanzen (K4); `content: null` nur noch bei Assistant mit
  nichtleeren `tool_calls` (K5); leere `tools` werden weggelassen (K6); Abschluss-/Abbruchpfade
  getestet (T11–T14, K7); 30-s-Gesamtbudget (K8); eigene Fehlertexte für fehlenden
  Content-Type, 3xx ohne Location und leeren Text (K9); `html_to_text` mit Kommentaren,
  nacktem `<`, unabgeschlossenen Blöcken und Blockumbrüchen (K10). Gates: `cargo fmt --check`
  sauber, `cargo test` (170 bestanden, 3 ignoriert), `npm run build` und `npm run test:ui`
  (30 bestanden, unverändert) grün. Mutationsbelege M4 (S17 rot ohne `no_proxy`) und
  M5 (S16 rot bei verschobener 172.16/12-Grenze) in `/tmp/yts-mut-2a`. Bericht:
  `.herd/impl-2a-korrekturen-bericht.md`. Kein Dev-Server oder Tauri-Prozess gestartet.

- 2026-09-19: Video-Chat Etappe 2a (Tool-Calling im Client + Webtools, noch nicht an
  die Chat-Schleife angeschlossen): `ChatMessage.content` ist jetzt `Option<String>` mit
  `tool_calls`/`tool_call_id` (leerer Text → JSON `null`, T9), gemeinsame Stream-Bausteine
  in `ai/client.rs` (`read_sse_stream`, `send_chat_request`, `SseStep`), neu
  `ai/tool_stream.rs` (Delta-Zusammenbau nach `index`, JSON-Fallback) und `websearch.rs`
  (Adresssperren inkl. eingebettetem IPv4, eigener DNS-Resolver mit Filter, manuelle
  Redirects, 2-MB-Limit, HTML→Text) mit Tests T1–T10 und S1–S16. Gates: `cargo fmt --check`
  sauber, `cargo test` (152 bestanden, 1 ignoriert), `npm run build` und `npm run test:ui`
  (30 bestanden, unverändert) grün. Mutationsnachweise M1 (T4 rot), M2 (S4/S5/S6 rot),
  M3 (S10 rot) in `/tmp/yts-mut-2a`. Bericht: `.herd/impl-2a-bericht.md`. Etappe 2b
  (Schleife L1–L6, `websearch.json`, Einstellungs-Tab, Chat-UI für Tool-Aktivität) folgt.
  Kein Dev-Server oder Tauri-Prozess gestartet.

- 2026-09-19: Abnahme Etappe 1 Video-Chat. Nativer Durchlauf mit `npm run tauri dev`
  (isoliertes `XDG_DATA_HOME` mit Kopie von DB und Konfiguration, danach gelöscht) über
  die Automation-API gegen OpenRouter `deepseek/deepseek-v4.1-flash`: Antwort mit
  Zeitstempeln aus dem Transkript, Folgerunde kennt den Verlauf, Fehlerwortlaute für
  fremde Chat-ID und leere Frage, nach Neustart Chat mit 4 Nachrichten vorhanden.
  Nicht nativ geprüft: Chat-Tab selbst (Streaming-Events, Stopp). Dev-App beendet.
- 2026-09-19: Video-Chat Korrekturpaket 2 (Nachprüfung zu Etappe 1): Entwürfe gehören
  jetzt zum verlassenen Chat (`stashChatDraft`/`restoreChatDraft`, kein Löschen beim
  Einsetzen, Voranstellen bei Fehlschlag im unsichtbaren Kontext), die Chat-Auswahl folgt
  auch einem im Hintergrund fertig gewordenen Chat (N3), Fokus kehrt nach Abschluss in
  `#chatInput` zurück (N4), `forgetChatState`/`deleteChat` räumen Auswahl und Entwürfe auf
  (N2), D14 kommt ohne Timing aus (N5). Neue UI-Fälle U22–U24, U1 prüft den Fokus.
  Gates: `cargo fmt --check` sauber, `cargo test` (124 bestanden, 1 ignoriert),
  `npm run build` und `npm run test:ui` (30 bestanden) grün, `npm run tauri -- build`
  erfolgreich. Mutationsbelege im Bericht: `.herd/impl-1-korrekturen-2-bericht.md`.
  Kein Dev-Server oder Tauri-Prozess gestartet; `sudo dpkg -i youtube-summarizer.deb`
  steht weiterhin beim Maintainer aus.
- 2026-09-19: Video-Chat Korrekturpaket 1 (Reviews Grok/Gemini): chatSelection,
  Render-Zähler nur für den sichtbaren Chat, Status/Entwürfe/Input-Sperre, eigene
  CSS-Klassen (`chat-row…`), Listener-Rejection abgefangen, Abbruchprüfung vor dem
  Speichern (R1), camelCase im Automation-Body (R2), U15–U21 und verschärfte Tests.
  Bericht: `.herd/impl-1-korrekturen-bericht.md`.

- 2026-09-19: Video-Chat Etappe 1 komplett. Korrekturen an 1a (K1: `ChatTurnResult.messages`
  ist jetzt der vollständige Verlauf nach der Runde; K2: `request_id` aus reinem Whitespace
  wird abgelehnt, Tests `d13_second_turn_returns_the_full_history` und
  `k2_request_id_with_only_whitespace_is_invalid`). Etappe 1b: neuer Chat-Tab (`src/chat.ts`,
  Template/CSS/Typen/State, `renderMarkdownInto` in `summary-view.ts`, `detail.ts`-Hooks,
  `bindChatEvents`), UI-Mock um `chat_*` und Video 3 erweitert, `tests/ui/chat.test.mjs` mit
  U1–U14. Gates: `cargo fmt --check` sauber, `cargo test` (121 bestanden, 1 ignoriert),
  `npm run build` und `npm run test:ui` (20 bestanden) grün, `npm run tauri -- build`
  erfolgreich (deb/rpm/AppImage, Symlinks aktualisiert). Bericht:
  `.herd/impl-1b-bericht.md`. Installation per `sudo dpkg -i youtube-summarizer.deb` steht
  aus; kein Dev-Server oder Tauri-Prozess gestartet.
- 2026-09-19: Video-Chat Etappe 1a (Backend) umgesetzt: `chats`/`chat_messages`
  in `storage.rs` (inkl. `append_chat_turn` in einer Transaktion), Modelle
  (`Chat`, `ChatMessageRecord`, `NewChatMessage`, `ChatTurnResult`, Serde
  camelCase), neues Modul `src-tauri/src/chat.rs` (`build_chat_messages`,
  `chat_send_impl`, `ChatRuns`-Laufregister, Commands `chat_list`/`chat_messages`/
  `chat_delete`/`chat_send`/`chat_cancel`, Event `ai:chat_stream`) und die drei
  Automation-Endpunkte. Tests P1–P10 und D1–D12 in `src-tauri/src/chat/tests.rs`,
  Provider-Seite über einen lokalen Test-HTTP-Server. `cargo fmt` und `cargo test`
  (119 bestanden, 1 ignoriert) grün; Mutationsnachweis P4/P7 in einer Kopie unter
  `/tmp/yts-mut-1a` (beide rot, wenn der Verlauf nicht in `extra_parts` einfließt).
  Bericht: `.herd/impl-1a-bericht.md`. Etappe 1b (Frontend) und der Release-Build
  stehen noch aus; kein Dev-Server oder Tauri-Prozess gestartet.

- 2026-09-19: Windows-Befunde vom 2026-09-11 umgesetzt (auf Linux, unter Windows
  noch nicht gegengeprüft): UI-Harness sucht Chrome/Edge/Chromium pro Plattform
  (`CHROMIUM_PATH` hat Vorrang) und weicht bei belegtem Vite-Port aus, daher läuft
  `test:ui` wieder parallel ohne `--test-concurrency=1`; `.gitattributes` mit
  `* text=auto eol=lf` (Renormalisierung ohne Änderungen); `.lnk`-Shortcut ignoriert;
  Windows-Abschnitt in `AGENTS.md`. Abweichend vom Befund wird
  `src-tauri/gen/schemas/` nicht mehr versioniert statt `windows-schema.json`
  einzuchecken: Die Dateien erzeugt jeder Build neu, das Tauri-Template ignoriert
  sie ebenfalls. `npm run test:ui` dreimal in Folge grün (6 bestanden).
- 2026-09-19: Mermaid-Flowcharts zeigten leere Kästen: Mermaid legt Labels als
  HTML in `<foreignObject>` ab, das SVG-Profil von DOMPurify entfernt diese.
  Fix in `getMermaid()` (`src/summary-view.ts`): `htmlLabels: false` (native
  SVG-Texte) plus `flowchart.wrappingWidth: 320` gegen Umbruch mitten im Wort.
  Neuer Regressionstest `tests/ui/summary-mermaid.test.mjs` (vor Fix rot, nach
  Fix grün); `test:ui` läuft jetzt mit `--test-concurrency=1`, weil alle
  Testdateien denselben festen Vite-Port nutzen. `npm run build`,
  `npm run test:ui` (6 bestanden) und `npm run tauri -- build` grün; kein
  Rust-Code geändert, `cargo test` nicht erneut gelaufen. Installation per
  `sudo dpkg -i youtube-summarizer.deb` steht noch aus. Kein Dev-Server aktiv.
- 2026-09-11 (Windows 11): `origin/main` per Fast-Forward auf 80bd721 gezogen, `npm run build`,
  `npm run tauri -- build` (NSIS-Setup 5,4 MB und MSI 7,3 MB) und `cargo test` (92 bestanden,
  1 Netzwerktest ignoriert) grün. `npm run test:ui` nur mit
  `CHROMIUM_PATH` auf das lokale Chrome grün (5 bestanden), siehe Windows-TODOs. Kein Dev-Server aktiv.
- 2026-09-06: Etappe 3 umgesetzt (Reines Refactoring der Modulgrenzen ohne
  Verhaltensänderung). Backend: `migration.rs` für Alt-Konfig-Migration,
  `summarize.rs` für Zusammenfassungslogik und Tests aus `commands.rs`
  ausgelagert. Frontend: `main.ts` auf 140 Zeilen Bootstrap
  reduziert (vorher rund 2.150), Domänenlogik in `types.ts`, `utils.ts`, `template.ts`, `state.ts`,
  `summary-view.ts`, `summary-dialog.ts`, `detail.ts` und `library.ts` aufgeteilt.
  Kein Feature-Modul über 600 Zeilen, kein DOM-Zugriff auf Modulebene.
  `cargo fmt`, `cargo test` (94 bestanden, 1 Netzwerktest ignoriert),
  `npm run build` und `npm run test:ui` (5 bestanden) grün. Kein Dev-Server aktiv.
  Release-Build (`npm run tauri -- build`) danach erfolgreich, `.deb` über
  den Projekt-Symlink installierbar.
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
