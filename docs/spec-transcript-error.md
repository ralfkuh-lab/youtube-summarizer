# Spec: Transkript-Fehler sichtbar machen

Stand: 2026-09-05.

## Problem

`add_video_impl` in `src-tauri/src/commands.rs` verschluckt den Fehler von
`youtube::fetch_transcript` (`.ok()`), und das Frontend zeigt danach nur
„Video hinzugefügt, aber kein Transkript gefunden". Der Grund geht verloren.
Realer Fall (2026-07-16): YouTubes Bot-Check lehnt bekannte VPN-Exit-IPs
(z. B. Mullvad) ab, das Backend erzeugt den ehrlichen Fehler
`Video nicht abrufbar (LOGIN_REQUIRED): Sign in to confirm you're not a bot`,
der Nutzer sieht davon nichts. Für alte Videos ohne Transkript ist der Grund
ebenfalls nirgends nachzulesen.

## Ziel

Der Backend-Fehler wird pro Video **gespeichert** und an drei Stellen gezeigt:

1. Statuszeile direkt nach dem Hinzufügen.
2. Transkript-Tab der Detailseite (statt „Kein Transkript verfügbar"), bei
   `LOGIN_REQUIRED` mit VPN-Hinweis und Abhilfe.
3. Tooltip am „T"-Statuschip in der Videoliste.

Invariante: `transcript_error` ist nur gesetzt, solange `transcript` NULL ist.
Ein erfolgreich geladenes Transkript löscht den Fehler.

## Backend

### `src-tauri/src/models.rs`

- `Video.transcript_error: Option<String>` (serialisiert wie die übrigen
  Felder, kein Rename).
- `NewVideo.transcript_error: Option<String>`.

### `src-tauri/src/storage.rs`

- Neue Spalte `transcript_error TEXT` in `CREATE TABLE videos` **und**
  `ensure_video_column(&conn, "transcript_error", "TEXT")` neben der
  `description`-Migration.
- `VIDEO_COLUMNS`, `insert_video` und `row_to_video` um die Spalte ergänzen.
- `update_transcript` setzt zusätzlich `transcript_error = NULL`.
- Neue Funktion
  `set_transcript_error(paths: &AppPaths, id: i64, error: &str) -> AppResult<Video>`:
  `UPDATE videos SET transcript_error = ?1, updated_at = ?2 WHERE id = ?3`;
  Transkript, Kapitel und Beschreibung bleiben unangetastet. Gibt das
  aktualisierte Video zurück (Muster wie `update_transcript`).
- Tests im bestehenden `mod tests` (`temp_paths()`):
  - `insert_video` mit `transcript_error: Some(..)` → `get_video` liefert den
    Fehler zurück, `transcript` ist `None`.
  - `set_transcript_error` danach `update_transcript` → `transcript_error`
    ist `None`, Transkript gesetzt.
  - Migration: Datenbank ohne die Spalte anlegen (CREATE TABLE ohne
    `transcript_error`, wie ein Alt-Stand), dann `init_db` → Spalte existiert,
    `get_videos` funktioniert. Falls es ein solches Migrations-Testmuster
    bereits für `description` gibt, dasselbe Muster verwenden.

### `src-tauri/src/commands.rs`

- `add_video_impl`: statt `.ok()`
  ```rust
  let (transcript, transcript_error) = match youtube::fetch_transcript(&client, &video_id).await {
      Ok(transcript) => (Some(transcript), None),
      Err(error) => (None, Some(error)),
  };
  ```
  und beides in `NewVideo` übergeben. `add_video` bleibt erfolgreich, wenn
  nur das Transkript fehlt (wie heute).
- `refresh_transcript_impl`: schlägt `fetch_transcript` fehl, wird der Fehler
  **persistiert, wenn das Video noch kein Transkript hat**
  (`storage::set_transcript_error`), anschließend in **beiden** Fällen wie
  bisher als `Err(error)` zurückgegeben (die Statuszeile zeigt ihn weiter).
  Hat das Video bereits ein Transkript, wird nichts geschrieben (Invariante).
  Kapitel/Beschreibung werden bei Fehlschlag weiterhin nicht angefasst.
- `summarize_video_impl`: Fehlertext „Kein Transkript vorhanden - bitte Video
  neu hinzufügen" → „Kein Transkript vorhanden – bitte „Transkript laden“
  versuchen" (der Button existiert inzwischen; Neu-Hinzufügen ist der falsche
  Rat).
- Sonstige Aufrufer von `NewVideo` (Tests, Automation) mit
  `transcript_error: None` ergänzen.

### `src-tauri/src/automation.rs`

Keine Änderung nötig; die Video-JSON enthält das Feld automatisch.

## Frontend (`src/main.ts`, `src/styles.css`)

- Typ `Video`: `transcript_error?: string | null`.
- `addVideo()` Statusmeldung:
  - Transkript vorhanden → wie heute „Video hinzugefügt und Transkript geladen".
  - sonst mit `transcript_error` → `Video hinzugefügt, Transkript fehlgeschlagen: <Fehlertext>`.
  - sonst (Fallback) → heutiger Text „Video hinzugefügt, aber kein Transkript gefunden".
- Neue Hilfsfunktion `transcriptErrorHint(error: string): string | null`:
  - enthält der Fehler `LOGIN_REQUIRED` →
    „YouTube verlangt hier eine Anmeldung (Bot-Check). Das passiert typischerweise
    über bekannte VPN-Ausgangs-IPs, z. B. Mullvad. Abhilfe: die App außerhalb des
    VPN-Tunnels starten (unter Linux etwa mit mullvad-exclude) und dann
    „Transkript laden“ klicken."
  - sonst `null`. Die Funktion ist bewusst die eine Stelle für weitere
    Fehlerklassen.
- `renderTranscript(raw, chapters, error?)`: fehlt das Transkript und ist
  `error` gesetzt, statt `<p class="empty">Kein Transkript verfügbar</p>`:
  ```html
  <div class="transcript-error">
    <p class="transcript-error-title">Transkript konnte nicht geladen werden</p>
    <p class="transcript-error-message">{escapeHtml(error)}</p>
    <p class="transcript-error-hint">{escapeHtml(hint)}</p>   <!-- nur wenn hint -->
    <p class="transcript-error-retry">Erneut versuchen über „Transkript laden“.</p>
  </div>
  ```
  Ohne Fehler bleibt der heutige „Kein Transkript verfügbar"-Text.
- `renderVideoStatusChip`: bei fehlendem Transkript und gesetztem
  `transcript_error` lautet das `title`-Attribut
  `Transkript fehlt: <Fehlertext>` — **mit `escapeHtml`**, weil der Text in
  ein Attribut geht (heute ist `title` statisch und unescaped; die Funktion
  bekommt einen optionalen vierten Parameter `detail?: string | null`).
- `openSummaryDialog()`: Meldung „Kein Transkript vorhanden - bitte Video neu
  hinzufügen" → „Kein Transkript vorhanden – bitte „Transkript laden“ versuchen".
- CSS: `.transcript-error` als ruhiger Hinweiskasten im bestehenden
  Dark-Theme — `background: var(--panel-2)`, `border-left: 3px solid
  var(--danger)`, `border-radius: var(--radius)`, Padding 12–14px, Titel
  in `var(--text)` fett, Fehlertext in `var(--muted)` mit `word-break:
  break-word`, Hinweis normal in `var(--text)`, Retry-Zeile in `var(--muted)`
  kursiv. Keine neuen Farben, keine neuen Variablen.

## Doku

- `TODO.md`: den Eintrag „Surface transcript fetch failures transparently in
  the UI …" aus „Next TODOs" entfernen; in „Current State" einen Satz zur
  gespeicherten `transcript_error`-Spalte und der Anzeige ergänzen; „Last
  Verified State" aktualisieren (Datum 2026-09-05, Gate-Ergebnisse).
- `README.md`: nur anpassen, falls dort das Verhalten bei fehlendem
  Transkript beschrieben ist (`grep -n -i transkript README.md`).

## Gates

Aus `src-tauri/`: `cargo fmt`, `cargo test` (der Netzwerk-Test bleibt
ignored). Aus dem Repo-Root: `npm run build`. **Nicht committen.**
