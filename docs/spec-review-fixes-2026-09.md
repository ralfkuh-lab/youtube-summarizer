# Spec: Abarbeitung des Code-Reviews vom 2026-09-06

Bezug: [Befunde und Abhilfen](code-review-2026-09-06.md). Alle fünf Befunde
wurden am Code bestätigt. Die Umsetzung läuft in drei Etappen, jede Etappe
wird separat implementiert, kreuz-reviewt und abgenommen.

| Etappe | Inhalt | Befunde |
|---|---|---|
| 1 | Race Conditions, Standardmodell-Validierung, Stream-Abschluss, UI-Testharness | 1, 5, 3 |
| 2 | Eine Konfigurationsquelle, Commit-nach-Save, gebündelte Sammlungsabfrage, schlanke Listenobjekte | 2, 4 |
| 3 | Modulgrenzen in `src/main.ts` und `commands.rs` | Refactoring |

Allgemeine Regeln für alle Etappen:

- Bestehende Muster weiterverwenden, keine neuen Frameworks außer den hier
  ausdrücklich genannten Dev-Abhängigkeiten.
- Alle Nutzertexte auf Deutsch, wie im übrigen Code.
- Gates: aus `src-tauri/` `cargo fmt` und `cargo test`, aus dem Repo-Root
  `npm run build`; ab Etappe 1 zusätzlich `npm run test:ui`.
- Nicht committen; das macht der Orchestrator nach der Abnahme.

---

## Etappe 1

### 1.1 Race Conditions beim Videowechsel (Befund 1)

Betroffen: `selectVideo()` und `refreshActiveTranscript()` in `src/main.ts`.
Der Klick auf einen Listeneintrag ruft `selectVideo()` ohne `busy`-Prüfung
auf, ein Wechsel ist also jederzeit möglich.

Regel: **Nach jedem `await` darf die Detailansicht nur dann aktualisiert
werden, wenn das betroffene Video noch das aktive ist** (`activeVideoId ===
id`). Das Datenmodell `videos` wird dagegen immer aktualisiert, denn die
geladenen Daten sind unabhängig von der Auswahl gültig.

Konkret:

- `selectVideo(id)`: nach dem `await` von `get_video_detail` wird `videos`
  aktualisiert. `showDetail(video)` nur, wenn `activeVideoId === id`. Auch
  `setStatus(errorMessage(error))` im Fehlerfall nur, wenn das Video noch aktiv
  ist. `renderVideoList()` nach der Aktualisierung von `videos` aufrufen, damit
  Status-Chips des nachgeladenen Videos stimmen.
- `refreshActiveTranscript()`: im Erfolgspfad `videos` aktualisieren und
  `renderVideoList()` aufrufen; `showDetail(updated)` und
  `switchTab("transcript")` nur, wenn `activeVideoId === video.id`. Die
  Statuszeile „Transkript geladen" bleibt in jedem Fall (sie beschreibt den
  abgeschlossenen Vorgang, nicht die Auswahl). Der Fehlerpfad prüft bereits
  korrekt und bleibt unverändert.
- Keine Änderung an `summarizeVideo()`: der Stream-Handler prüft schon
  `streamingVideoId` und das aktive Video.

Keine neue Abstraktion (kein Request-Token-Helfer): die beiden Guards sind
ausreichend und lesbar.

### 1.2 UI-Testharness und Regressionstests

Es gibt bisher keinen Test-Runner für das Frontend. Für asynchrone UI-Zustände
wird ein schlanker Browser-Harness eingeführt, der auch in Etappe 3 als
Sicherheitsnetz dient.

Aufbau:

- Neue Dev-Abhängigkeit: `playwright-core` (kein Browser-Download). Es wird
  das System-Chromium benutzt, Pfad `/usr/bin/chromium`, überschreibbar per
  Umgebungsvariable `CHROMIUM_PATH`.
- Test-Runner: Nodes eingebauter `node --test` (kein vitest, kein jest).
  Neues npm-Skript `"test:ui": "node --test tests/ui/*.test.mjs"`.
- Dateien:
  - `tests/ui/harness.mjs`: startet den Vite-Dev-Server programmatisch
    (`createServer` aus `vite`, `server.listen()`, fester Port `5199` mit
    `strictPort: true`, überschreibbar per `UI_TEST_PORT`), startet Chromium
    headless über `playwright-core`, installiert per `page.addInitScript()`
    den Tauri-Mock **vor** dem App-Code und stellt `withApp(async (page, mock)
    => …)` bereit, das Server und Browser sauber wieder schließt.
  - `tests/ui/tauri-mock.mjs`: der Mock als String oder Funktion für
    `addInitScript`. Er setzt `window.__TAURI_INTERNALS__ = { invoke,
    transformCallback, metadata }`. `@tauri-apps/api/core` ruft zur Laufzeit
    genau `window.__TAURI_INTERNALS__.invoke(cmd, args, options)` auf;
    `listen()` aus `@tauri-apps/api/event` ruft `invoke("plugin:event|listen",
    …)` und `transformCallback()`. Der Mock muss daher mindestens bedienen:
    `plugin:event|listen` und `plugin:event|unlisten` (Dummy-IDs zurückgeben),
    `plugin:opener|open_url` (no-op), `get_videos`, `get_collections`,
    `ai_config_get`, `ai_catalog_get`, `get_video_detail`,
    `refresh_transcript`, und für alles Unbekannte einen aussagekräftigen
    Fehler werfen, damit Tests nicht still hängen. `transformCallback(cb)`
    registriert `cb` unter einer Zahl und gibt die Zahl zurück.
  - Der Mock ist steuerbar: Fixtures (Videos, Collections, Konfiguration)
    werden vom Test übergeben; pro Command und Video-ID lässt sich eine
    Verzögerung in Millisekunden setzen (`delays: { get_video_detail: { 1:
    300, 2: 20 } }`); der Mock protokolliert alle Aufrufe
    (`window.__tauriMock.calls`) und zählt offene Aufrufe
    (`window.__tauriMock.pending`), damit Tests per `page.waitForFunction()`
    auf „alles beantwortet" warten können statt auf feste Zeiten.
  - `tests/ui/video-switch.test.mjs` mit zwei Tests.
- Fixture-Daten: mindestens zwei Videos mit unterschiedlichen Titeln,
  Video 1 ohne Transkript, Video 2 mit Transkript; eine leere
  Collection-Liste; eine minimale KI-Konfiguration, wie sie `ai_config_get`
  liefert (Struktur aus `src/main.ts` bzw. `src/ai-config.ts` übernehmen).

Die zwei Tests:

1. **Vertauschte Antwortreihenfolge**: Video 1 anklicken, sofort Video 2
   anklicken; `get_video_detail` für 1 antwortet nach 300 ms, für 2 nach
   20 ms. Erwartung nach Abschluss aller Aufrufe: `#detailTitle` zeigt den
   Titel von Video 2, der Listeneintrag von Video 2 trägt die Klasse `active`,
   der von Video 1 nicht.
2. **Videowechsel während Transkript-Neuladen**: Video 1 auswählen (schnell),
   dann „Transkript laden" klicken (`refresh_transcript` antwortet nach 300 ms
   mit Video 1 inklusive Transkript), währenddessen Video 2 anklicken.
   Erwartung nach Abschluss: `#detailTitle` zeigt Video 2, der Transkript-Tab
   wurde nicht aktiviert (aktiver Tab bleibt der Default), und der
   Listeneintrag von Video 1 zeigt den Transkript-Chip als vorhanden
   (`videos` wurde aktualisiert).

Beide Tests müssen **vor** dem Fix rot und **nach** dem Fix grün sein; das ist
in der Zusammenfassung mit dem jeweiligen Output zu belegen (Fix zunächst
auskommentieren oder per `git stash` prüfen).

Dokumentation: `AGENTS.md` unter „Commands" um `npm run test:ui` ergänzen,
mit dem Hinweis auf System-Chromium und `CHROMIUM_PATH`. README nur, falls
dort Testkommandos gelistet sind.

### 1.3 Standardmodell wird genauso validiert (Befund 5)

`resolve_summary_model()` in `src-tauri/src/commands.rs`: Das Ergebnis des
Default-Pfads (`ai.default_model`) durchläuft dieselben drei Prüfungen wie die
explizite Auswahl (Provider vorhanden, Provider aktiviert, Modell in der
Whitelist). Die Funktion merkt sich, ob die Auswahl aus dem Default stammt,
und formuliert die Fehlertexte dann so, dass der Nutzer weiß, was zu tun ist:

- Provider fehlt oder deaktiviert:
  `Standardmodell '{model}' von '{provider}' ist nicht mehr verfügbar, weil der Anbieter fehlt oder deaktiviert ist - bitte in den Einstellungen ein KI-Modell auswählen`
- Modell nicht in der Whitelist:
  `Standardmodell '{model}' von '{provider}' ist nicht mehr aktiviert - bitte in den Einstellungen ein KI-Modell auswählen`

Die bestehenden Texte für die explizite Auswahl bleiben unverändert.

Tests (im bestehenden `mod tests` von `commands.rs`, Muster
`config_with_enabled_model()` weiterverwenden):

- Default-Modell, Provider deaktiviert → Fehler mit dem Text oben.
- Default-Modell, Modell aus der Whitelist entfernt → Fehler mit dem Text oben.
- Default-Modell, Provider gelöscht → Fehler.
- Bestehender Test „Default-Modell gültig → Ok" bleibt grün.

### 1.4 Unvollständige Streams erkennen (Befund 3)

`chat_stream_cancellable()` in `src-tauri/src/ai/client.rs`.

Definition „abgeschlossen": Der Stream gilt als vollständig, wenn entweder das
Ereignis `[DONE]` empfangen wurde oder ein Chunk in `choices[0]` ein
nicht-leeres `finish_reason` ungleich `"length"` trug (`"length"` bleibt wie
bisher `TruncatedOutput`). Nicht jeder OpenAI-kompatible Provider sendet
`[DONE]`, daher zählt `finish_reason` gleichwertig.

Verhalten:

- Endet der Byte-Stream (`stream.next()` liefert `None`), ohne dass ein
  Abschluss gesehen wurde, ist das Ergebnis ein Fehler, **auch wenn schon
  Text angekommen ist**. Neue Variante `ChatError::IncompleteStream` mit dem
  Text: `Die KI-Antwort endete vorzeitig ohne Abschlusssignal des Providers, das Ergebnis ist unvollständig - bitte erneut versuchen`.
- Endet der Stream regulär abgeschlossen, aber ohne Text, bleibt es
  `MissingChoice`.
- `[DONE]` beendet den Stream weiterhin sofort. Ein `finish_reason` beendet
  den Stream **nicht** vorzeitig, es setzt nur die Abschluss-Markierung; das
  Lesen läuft bis zum Stream-Ende oder `[DONE]` weiter, damit nachfolgende
  Chunks (etwa `usage`-Events) nichts kaputt machen.
- Der nicht-streamende Pfad (`parse_chat_response`) bleibt unverändert.
- Eine Zusammenfassung mit `IncompleteStream` darf **nicht** gespeichert
  werden; das ergibt sich aus dem `?` in `summarize_video_impl`, ist aber zu
  verifizieren.

Tests (Muster der bestehenden Tests mit lokalem `TcpListener` verwenden):

- Stream liefert zwei Text-Chunks und schließt die Verbindung ohne `[DONE]`
  und ohne `finish_reason` → `Err(ChatError::IncompleteStream)`.
- Stream liefert Text-Chunks, dann ein Chunk mit `finish_reason: "stop"`,
  dann Verbindungsende ohne `[DONE]` → `Ok(text)`.
- Stream liefert Text, dann `[DONE]` → `Ok(text)` (bestehender Fall, falls
  nicht schon getestet).

---

## Etappe 2

### 2.1 Eine Konfigurationsquelle für die Zusammenfassung (Befund 2, Teil A)

`summarize_video_impl()` lädt heute `AiConfigService::load(paths)` und
`AuthStore::load(paths)` erneut von Platte, während die Settings-Commands auf
dem verwalteten Tauri-State (`Mutex<AiConfigService>`, `Mutex<AuthStore>`)
arbeiten.

Zielbild: **`summarize_video_impl()` kennt keine Konfigurationsspeicher
mehr.** Der Aufrufer löst das Ziel vorher auf und übergibt es.

- Neuer Typ in `commands.rs`:
  `pub struct SummaryTarget { pub provider: String, pub model: String, pub base_url: String, pub api_key: Option<String> }`.
- Neue Funktion
  `pub fn resolve_summary_target(ai: &AiConfig, catalog: &Catalog, provider_id: Option<String>, model_id: Option<String>) -> AppResult<(AiModelRef, String)>`
  (Modell-Referenz plus Basis-URL), die `resolve_summary_model()` und
  `provider_base_url()` zusammenführt. Der Schlüssel wird vom Aufrufer
  nachgeschlagen, weil die Automation-API keinen verwalteten State hat.
- `summarize_video_impl(paths, http, id, system_prompt, target: SummaryTarget, timestamps, options, on_delta)`.
- Tauri-Command `summarize_video`: bekommt zusätzlich `cfg:
  State<'_, Mutex<AiConfigService>>` und `auth: State<'_, Mutex<AuthStore>>`,
  liest die Konfiguration per `ai_config_data_from_state()`, den Katalog wie
  bisher, den Schlüssel per `lock_ai_auth_from_state()`. **Der Mutex-Guard
  muss vor dem `await` freigegeben sein** (eigener Block), sonst ist die
  Future nicht `Send`.
- Automation-API (`automation.rs`, debug-only, ohne verwalteten State): löst
  das Ziel über `AiConfigService::load(paths).data()` und
  `AuthStore::load(paths)` auf, wie es dort für `GET /api/ai/config` schon
  üblich ist. Das ist bewusst so und in einem Kommentar zu vermerken.
- Bekannte Verhaltensänderung: Konfigurationsfehler (kein Modell, Provider
  deaktiviert) werden jetzt **vor** dem Laden des Videos gemeldet. Das ist
  akzeptiert; das Frontend öffnet den Dialog ohnehin nur bei vorhandenem
  Transkript.

### 2.2 Setter übernehmen Änderungen erst nach erfolgreichem Speichern (Befund 2, Teil B)

`AiConfigService` in `src-tauri/src/ai/config.rs` und `AuthStore` in
`src-tauri/src/ai/auth.rs`: Alle mutierenden Methoden (`provider_enable`,
`model_toggle`, `custom_upsert`, `custom_delete`, `custom_models_replace`,
`default_model_set`; `set`, `remove`) arbeiten auf einer **Kopie** des
Zustands, speichern die Kopie und übernehmen sie erst bei Erfolg:

```rust
fn commit(&mut self, next: AiConfig) -> Result<(), AiConfigError> {
    save_to(&self.path, &next)?;
    self.data = next;
    Ok(())
}
```

Die Idempotenz-Kurzschlüsse („nichts zu tun") bleiben, sie beziehen sich dann
auf den zuletzt **erfolgreich gespeicherten** Zustand. Damit ist auch der
zweite Teil des Befunds erledigt: ein wiederholter Setter-Aufruf nach einem
Schreibfehler versucht wieder zu speichern.

Tests ohne Mocks: `load_from()` mit einem Pfad, dessen Elternverzeichnis eine
**reguläre Datei** ist (Schreiben scheitert deterministisch mit I/O-Fehler).
Je Store ein Test:

- Setter liefert `Err`, `data()` bzw. `status()` zeigt den alten Zustand.
- Derselbe Setter-Aufruf ein zweites Mal liefert erneut `Err` (kein stiller
  `Ok` wegen vermeintlich unveränderten Zustands).

### 2.3 Sammlungszuordnungen gebündelt laden (Befund 4, Teil A)

`hydrate_video_collections()` in `src-tauri/src/storage.rs` führt eine
Abfrage pro Video aus. Ersetzen durch **eine** Abfrage
`SELECT video_id, collection_id FROM video_collections ORDER BY video_id, collection_id`,
gruppiert in eine `HashMap<i64, Vec<i64>>`, danach zuweisen. Videos ohne
Zuordnung bekommen ein leeres Vec. Ein bestehender oder neuer Test in
`storage.rs` belegt, dass die Zuordnung mit mehreren Videos und Sammlungen
korrekt ist (Sortierung der IDs beibehalten).

### 2.4 Schlanke Listenobjekte (Befund 4, Teil B)

`get_videos()` liefert heute jede Zeile mit Transkript, Zusammenfassung,
Beschreibung und Kapiteln. Das Frontend braucht in der Liste nur Metadaten
und **Vorhanden-Flags**; die Inhalte werden beim Auswählen ohnehin per
`get_video_detail` nachgeladen (`selectVideo()` ersetzt den Eintrag in
`videos` durch das vollständige Objekt). `get_videos` wird nur beim
App-Start aufgerufen, wenn noch kein Video aktiv ist.

Backend (`storage.rs`):

- `Video` bekommt zwei neue Felder `has_transcript: bool` und
  `has_summary: bool`. Sie werden in SQL berechnet:
  `(transcript IS NOT NULL AND transcript != '') AS has_transcript`,
  analog `has_summary`. `row_to_video` liest sie aus der Zeile.
- Zwei Spaltenlisten: `VIDEO_COLUMNS` (Detail, wie bisher plus die Flags) und
  `VIDEO_LIST_COLUMNS`, die statt der Inhalte `NULL AS transcript, NULL AS
  summary, NULL AS description, NULL AS chapters` liefert; Thumbnail-Daten
  bleiben in der Liste enthalten, sie werden dort angezeigt.
- `get_videos()` benutzt `VIDEO_LIST_COLUMNS`. Alle Einzelvideo-Funktionen
  (`get_video`, Rückgaben von `add_video`, `refresh_transcript`,
  `summarize_video`, Collection-Zuordnungen usw.) liefern weiterhin das
  vollständige Objekt. Invariante: **Nur `get_videos` liefert schlanke
  Objekte.**
- Automation-API `GET /api/videos` liefert damit ebenfalls die schlanken
  Objekte; der Punkt „compact video objects" in `TODO.md` ist damit erledigt
  und wird dort entfernt.

Frontend (`src/main.ts`):

- Typ `Video` um `has_transcript: boolean` und `has_summary: boolean`
  erweitern.
- Jede **Status**-Verwendung von `video.transcript` / `video.summary`
  (Status-Chips in `renderVideoList`, Listenfilter, Beschriftung des
  Transkript-Buttons, Statuszeile nach `addVideo`, Provider/Modell-Anzeige im
  Detail) wechselt auf die Flags.
- **Inhalts**-Verwendungen (`renderTranscript`, Zusammenfassung rendern,
  Beschreibung, Vorbedingung im Summarize-Dialog) bleiben auf den
  Inhaltsfeldern; sie sind nur auf Objekten gültig, die aus
  `get_video_detail` oder einem Einzelvideo-Command stammen. Prüfen, dass
  jeder solche Pfad über `getActiveVideo()` läuft und das aktive Video stets
  über `selectVideo()` hydriert wurde.
- UI-Test-Fixtures um die Flags ergänzen; ein dritter UI-Test prüft, dass
  die Liste mit schlanken Objekten (ohne Inhaltsfelder) die Chips „T" und „Z"
  korrekt aus den Flags rendert und der Filter „mit Transkript" funktioniert.

Dokumentation: `TODO.md` (Punkt „compact video objects" entfernen, Etappe als
erledigt markieren), `AGENTS.md` nur bei geänderten Kommandos.

---

## Etappe 3: Modulgrenzen

Reines Verschieben ohne Verhaltensänderung. Sicherheitsnetz: `cargo test`,
`npm run build` (tsc mit strikten Typen) und `npm run test:ui`. Vor dem
Start `git diff --stat` leer, danach müssen alle drei Gates unverändert grün
sein. Kein Feature, kein Bugfix, keine Umbenennung von Nutzertexten in dieser
Etappe; wer beim Verschieben einen Fehler entdeckt, notiert ihn in der
Zusammenfassung statt ihn nebenbei zu fixen.

### 3.1 Frontend: `src/main.ts` aufteilen

Vorbild ist das bestehende Muster in `src/ai-config.ts`: ein Modul hält
seinen Zustand selbst, bekommt Abhängigkeiten über eine `init…()`-Funktion
und bindet seine Events in einer `bind…Events()`-Funktion, die `main.ts`
nach dem Rendern des Templates aufruft.

Zwei Fallstricke, die die Aufteilung zwingend beachten muss:

1. **Keine DOM-Zugriffe auf Modulebene.** `import` wird gehoistet; der
   Modulrumpf eines importierten Moduls läuft, **bevor** `main.ts` das
   Template in `#app` schreibt. Alle `$("#…")`-Konstanten (heute Zeilen
   440 ff. in `main.ts`) wandern deshalb entweder in Funktionsrümpfe oder in
   eine `init…()`-Funktion des jeweiligen Moduls, die die Elemente in
   modul-lokale `let`-Variablen schreibt.
2. **Geteilter veränderlicher Zustand.** Ein importiertes `let` ist im
   Importeur nur lesbar. Der gemeinsame Zustand (`videos`, `collections`,
   `activeVideoId`, `activeCollectionId`, `activeTab`, `busy`,
   `videoSearchQuery`, `videoStatusFilter`, `streamingVideoId`,
   `summaryRenderGen`) wandert in ein exportiertes Objekt
   `state` in `src/state.ts`; Zugriffe werden zu `state.videos` usw. Zustand,
   den nur ein Modul benutzt (Preset-Editor, Mermaid-Loader, Summary-Historie),
   bleibt modul-lokal in diesem Modul.

Zielstruktur (Funktionsnamen wie heute in `main.ts`):

| Datei | Inhalt |
|---|---|
| `src/types.ts` | `Chapter`, `TranscriptSnippet`, `Video`, `Collection`, `TabName`, `SummaryRecord`, `SummaryPreset`, `SummaryModules`, `VideoStatusFilter`, `SummarySettings` |
| `src/template.ts` | `export const appTemplate = \`…\`` (heutiger `app.innerHTML`-String, unverändert) |
| `src/state.ts` | `state`-Objekt (siehe oben), `getActiveVideo()`, `setBusy()`, `setStatus()`, `initState({ statusEl })` für die DOM-Elemente, die `setBusy`/`setStatus` anfassen |
| `src/utils.ts` | `formatDate`, `normalizeSearch`, `isVideoStatusFilter`, `compareCollections`, `capitalize`, `coerceBool` |
| `src/library.ts` | Liste und Sammlungen: `renderVideoList`, `renderVideoFilters`, `getFilteredVideos`, `matchesActiveCollection`, `matchesVideoStatusFilter`, `matchesVideoSearch`, `renderVideoStatusChip`, `addVideo`, `selectVideo`, `deleteActiveVideo`, `renderCollectionList`, `renderCollectionItem`, `openCollectionDialog`, `saveCollection`, `deleteCollection`, `loadCollections`; `initLibrary()`, `bindLibraryEvents()` |
| `src/detail.ts` | Detailansicht: `showDetail`, `renderVideoCollections`, `updateActiveVideoCollections`, `transcriptErrorHint`, `renderTranscript`, `renderChapters`, `updateDetailSummaryMeta`, `seekVideo`, `buildYouTubeEmbedUrl`, `switchTab`, `detailParagraph`, `renderDescriptionHtml`, `renderDescriptionTextFragment`, `refreshActiveTranscript`, `hasPlayableVideoCodec`, `updateVideoCodecNotice`; `initDetail()`, `bindDetailEvents()` |
| `src/summary-view.ts` | Zusammenfassungs-Tab und Historie: `stripWrappingCodeFence`, `markdownToHtml`, `timestampSeconds`, `linkifySummaryTimestamps`, `replaceTimestampsInTextNode`, `getMermaid`, `removeMermaidArtifacts`, `renderMermaidBlocks`, `renderSummaryMarkdown`, `hideSummaryHistoryBar`, `formatSummaryHistoryTime`, `summaryHistoryLabel`, `fillSummaryHistorySelect`, `renderSummaryTab`, `showSummaryVersion`, `deleteDisplayedSummary`; `bindSummaryViewEvents()` |
| `src/summary-dialog.ts` | Dialog, Einstellungen, Presets, Start: `openSummaryDialog`, `defaultSummarySettings`, `parseSummaryModules`, `parseSummarySettings`, `readSummaryModules`, `applySummarySettings`, `loadSummarySettings`, `saveSummarySettings`, `parseModelValue`, `startSummary`, `selectedPresetId`, `selectedPresetPrompt`, `buildSummaryPrompt`, `updateSummaryPromptEditedBadge`, `recomposeSummaryPrompt`, `onSummaryPresetChange`, `loadSummaryPresets`, `fillPresetPicker`, `openPresetManager`, `renderPresetList`, `renderPresetRow`, `slugFromName`, `setPresetEditError`, `openPresetEditDialog`, `onPresetNameInput`, `savePresetEdit`, `deletePreset`; `bindSummaryDialogEvents()` |
| `src/main.ts` | Imports, `marked.setOptions`, Template rendern, `init…()`-Aufrufe, `bindEvents()` als Verdrahtung der `bind…Events()`, `isModalOpen`, `bindEscapeToCloseModals`, `loadInitialData` |

Regeln:

- `marked`, `DOMPurify` und `mermaid` werden nur dort importiert, wo sie
  benutzt werden (`summary-view.ts`).
- Zyklische Imports zwischen Feature-Modulen (etwa `library.selectVideo` →
  `detail.showDetail` → `library.renderVideoList`) sind erlaubt, solange auf
  Modulebene nichts aus dem Zyklus ausgewertet wird; genau deshalb Regel 1.
- `bindEvents()` in `main.ts` bleibt die einzige Stelle, die
  `addEventListener` für die Kopfzeile (URL-Eingabe, Hinzufügen, Suche,
  Filter, Einstellungen) setzt; Feature-Module binden nur ihre eigenen
  Elemente.
- Keine `export default`, benannte Exporte wie in `ai-config.ts`.
- `main.ts` soll danach unter 250 Zeilen liegen; kein Feature-Modul über
  600 Zeilen.
- `tests/ui/` bleibt unverändert grün; DOM-IDs und Klassen ändern sich nicht.

### 3.2 Backend: `commands.rs` entlasten

- `src-tauri/src/ai/migration.rs`: `ensure_migrated()` samt Helfern
  (`map_old_provider_id`, `strip_chat_suffix`) und deren Tests. Aufrufer
  (`lib.rs` bzw. wo heute `commands::ensure_migrated` benutzt wird) anpassen;
  Sichtbarkeit `pub(crate)`.
- `src-tauri/src/summarize.rs`: `SummaryTarget`, `resolve_summary_target`,
  `resolve_summary_model`, `build_summary_prompts`,
  `strip_wrapping_code_fence`, `summarize_video_impl` und deren Tests.
  `provider_base_url` bleibt in `commands.rs`, falls es dort auch von den
  Custom-Model-Commands gebraucht wird; sonst mitwandern.
- `commands.rs` behält die `#[tauri::command]`-Funktionen, die
  State-Helfer (`ai_config_data_from_state`, `mutate_ai_config_state`,
  `lock_ai_auth_from_state`) und die Custom-Model-Helfer.
- `automation.rs` ruft `summarize::summarize_video_impl` bzw.
  `summarize::resolve_summary_target` auf.
- `lib.rs`: neue Module registrieren. `cargo fmt` und `cargo test` grün,
  Testanzahl unverändert.

### 3.3 Dokumentation

- `AGENTS.md`, Abschnitt „Important Files": die neuen Frontend-Module und
  `summarize.rs`/`ai/migration.rs` aufnehmen, `src/main.ts` als Bootstrap
  beschreiben.
- `TODO.md`: Etappe 3 abhaken, „Last Verified State" ergänzen.
