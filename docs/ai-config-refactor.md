# AI-Provider-Config — Verbesserungen & Refactoring-Plan

Stand: 2026-05-02. Bezugspunkt ist die aktuelle KI-Config in `src/main.ts` + `src-tauri/src/ai.rs` / `models.rs` / `storage.rs`.

## Ist-Zustand (was bereits gut funktioniert)

- Tabs **Providers / Models** trennen sauber „was ist verbunden" vs. „was ist ausgewählt".
- **Instant-apply** überall (Toggle, Modell-Auswahl, API-Key/Endpoint auf Blur). Kein Save-Button mehr.
- **Slider-Toggle** im Models-Tab pro Provider (enabled/disabled), automatisch deaktiviert solange der Provider nicht konfiguriert ist (`isProviderConfigured`).
- **Recommended / Custom-Local** als Sektionen im Provider-Nav.
- Pro Provider: `description`, `badge`, `recommended`, `homepage_url`, `default_endpoint`, `requires_api_key`, `endpoint_editable`, `supports_model_refresh`.
- **Probing für Ollama Cloud** beim Refresh: Ein Mini-Chat-Call pro Modell (`num_predict: 1`, max. 6 parallel via Semaphore). 200 → free, 403 → paid, sonst konservativ false.
- `last_error` pro Provider sichtbar.
- **Free-only-Filter** in der globalen Modellsuche.
- **Selected-Model-Card** dauerhaft sichtbar.
- **Custom-Provider** anlegen/löschen, plus „Ollama local" als user-managed Sonderfall.
- **External-Link-Plugin** (`tauri-plugin-opener`) für Provider-Website-Links.
- **Provider+Modell pro Zusammenfassung** in DB persistiert, im Detail-Header angezeigt.
- DB-Reads über **Spaltenname** (`row.get("…")`) und `SELECT *` — robust gegen Schema-Erweiterungen.

## Verbesserungen (sortiert nach Nutzen)

### 1. „Test connection"-Button pro Provider

**Was:** Neben „Refresh models" ein zweiter Button. Sendet einen 1-Token-Chat-Call gegen das aktuell ausgewählte Modell des Providers. Zeigt: ✅ ok / ❌ Fehlermeldung mit HTTP-Status.

**Warum:** Heute weiß man erst beim ersten echten Summary, ob Key/Endpoint/Modell zusammen funktionieren. Häufige Fehlerursachen (falscher Endpoint-Pfad, abgelaufener Key, Modell ausgemustert) werden so vor dem ersten produktiven Lauf gefunden.

**Aufwand:** Klein. Die Probe-Logik aus `probe_ollama_cloud_free` lässt sich für einen Einzelaufruf trivial extrahieren.

### 2. Status-Punkt im Provider-Nav

**Was:** Farbiger Dot links neben dem Provider-Namen.
- 🟢 enabled + configured + kein `last_error`
- 🟡 configured aber disabled
- ⚫ nicht konfiguriert
- 🔴 `last_error` gesetzt

**Warum:** Der heutige Text-Suffix („· configured · disabled") scannt sich schlecht. Ein Dot ist sofort erkennbar, gerade bei vielen Providern.

**Aufwand:** Klein, reines CSS + Render-Funktion in `renderProviderNavItem`.

### 3. „Subscription required"-Pill für Ollama-Cloud-Modelle ohne Free-Status

**Was:** In der Modellliste explizit anzeigen, wenn ein Ollama-Cloud-Modell beim letzten Probe 403 lieferte. Aktuell ist nur die Abwesenheit des Free-Tags zu sehen.

**Warum:** Schließt die UX-Schleife zum Probing. Verhindert „warum kann ich dieses Modell nicht nutzen?"-Verwirrung.

**Aufwand:** Klein. Erfordert ein zusätzliches Feld am `AiModel` (z. B. `requires_subscription: bool`) oder ein Tag-Eintrag „Subscription".

### 4. Context-Window + Preis als Tags

**Was:** OpenRouter und OpenCode liefern in `/v1/models` Felder wie `context_length`, `pricing.prompt`, `pricing.completion`. Als kleine Tags rendern (z. B. „128k", „$0.50/1M in").

**Warum:** Sehr nützlich beim Modell-Picken — wer ein langes Transkript zusammenfassen will, sieht sofort welche Modelle reichen und welche teuer werden.

**Aufwand:** Mittel. `AiModel` um optionale Felder erweitern, `parse_models` pro Provider erweitern, Render anpassen.

### 5. API-Key-Reveal-Toggle (👁)

**Was:** Augen-Icon am Password-Feld. Klick toggelt zwischen `type="password"` und `type="text"`.

**Warum:** Standard-UX-Pattern. Hilft beim Verifizieren des Keys ohne ihn neu reinzukopieren.

**Aufwand:** Trivial, reines Frontend.

### 6. Probe-Cache mit TTL

**Was:** Probe-Ergebnis (free / paid) pro Modell mit Zeitstempel speichern. Beim Refresh nur neu proben, wenn der letzte Probe älter als z. B. 7 Tage ist (oder der Modellname neu auftaucht).

**Warum:** Aktuell pingt jeder Refresh alle 39 Ollama-Cloud-Modelle — vergeudet Zeit (~10 s) und einen winzigen Teil der Free-Quota. Ein „Force re-probe"-Knopf für Edge-Cases.

**Aufwand:** Klein-mittel. `AiModel` um `free_probed_at: Option<String>` erweitern, Logik in `probe_ollama_cloud_free` anpassen.

### 7. `showOnlyFreeModels` persistieren

**Was:** Aktueller `let showOnlyFreeModels = false` in `main.ts` ist nur in-memory. In `AiConfig` (oder einem `UiPrefs`-Block) ablegen.

**Warum:** Klein, aber lästig dass die Filter-Einstellung jeden Neustart vergessen wird.

**Aufwand:** Trivial.

### 8. „Refreshed: 2.5.2026" → relative Zeit

**Was:** „vor 3 Tagen" / „gerade eben" statt absolutem Datum. Original-Datum als `title`-Tooltip.

**Warum:** Für „ist das aktuell genug?" ist die relative Zeit informativer.

**Aufwand:** Klein, Helper-Funktion plus Render-Anpassung.

### 9. Bessere Fehleranzeige inline

**Was:** Bei ungültigem Key / unerreichbarem Endpoint Feld-Border rot + Inline-Fehlermeldung am Feld, statt nur in der globalen Status-Zeile.

**Warum:** Lokales Feedback am Eingabefeld ist klarer als eine flüchtige Status-Zeile.

**Aufwand:** Mittel. Erfordert pro Feld einen Validierungs-Hook und ein Error-Span.

### 10. Default-Modell pro Provider sinnvoll vorbelegen

**Was:** Wenn ein Provider erstmals konfiguriert (Key eingetragen + Models geladen), automatisch ein sinnvolles Default-Modell setzen (z. B. das billigste mit Free-Tag, oder ein hardcoded Empfehlungs-Modell pro Provider).

**Warum:** Spart einen Klick und verhindert „kein Modell ausgewählt"-Zustand direkt nach Setup.

**Aufwand:** Klein. Pro Eintrag in `provider_catalog()` ein `default_model: Option<String>`-Feld.

### 11. Per-Use-Case-Modellzuweisung (Architektur)

**Was:** Statt einem globalen `model` in `AiConfig` ein Map-Feld `models_by_task: { summarize: …, chat: …, classify: … }` mit Fallback auf einen Default.

**Warum:** Sobald die App mehr als nur „summarize" macht, will man unterschiedliche Modelle pro Task (großes Modell für Summary, schnelles Modell für Chat). Lieber jetzt im Schema vorsehen als später migrieren.

**Aufwand:** Mittel. Schema-Migration + UI-Erweiterung. Kann anfangs versteckt bleiben (nur ein Eintrag „summarize") und mit zusätzlichen Tasks wachsen.

## Refactoring für Wiederverwendbarkeit

Heute ist die KI-Config eng mit `main.ts` verzahnt: HTML-Strings, globale Variablen (`aiConfig`, `aiProviders`, `selectedSettingsProviderId`), direkte `setStatus`-Aufrufe, `invoke()`-Calls. Für Wiederverwendung in anderen Apps zu unspezifisch.

### Ziel-Architektur

**Backend-Crate** (`ai-providers` als eigenständiges Rust-Crate, Tauri-unabhängig):

- `pub fn provider_catalog() -> Vec<AiProviderInfo>`
- `pub async fn fetch_models(client, ai, provider_id) -> Result<Vec<AiModel>>`
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

### Reihenfolge

1. **Erst die Architektur entscheiden** (eigenes Crate + Web Component? oder vorerst nur Modul-Refactor in dieser App?). Sonst werden alle Verbesserungen unten doppelt gepflegt.
2. **Dann Punkte 1, 2, 3, 5, 7** als Quick-Wins (alle klein, hohe Sichtbarkeit).
3. **Punkt 6, 8, 10** als zweite Welle.
4. **Punkt 4, 9, 11** wenn der Bedarf konkret wird.

## Notizen aus Diskussion

- Ollama Cloud hat aktuell **keinen API-Endpoint** der Free vs. Subscription unterscheidet. Probing bleibt bis auf weiteres der einzige Weg. Ggf. gelegentlich `https://docs.ollama.com/cloud` checken ob ein neues Feld in `/v1/models` dazukommt.
- Ollama Cloud rechnet nach **GPU-Zeit pro Tier** (Free / Pro $20 / Max $100), nicht per-Token. Manche Modelle sind für Free schlicht gesperrt — daher der 403.
- `probe_ollama_cloud_free` verbraucht pro freiem Modell ~1 Token Free-Quota pro Refresh. Vernachlässigbar, aber Argument für TTL-Cache (Punkt 6).
