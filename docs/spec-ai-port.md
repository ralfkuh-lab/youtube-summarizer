# Spec: KI-Konfiguration nach folio-Muster portieren

> Arbeitsdokument mit Fortschritts-Checkliste (Muster:
> `~/dev/folio/docs/spec-ki-tab.md`). Checkboxen pro grün getesteter
> Etappe abhaken. Beschlossen am 2026-07-09.

## Ziel

Der youtube-summarizer übernimmt die KI-Provider-/Modell-Verwaltung aus
**folio** (`~/dev/folio`) so 1:1 wie möglich. folio ist die maßgebliche
VORLAGE — bei Detailfragen gilt: so machen wie folio, Abweichungen nur
wo in dieser Spec ausdrücklich genannt. Das Settings-Panel wird nach dem
folio-Schema umgebaut (Bereichs-Tabs „KI-Anbieter" / „KI-Modelle").

Die bestehende Eigenimplementierung (`src-tauri/src/ai_config/`,
`src/ai-config.ts`, hartkodierter Provider-Katalog, Keys in
`config.json`) wird vollständig ersetzt.

## Vorlage-Dateien in folio (lokal lesen!)

| folio | Inhalt |
|---|---|
| `src-tauri/src/ai/types.rs` | Katalog- + ai.json-Datenmodell |
| `src-tauri/src/ai/catalog.rs` | models.dev-Snapshot + Cache + Refresh |
| `src-tauri/src/ai/models-dev-snapshot.json` | eingebetteter Snapshot (kopieren) |
| `src-tauri/src/ai/config.rs` | `AiConfigService` (ai.json, atomare Writes) |
| `src-tauri/src/ai/auth.rs` | `AuthStore` (auth.json, 0600) |
| `src-tauri/src/ai/client.rs` | OpenAI-kompatibler Chat-Client (SSE + JSON-Fallback) |
| `src-tauri/src/commands/ai.rs` | Tauri-Commands (nur die unten gelisteten übernehmen) |
| `src-tauri/web/app/ui/settings-ai.ts` | Settings-UI KI-Anbieter/KI-Modelle |
| `src-tauri/web/app/ui/ai-model-picker.ts` | Modell-Dropdown über Whitelist |
| `src-tauri/web/app/ui/controls.ts` | `makeToggle` |
| `src-tauri/web/app/ui/settings-dialog.ts` | Tab-Mechanik (role=tab/tabpanel, Pfeiltasten) |
| `src-tauri/dist/index.html` (Zeilen ~140–330) | Markup-Muster der Tabs/Panels |
| `src-tauri/web/styles/settings-ai.css` | Styles (kopieren/anpassen) |
| `scripts/update-models-snapshot.py` | Snapshot-Update-Skript (kopieren) |
| `docs/spec-ki-tab.md` | Architektur-Begründungen (Kontext) |

NICHT übernehmen: `ai/mask.rs`, Übersetzungs-/Theme-Author-Commands
(`ai_translate_*`, `ai_theme_author*`, `ai_recent_languages_set`),
`translate-dialog.ts` — das sind folio-Features ohne Entsprechung hier.

## Architektur (folio-Parität)

1. **Katalog von models.dev**: Snapshot via `include_str!` eingebettet,
   Laufzeit-Cache `ai-catalog.json` im App-Config-Verzeichnis
   (`AppPaths`-Config-Dir, wo heute `config.json` liegt), neuere Quelle
   gewinnt, Refresh NUR auf User-Klick.
2. **`ai.json`** im Config-Verzeichnis (Schema wie folio/opencode):
   aktivierte Provider, Modell-Whitelist pro Provider, Custom-Provider
   (`custom: true`, `options.baseURL`, `models`-Map), `defaultModel`
   (`{provider, model}`). Kein `translate`-Block.
3. **`auth.json`** im Config-Verzeichnis, 0600 (inkl. Temp-File),
   Format `{ "<providerId>": { "type": "api", "key": "..." } }`.
   Keys NIE in Logs, NIE in Automation-Responses, NIE im UI-Klartext.
4. **Client** (`ai/client.rs` aus folio): `/chat/completions`, SSE-Streaming
   mit 60-s-Chunk-Timeout, JSON-Fallback, Fehlermapping mit Key-Redaction.
   Die Zusammenfassung sammelt die Deltas zum Volltext (kein UI-Streaming
   in V1; Folgepunkt).
5. **Summarize-Verdrahtung**: `summarize_video` nutzt `defaultModel` aus
   `ai.json` + Key aus `AuthStore` + Base-URL aus Katalog bzw.
   Custom-Provider. Der bestehende Summary-Prompt und
   `parse_summary_response` inkl. `strip_wrapping_code_fence`
   (+ zugehörige Tests) bleiben unverändert erhalten.

## Tauri-Command-Oberfläche (neu)

Aus folio übernehmen (Signaturen/Verhalten identisch):
`ai_catalog_get`, `ai_catalog_refresh`, `ai_config_get`,
`ai_provider_enable`, `ai_model_toggle`, `ai_custom_upsert`,
`ai_custom_delete`, `ai_custom_models_fetch`, `ai_default_model_set`,
`ai_auth_set`, `ai_auth_remove`, `ai_auth_status`.

Zusätzlich (bewusste Abweichung, Feature bleibt erhalten):
`ai_model_chat_test { providerId, modelId, messages }` — ersetzt das
bisherige `test_provider_model_chat`, läuft über den neuen Client
(nicht-streamend, Deltas gesammelt) und versorgt den bestehenden
Chat-Test-Dialog.

Entfallen ersatzlos: `get_config`, `get_ai_providers`, `save_config`,
`save_provider_config`, `add_custom_provider`, `delete_custom_provider`,
`refresh_provider_models` (Custom-Modelle laufen über
`ai_custom_models_fetch`).

## Migration (einmalig, best-effort)

Beim ersten Start ohne `ai.json`, wenn ein altes `config.json` mit
`ai`-Block existiert:

- Provider-Keys übernehmen: Mapping alte ID → models.dev-ID anhand des
  Snapshots festlegen (im Snapshot nachsehen; erwartet u. a.
  `opencode` für OpenCode Zen, `openrouter`; OpenCode Go und
  Ollama Cloud nur mappen, falls im Katalog vorhanden — sonst Key
  überspringen). Gemappte Keys → `auth.json`, Provider → `enabled: true`.
- Alte Custom-Provider (inkl. „Ollama local") → Custom-Einträge in
  `ai.json` (`options.baseURL` aus `endpoint_override`).
- Altes ausgewähltes Provider/Modell-Paar → `defaultModel` (nur wenn
  der Provider gemappt werden konnte) + Modell in die Whitelist.
- `config.json` unangetastet lassen (bleibt als Backup liegen); die
  Existenz von `ai.json` markiert die Migration als erledigt.
- Migration loggt WAS migriert wurde (Provider-IDs), nie Keys.

## Etappen & Checkliste

### Etappe E1 — Backend

- [x] Modul `src-tauri/src/ai/` (types, catalog + Snapshot, config,
      auth, client mit SSE, mod) aus folio portieren; Pfade auf `AppPaths`
      umstellen.
- [x] `scripts/update-models-snapshot.py` kopiert.
- [x] Neue Commands + Registrierung; legacy ai_config/ + ai aus AppConfig/storage entfernt.
- [x] summarize auf defaultModel + Auth + stream Client; Prompt + strip + Tests (moved to commands) erhalten.
- [x] Migration robust (keys first+atomic ai marker, no-key custom, hosted detection, once in setup).
- [x] Automation + managed state in commands.
- [x] Unit-Tests + SSE; Gates grün.
- [x] Gates: `cargo test`, `cargo fmt --check`, `npm run build`.

### Etappe E2 — Frontend (Settings-Panel nach folio-Schema)

- [x] Settings-Modal-Markup in `src/main.ts` (Shell-Template) auf das
      folio-Tab-Schema umbauen: vertikale Tab-Leiste
      `settings-tab-ki-anbieter` / `settings-tab-ki-modelle`
      (role=tab/aria-selected/tabindex + Panels
      `data-settings-tab=…`, Pfeiltasten-Navigation). Modal + Schließen-Button erhalten.
- [x] `src/ai-config.ts` durch Port/Adapt von folios `settings-ai.ts`
      ersetzt: Anbieter-Panel + Modelle-Panel, Toggles, Key-Status (ohne Klartext),
      Custom upsert, Modelle abrufen, Katalog refresh, Default-Set, Test-Button.
      Anpassungen: @tauri core invoke, dom-utils makeToggle, vorhandene setStatus.
- [x] Chat-Test-Dialog an `ai_model_chat_test` angebunden (Dialog-UX reuse).
- [x] `src/settings-ai.css` angelegt und importiert.
- [x] Fußzeile `statusModel` zeigt defaultModel aus ai_config_get.
- [x] Gates: `npm run build`, `cargo test` (grün).

## Entfallende Features (bewusst, bei Bedarf Folgepunkte)

- Ollama-Cloud-Plan-Auswahl + Free-Tier-Availability-Probing (der
  models.dev-Katalog liefert Kosten-/Kontext-Metadaten).
- „Free only"-Filter und heuristische Tags.
- Provider-Status-Punkte + „models_updated_at"-Relativzeit (ersetzt
  durch Katalog-Standdatum).
- `:cloud`-Filter beim lokalen Ollama-Refresh (die Whitelist übernimmt
  die Kuratierung).

## Risiken / bewusste Entscheidungen

- Keys wandern von `config.json` (Klartext, 0644) nach `auth.json`
  (Klartext, 0600) — Verbesserung, bewusste opencode-Parität statt
  Keyring (wie folio).
- Kein jsdom-Test-Setup in diesem Projekt; Frontend-Absicherung über
  `npm run build` + Funktionstest via Automation-API (Folgepunkt:
  vitest wie folio).
- models.dev-Refresh ist der einzige neue Netz-Zugriff, ausschließlich
  user-initiiert.

## Verifikation

Pro Etappe: `cargo test`, `cargo fmt --check` (in `src-tauri/`),
`npm run build` (Repo-Root). Nach E2 Funktionstest im Dev-Lauf
(`npm run tauri dev`, Automation-API): Provider aktivieren, Key setzen,
Modell whitelisten, defaultModel setzen, Zusammenfassung erzeugen.
