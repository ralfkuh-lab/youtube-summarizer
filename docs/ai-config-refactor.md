# AI-Provider-Config — Backlog

Stand: 2026-05-03. Die KI-Config ist innerhalb der App bereits separat genug: Backend in `src-tauri/src/ai_config/`, Frontend in `src/ai-config.ts`, gemeinsame UI-Helfer in `src/dom-utils.ts`. Ein eigenes wiederverwendbares Crate oder eine framework-agnostische Komponente ist aktuell nicht priorisiert.

## Ist-Zustand (was bereits gut funktioniert)

- Tabs **Providers / Models** trennen sauber „was ist verbunden" vs. „was ist ausgewählt".
- **Instant-apply** überall (Toggle, Modell-Auswahl, API-Key/Endpoint auf Blur). Kein Save-Button mehr.
- **Slider-Toggle** im Models-Tab pro Provider (enabled/disabled), automatisch deaktiviert solange der Provider nicht konfiguriert ist (`isProviderConfigured`).
- **Recommended / Custom-Local** als Sektionen im Provider-Nav.
- Pro Provider: `description`, `badge`, `recommended`, `homepage_url`, `default_endpoint`, `requires_api_key`, `endpoint_editable`, `supports_model_refresh`.
- **Probing für Ollama Cloud** beim Refresh — nur im Free-Tier, nur für Modelle ohne bekannten Status. Ein Mini-Chat-Call pro Modell (`num_predict: 1`, max. 6 parallel via Semaphore). 200 → free, 403 → subscription_required, sonst unknown. Auf Pro/Max-Tier wird das Probing übersprungen und Free/Subscription-Tags werden überall unterdrückt. Manueller Re-Probe-Knopf für Edge-Cases.
- **Account-Tier** (Free/Pro/Max) pro Ollama-Cloud-Provider speicherbar.
- `last_error` pro Provider sichtbar.
- **Free-only-Filter** in der globalen Modellsuche, Auswahl persistiert in `localStorage`.
- **Selected-Model-Card** dauerhaft sichtbar.
- **Custom-Provider** anlegen/löschen, plus „Ollama local" als user-managed Sonderfall.
- **External-Link-Plugin** (`tauri-plugin-opener`) für Provider-Website-Links.
- **Provider+Modell pro Zusammenfassung** in DB persistiert, im Detail-Header angezeigt.
- **Per-Modell „Test chat"** als kleiner Multi-Turn-Dialog mit „Hi"-Vorbelegung.
- **Status-Punkt** im Provider-Nav (ready / disabled / unconfigured / error).
- **API-Key-Reveal-Toggle** (👁) am Password-Feld.
- **Subscription-required-Tag** für Ollama-Cloud-Modelle die im Probe 403 lieferten.
- **Relative Refresh-Zeit** („vor 3 Tagen") mit absolutem Datum als Tooltip.
- DB-Reads über **Spaltenname** (`row.get("…")`) und `SELECT *` — robust gegen Schema-Erweiterungen.

## Offene Verbesserungen

### 1. Context-Window + Preis als Tags

**Was:** OpenRouter und OpenCode liefern in `/v1/models` Felder wie `context_length`, `pricing.prompt`, `pricing.completion`. Als kleine Tags rendern (z. B. „128k", „$0.50/1M in").

**Warum:** Sehr nützlich beim Modell-Picken — wer ein langes Transkript zusammenfassen will, sieht sofort welche Modelle reichen und welche teuer werden.

**Aufwand:** Mittel. `AiModel` um optionale Felder erweitern, `parse_models` pro Provider erweitern, Render anpassen.

### 2. Bessere Fehleranzeige inline

**Was:** Bei ungültigem Key / unerreichbarem Endpoint Feld-Border rot + Inline-Fehlermeldung am Feld, statt nur in der globalen Status-Zeile.

**Warum:** Lokales Feedback am Eingabefeld ist klarer als eine flüchtige Status-Zeile.

**Aufwand:** Mittel. Erfordert pro Feld einen Validierungs-Hook und ein Error-Span.

## Spätere Refactoring-Ideen

Nur angehen, wenn wirklich Bedarf entsteht, die KI-Provider-Config außerhalb dieser App wiederzuverwenden.

**Backend-Crate** (`ai-providers` als eigenständiges Rust-Crate, Tauri-unabhängig):

- `pub fn provider_catalog() -> Vec<AiProviderInfo>`
- `pub async fn fetch_models(client, ai, provider_id, …) -> Result<Vec<AiModel>>`
- `pub async fn summarize(client, ai, transcript, opts) -> Result<String>`
- `pub async fn chat(client, ai, messages) -> Result<String>` (für Multi-Use-Case)
- `pub async fn probe_model(client, ai, model_id) -> ProbeResult`
- Trait `ConfigStore` für lesen/schreiben — Implementierungen für JSON-File / SQLite / In-Memory.
- Keine Tauri-Imports.

**Tauri-Wrapper** (in der jeweiligen App):

- Dünne `#[tauri::command]`-Funktionen die das Crate aufrufen.
- App-spezifische Storage-Anbindung.

**Frontend-Komponente** als framework-agnostisches **Web Component**:

```html
<ai-provider-settings
  config='{...}'
  providers='[...]'
></ai-provider-settings>
```

Events:
- `config-changed` (CustomEvent mit der vollen neuen Config)
- `model-selected` (provider_id, model_id)
- `request-refresh-models` (provider_id) — Host kümmert sich um den Backend-Call
- `request-test-connection` (provider_id)
- `status` (level, message) — Host rendert wo er will, statt eingebauter Status-Zeile

Slot für Custom-Status-Zeile / Custom-Header.

**Vorteil Web Component:** Lädt in jedem Framework (vanilla TS, React via wrapper, Svelte, etc.). Single-Source-Pflege.

**Alternative:** Lit-basierte Komponente mit kleinem React/Svelte-Wrapper-Package falls Web Components zu rough sind.

### Konfig-Schema versionieren

`AiConfig` um `schema_version: u32` erweitern. Migrationsfunktionen pro Version. Verhindert Datenverlust beim App-Update mit erweitertem Schema.

## Notizen aus Diskussion

- Ollama Cloud hat aktuell **keinen API-Endpoint** der Free vs. Subscription unterscheidet. Probing bleibt bis auf weiteres der einzige Weg. Ggf. gelegentlich `https://docs.ollama.com/cloud` checken ob ein neues Feld in `/v1/models` dazukommt.
- Ollama Cloud rechnet nach **GPU-Zeit pro Tier** (Free / Pro $20 / Max $100), nicht per-Token. Manche Modelle sind für Free schlicht gesperrt — daher der 403.
- Pro/Max-Tier: alle Aufrufe gehen aufs Plan-Kontingent, daher wird der Free-vs-Subscription-Unterschied im UI ausgeblendet.
