# Impl-Notes: Transcript-/Metadaten-Refactor (`youtube.rs`)

Umsetzung gemäß `docs/spec-transcript-refactor.md`. Äußere API unverändert.

## Änderungen

- **A1**: `fetch_transcript` lädt keine Watch-HTML mehr; ruft `fetch_innertube_player`
  ohne `key`-Param auf und setzt den Client-passenden User-Agent-Header
  (`com.google.android.youtube/20.10.38 …`) nur für diesen Request.
  `extract_innertube_api_key` + Regex entfernt.
- **A2**: `playabilityStatus/status` wird geprüft; ≠ `OK` → Fehler mit Status und
  (falls vorhanden) `reason`. Danach erst die „kein Transkript"-Meldung.
- **A3**: `select_caption_track` neu — pro Sprache erst manueller Track, dann ASR;
  Fallback: erster manueller Track, sonst `tracks.first()`. 3 Unit-Tests.
- **A4**: `with_json3_format` per String-Op (split am ersten `?`, `fmt`-Segmente
  raus, `fmt=json3` anhängen) → signierte Query bleibt byte-identisch. 3 Tests.
- **A5**: `fetch_watch_html` (pub) + reine Parser `publish_date_from_html` /
  `chapters_from_html`. `add_video_impl`: 1× Watch-HTML (Publish-Date + Chapters),
  HTML-Fehler bleibt weich (beide `None`). `refresh_transcript_impl`: 1× HTML (Chapters).
  `fetch_video_info` nimmt jetzt `html: Option<&str>` für das Publish-Date.

## Entscheidungen

- `VideoInfo` beibehalten; Publish-Date wird per HTML-Parameter befüllt statt intern
  nachzuladen. `fetch_video_info` verliert die oembed/HTML-Parallelität (join), dafür
  netto 3×→1× Watch-HTML pro Flow.

## Gates (alle grün)

1. `cargo fmt` — sauber. 2. `cargo test` — 33 passed. 3. Netzwerk-Test
   `fetches_transcript_from_innertube_caption_url --ignored` — passed (key-loser Call ok).
   4. `npm run build` — ok.

Nicht committet.

## Review-Fixes (codex + agy, konsolidiert)

- **F1**: `with_json3_format` → `-> String`. Trennt zuerst ein `#fragment` ab
  (fmt landet vor dem Fragment), ersetzt `fmt`-Segmente in place statt umzusortieren,
  normalisiert nichts (leere Segmente `a&&b&` bleiben byte-identisch). Tests jetzt mit
  exaktem `assert_eq!` inkl. Mitte-Position, trailing `&`, `#fragment`, Percent-Encoding.
- **F2**: Leeres `captionTracks`-Array wird wie ein fehlendes behandelt (`.filter(!is_empty)`).
- **F3**: `add_video_impl` lädt oembed und Watch-HTML wieder parallel (`tokio::join!`);
  `fetch_video_info` ist oembed-only, Publish-Date wird im Command geparst. oembed hart,
  HTML weich — unverändert.
- **F4**: `refresh_transcript_impl` behält bei HTML-Fehler die bestehenden Kapitel
  (Re-Serialisierung von `video.chapters`) statt sie zu löschen.
- **F5**: `check_playability(&Value) -> Result<(), String>` extrahiert, 4 Unit-Tests.
- **F6**: `OnceLock<Regex>` für `extract_video_id` und `publish_date_from_html`
  (keine Kompilierung pro Aufruf/in Schleife mehr).
- **F7**: `select_caption_track` nutzt ein `find` mit kombinierter Bedingung
  (`same_language && !is_asr` bzw. `&& is_asr`).
- Abgelehnt: HTTP-Mocking / Request-Zählungs-Tests (kein Mocking eingeführt).

### Gates nach Fixes (alle grün)

1. `cargo fmt` — sauber. 2. `cargo test` — 39 passed, 1 ignored. 3. Netzwerk-Test
   `--ignored` — passed. 4. `npm run build` — ok. Nicht committet.
