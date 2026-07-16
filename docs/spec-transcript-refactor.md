# Spec: Refactoring Transcript-/Metadaten-Abruf (`youtube.rs`)

## Ziel

Den YouTube-Abruf robuster, effizienter und ehrlicher in den Fehlermeldungen
machen. **Keine Änderungen an der äußeren API**: Tauri-Command-Signaturen,
Storage-Format (Transcript-JSON mit `text`/`start`/`time`, Chapters-JSON) und
Frontend bleiben unverändert.

Betroffene Dateien: `src-tauri/src/youtube.rs`, `src-tauri/src/commands.rs`
(nur Orchestrierung in `add_video_impl` / `refresh_transcript_impl` /
`http_client`), zugehörige Tests.

## Hintergrund

Aktueller Ablauf beim Hinzufügen eines Videos lädt die Watch-HTML (~1,5 MB)
bis zu **dreimal** (Publish-Date, Innertube-API-Key, Chapters). Der
API-Key-Fetch ist überflüssig (der `key`-Query-Param wird von
`youtubei/v1/player` seit 2025 ignoriert; yt-dlp hat ihn entfernt). Die
Track-Auswahl ignoriert `kind == "asr"`, `playabilityStatus` wird nicht
geprüft, und `with_json3_format` re-encodiert die signierte Caption-URL.

## Anforderungen

### A1 — Innertube-Call ohne Watch-HTML und ohne API-Key

- `fetch_transcript` lädt die Watch-Seite **nicht mehr**.
- `fetch_innertube_player` ruft `https://www.youtube.com/youtubei/v1/player`
  **ohne** `key`-Query-Parameter auf (POST-Body unverändert: ANDROID-Client,
  `clientVersion` 20.10.38).
- Beim Innertube-Request einen zum deklarierten Client passenden User-Agent
  als Request-Header setzen:
  `com.google.android.youtube/20.10.38 (Linux; U; Android 14) gzip`.
  Der globale Client-UA (`http_client()`) bleibt für alle anderen Requests
  unverändert.
- `extract_innertube_api_key` inklusive Regex entfernen.

### A2 — `playabilityStatus` prüfen

- Nach der Player-Response zuerst `/playabilityStatus/status` auswerten.
- Ist der Status vorhanden und ≠ `OK`, Fehler zurückgeben, der Status und —
  falls vorhanden — `/playabilityStatus/reason` enthält, z. B.:
  `"Video nicht abrufbar (LOGIN_REQUIRED): Melde dich an, um dein Alter zu bestätigen"`.
- Erst danach greift die bestehende Meldung „Für dieses Video wurde kein
  Transkript gefunden", wenn `captionTracks` fehlt/leer ist.

### A3 — Track-Auswahl: manuelle Untertitel vor ASR

- `select_caption_track` neu: Für jede Sprache der `LANGUAGES`-Prioritätsliste
  zuerst einen Track **ohne** `"kind": "asr"` suchen, dann einen mit `asr`.
- Globaler Fallback (keine Sprache matcht): erster manueller Track der Liste,
  sonst `tracks.first()`.
- Unit-Tests mit Inline-JSON-Fixtures: (a) manuell schlägt ASR bei gleicher
  Sprache, (b) Sprachpriorität schlägt Manuell-vs-ASR (de-ASR gewinnt gegen
  en-manuell), (c) Fallback ohne Sprach-Match bevorzugt manuellen Track.

### A4 — `fmt=json3` ohne Re-Encoding der signierten URL

- Die `baseUrl` aus `captionTracks` ist signiert (`sig`/`sparams`/`pot`);
  ein Neuaufbau der Query über `url::Url::query_pairs_mut` kann
  Percent-Encoding verändern und die Signatur brechen.
- `with_json3_format` so umbauen, dass die vorhandene Query **byte-identisch**
  erhalten bleibt: ein vorhandenes `fmt=<wert>`-Segment per String-Operation
  entfernen bzw. ersetzen, andernfalls `&fmt=json3` (bzw. `?fmt=json3` bei
  leerer Query) anhängen.
- Bestehenden Test `json3_format_replaces_existing_fmt` beibehalten/anpassen;
  neuer Test: eine URL mit percent-encodeten Parametern (z. B.
  `sparams=ip%2Cipbits`) bleibt außer dem `fmt`-Teil byte-identisch.

### A5 — Watch-HTML höchstens einmal pro Flow

- Publish-Date und Chapters brauchen weiterhin die Watch-HTML. Umbau in reine
  Parser + eine Fetch-Funktion, z. B.:
  - `fetch_watch_html(client, video_id) -> AppResult<String>` (existiert),
  - `publish_date_from_html(&str) -> Option<String>`,
  - `chapters_from_html(&str) -> Option<String>`.
- `add_video_impl`: Watch-HTML **genau einmal** laden; Publish-Date und
  Chapters aus derselben HTML parsen. `fetch_transcript` braucht keine HTML
  mehr (A1). Netto: 1× Watch-HTML statt 3×.
- `refresh_transcript_impl`: 1× Watch-HTML (nur für Chapters).
- **Fehlersemantik unverändert:** Scheitert der Watch-HTML-Abruf, bleiben
  `published_at` und `chapters` `None`; `add_video` scheitert dadurch nicht
  hart. Oembed (Titel) bleibt wie bisher harter Fehler. Transcript bleibt in
  `add_video_impl` `.ok()` (weich), in `refresh_transcript_impl` hart.
- `fetch_video_info` entsprechend anpassen (oembed separat; Publish-Date aus
  der geteilten HTML). Signatur-Änderungen innerhalb des Crates sind okay.

## Out of Scope (nicht umsetzen, in TODO.md als Ideen notieren)

- Übersetzungs-Fallback via `tlang` für `isTranslatable`-Tracks.
- Fallback-Kette über weitere Innertube-Clients (WEB, TV_EMBEDDED) oder yt-dlp.

## Gates (alle müssen grün sein)

Aus `src-tauri/`:

1. `cargo fmt` (danach `git diff` sauber formatiert)
2. `cargo test`
3. Netzwerk-Test: `cargo test fetches_transcript_from_innertube_caption_url -- --ignored`
   (muss mit dem neuen key-losen Innertube-Call bestehen)

Aus dem Repo-Root:

4. `npm run build` (AGENTS.md-Regel bei Transcript-Änderungen)

## Arbeitsregeln

- **Nicht committen.**
- TODO.md gemäß Session-Handoff-Regel aktualisieren (erledigte Arbeit,
  Out-of-Scope-Ideen, Testergebnisse).
- Kurze Implementierungs-Zusammenfassung (max. ~20 Zeilen: was geändert,
  welche Entscheidungen, Gate-Ergebnisse) nach
  `docs/impl-notes-transcript-refactor.md` schreiben.
