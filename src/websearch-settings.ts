// Einstellungen der Webrecherche (Tab „Websuche“). Der Chat liest die
// Konfiguration ueber `isWebSearchConfigured()` und wird bei Aenderungen ueber
// registrierte Listener benachrichtigt.

import { invoke } from "@tauri-apps/api/core";
import { activateSettingsTab } from "./ai-config";
import { $, errorMessage } from "./dom-utils";
import { setStatus } from "./state";

export type WebSearchConfig = {
  enabled: boolean;
  searxngUrl: string;
};

const EMPTY_CONFIG: WebSearchConfig = { enabled: false, searxngUrl: "" };
const EMPTY_URL_HINT = "Bitte zuerst eine SearXNG-URL eintragen";

let config: WebSearchConfig = { ...EMPTY_CONFIG };
const listeners: Array<() => void> = [];

export function getWebSearchConfig(): WebSearchConfig {
  return config;
}

export function isWebSearchConfigured(): boolean {
  return config.enabled && !!config.searxngUrl.trim();
}

export function onWebSearchConfigChanged(listener: () => void) {
  listeners.push(listener);
}

function notify() {
  for (const listener of listeners) listener();
}

export async function loadWebSearchConfig(): Promise<WebSearchConfig> {
  try {
    config = await invoke<WebSearchConfig>("web_search_config_get");
  } catch (error) {
    setStatus(errorMessage(error));
    config = { ...EMPTY_CONFIG };
  }
  renderConfig();
  notify();
  return config;
}

function renderConfig() {
  const enabled = document.querySelector<HTMLInputElement>("#webSearchEnabled");
  const url = document.querySelector<HTMLInputElement>("#webSearchUrl");
  if (enabled) enabled.checked = config.enabled;
  if (url) url.value = config.searxngUrl;
}

function setError(message: string | null) {
  const errorEl = $<HTMLParagraphElement>("#webSearchError");
  errorEl.hidden = !message;
  errorEl.textContent = message ?? "";
}

function setTestResult(message: string, ok: boolean) {
  const result = $<HTMLSpanElement>("#webSearchTestResult");
  result.textContent = message;
  result.classList.toggle("settings-ai-error", !ok);
  result.classList.toggle("settings-hint", ok);
}

async function saveFromInputs() {
  const enabled = $<HTMLInputElement>("#webSearchEnabled").checked;
  const searxngUrl = $<HTMLInputElement>("#webSearchUrl").value;
  try {
    config = await invoke<WebSearchConfig>("web_search_config_set", {
      config: { enabled, searxngUrl },
    });
    setError(null);
    renderConfig();
    updateEmptyUrlHint();
    notify();
    setStatus("Websuche gespeichert");
  } catch (error) {
    setError(errorMessage(error));
  }
}

async function testConnection() {
  const field = $<HTMLInputElement>("#webSearchUrl");
  const url = field.value.trim();
  if (!url) {
    // Leeres Feld: kein Backend-Aufruf, Fokus ins Feld.
    setTestResult(EMPTY_URL_HINT, false);
    field.focus();
    return;
  }
  setTestResult("Teste…", true);
  try {
    const count = await invoke<number>("web_search_test", { url });
    setTestResult(`${count} Treffer`, true);
  } catch (error) {
    setTestResult(errorMessage(error), false);
  }
}

/// Hinweis, wenn aktiviert wird, ohne dass eine URL eingetragen ist.
function updateEmptyUrlHint() {
  const enabled = $<HTMLInputElement>("#webSearchEnabled").checked;
  const empty = !$<HTMLInputElement>("#webSearchUrl").value.trim();
  setError(enabled && empty ? EMPTY_URL_HINT : null);
}

export function bindWebSearchSettingsEvents() {
  const tab = document.querySelector<HTMLButtonElement>("#settings-tab-websuche");
  tab?.addEventListener("click", () => {
    activateSettingsTab("websuche");
    void loadWebSearchConfig();
  });
  $("#webSearchEnabled").addEventListener("change", () => {
    updateEmptyUrlHint();
    void saveFromInputs();
  });
  $("#webSearchUrl").addEventListener("change", () => void saveFromInputs());
  $("#webSearchTest").addEventListener("click", () => void testConnection());
  // Nach dem Schliessen der Einstellungen den Chat-Schalter neu bewerten.
  $("#configClose").addEventListener("click", () => notify());
}
