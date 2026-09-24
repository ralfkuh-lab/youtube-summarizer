// Einstellungen und Status der Synchronisation (Tab „Sync“) sowie der
// Sync-Knopf in der Kopfleiste. Der Token selbst verlaesst das Backend nie:
// das Frontend sieht nur `hasToken` und schickt ein leeres Feld als
// „unveraendert“ zurueck.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { activateSettingsTab } from "./ai-config";
import { forgetChatState } from "./chat";
import { clearDetail } from "./detail";
import { $, confirmDialog, errorMessage, escapeHtml, showModal } from "./dom-utils";
import { loadCollections, loadVideos, renderVideoList, selectVideo } from "./library";
import { getActiveVideo, setStatus, state } from "./state";
import type { SyncApplied, SyncConfigView, SyncStatus, SyncTestResult } from "./types";
import { formatShortDateTime } from "./utils";

const TOKEN_PLACEHOLDER = "gespeichert – leer lassen, um es zu behalten";
const EMPTY_CONFIG: SyncConfigView = {
  enabled: false,
  serverUrl: "",
  hasToken: false,
  newVideosLocal: false,
};
const STOP_TITLES: Record<string, string> = {
  auth: "Synchronisation angehalten: Anmeldung fehlgeschlagen – Token prüfen",
  dataset: "Synchronisation angehalten: anderer Datensatz – Neu abgleichen nötig",
  version: "Synchronisation angehalten: App oder Server aktualisieren",
};
const DEFERRED_POLL_MS = 400;

let config: SyncConfigView = { ...EMPTY_CONFIG };
let status: SyncStatus | null = null;
/// Aktives Video, dessen Detailansicht nach dem Ende einer laufenden
/// Zusammenfassung oder Chat-Anfrage nachgezogen werden muss.
let deferredDetailVideoId: number | null = null;
let deferredTimer: number | null = null;

function setError(message: string | null) {
  const errorEl = $<HTMLParagraphElement>("#syncError");
  errorEl.hidden = !message;
  errorEl.textContent = message ?? "";
}

function setTestResult(message: string, ok: boolean) {
  const result = $<HTMLSpanElement>("#syncTestResult");
  result.textContent = message;
  result.classList.toggle("settings-ai-error", !ok);
  result.classList.toggle("settings-hint", ok);
}

function renderConfig() {
  $<HTMLInputElement>("#syncEnabled").checked = config.enabled;
  $<HTMLInputElement>("#syncServerUrl").value = config.serverUrl;
  const token = $<HTMLInputElement>("#syncToken");
  token.value = "";
  token.placeholder = config.hasToken ? TOKEN_PLACEHOLDER : "Token";
  $<HTMLInputElement>("#syncNewVideosLocal").checked = config.newVideosLocal;
  $<HTMLButtonElement>("#syncNow").disabled = !config.enabled;
}

/// Formularinhalt als Backend-Eingabe; ein leeres Tokenfeld bleibt weg
/// (Backend: unveraendert).
function currentInput() {
  const token = $<HTMLInputElement>("#syncToken").value.trim();
  return {
    enabled: $<HTMLInputElement>("#syncEnabled").checked,
    serverUrl: $<HTMLInputElement>("#syncServerUrl").value.trim(),
    ...(token ? { token } : {}),
    newVideosLocal: $<HTMLInputElement>("#syncNewVideosLocal").checked,
  };
}

async function saveFromInputs() {
  try {
    config = await invoke<SyncConfigView>("sync_config_set", { config: currentInput() });
    setError(null);
    renderConfig();
    renderSyncButton();
    setStatus("Sync-Einstellungen gespeichert");
  } catch (error) {
    setError(errorMessage(error));
    renderConfig();
  }
}

async function testConnection() {
  setTestResult("Teste…", true);
  try {
    const result = await invoke<SyncTestResult>("sync_test", { config: currentInput() });
    if (result.sameDataset === false) {
      setTestResult("Verbunden, aber der Server gehört zu einem anderen Datensatz", false);
      return;
    }
    setTestResult(`Verbindung erfolgreich (Datensatz ${result.datasetId.slice(0, 8)}…)`, true);
  } catch (error) {
    setTestResult(errorMessage(error), false);
  }
}

async function syncNow() {
  setError(null);
  try {
    status = await invoke<SyncStatus>("sync_now");
    renderStatus();
    renderSyncButton();
    setStatus(status.lastError ?? "Synchronisation abgeschlossen");
  } catch (error) {
    setError(errorMessage(error));
  }
}

async function rebaseline() {
  const confirmed = await confirmDialog(
    "Neu abgleichen? Die Warteschlange wird verworfen, ab Position 0 gelesen und der vorhandene Bestand erneut hochgeladen.",
    { title: "Neu abgleichen", okLabel: "Neu abgleichen" },
  );
  if (!confirmed) return;
  setError(null);
  try {
    status = await invoke<SyncStatus>("sync_rebaseline");
    renderStatus();
    renderSyncButton();
    setStatus("Neu abgeglichen");
  } catch (error) {
    setError(errorMessage(error));
  }
}

function renderStatus() {
  const line = $<HTMLParagraphElement>("#syncStatusLine");
  const list = $<HTMLUListElement>("#syncUnsendableList");
  const stoppedBlock = $<HTMLDivElement>("#syncStoppedBlock");
  if (!status) {
    line.textContent = "";
    list.hidden = true;
    stoppedBlock.hidden = true;
    return;
  }

  const parts: string[] = [];
  if (!status.enabled) parts.push("Synchronisation aus");
  if (status.running) parts.push("Läuft…");
  parts.push(
    status.lastSuccessAt
      ? `Letzter Erfolg: ${formatShortDateTime(status.lastSuccessAt)}`
      : "Noch kein erfolgreicher Lauf",
  );
  parts.push(`Ausstehende Änderungen: ${status.pending}`);
  line.textContent = parts.join(" · ");

  list.innerHTML = status.unsendable
    .map((entry) => `<li>${escapeHtml(entry.entity)}: ${escapeHtml(entry.reason)}</li>`)
    .join("");
  list.hidden = status.unsendable.length === 0;
  stoppedBlock.hidden = status.stopped !== "dataset";
  setError(status.lastError);
}

function renderSyncButton() {
  const button = $<HTMLButtonElement>("#syncBtn");
  let symbol = "⇅";
  let className = "sync-btn--off";
  let title = "Synchronisation aus – klicken zum Einrichten";
  if (config.enabled && status?.stopped) {
    symbol = "⏸";
    className = "sync-btn--stopped";
    title = STOP_TITLES[status.stopped] ?? "Synchronisation angehalten";
  } else if (config.enabled && status?.running) {
    symbol = "⟳";
    className = "sync-btn--running";
    title = "Synchronisation läuft";
  } else if (config.enabled && status?.lastError) {
    symbol = "⚠";
    className = "sync-btn--error";
    title = `Synchronisation: Fehler – ${status.lastError}`;
  } else if (config.enabled) {
    symbol = "✓";
    className = "sync-btn--ok";
    title = status?.lastSuccessAt
      ? `Synchronisation aktiv – letzter Erfolg: ${formatShortDateTime(status.lastSuccessAt)}`
      : "Synchronisation aktiv – noch kein erfolgreicher Lauf";
  }
  button.textContent = symbol;
  button.className = `icon-btn sync-btn ${className}`;
  button.title = title;
  button.setAttribute("aria-label", title);
}

function requestRunning(videoId: number): boolean {
  return state.streamingVideoId === videoId || state.chatRuns.has(videoId);
}

function checkDeferredDetail() {
  const videoId = deferredDetailVideoId;
  if (videoId === null || requestRunning(videoId)) return;
  deferredDetailVideoId = null;
  if (deferredTimer !== null) {
    window.clearInterval(deferredTimer);
    deferredTimer = null;
  }
  if (state.activeVideoId === videoId) void selectVideo(videoId);
}

function refreshDetailAfterRun(videoId: number) {
  if (!requestRunning(videoId)) {
    deferredDetailVideoId = null;
    if (state.activeVideoId === videoId) void selectVideo(videoId);
    return;
  }
  deferredDetailVideoId = videoId;
  deferredTimer ??= window.setInterval(checkDeferredDetail, DEFERRED_POLL_MS);
}

async function onApplied(applied: SyncApplied) {
  const activeId = state.activeVideoId;
  // Laeuft fuer das aktive Video eine Anfrage, bleiben seine geladenen
  // Detailfelder in der Liste erhalten, bis der Nachzieher sie ersetzt.
  const keep = activeId !== null && requestRunning(activeId) ? getActiveVideo() : null;
  try {
    await loadVideos();
    if (keep) {
      state.videos = state.videos.map((video) =>
        video.id === keep.id
          ? {
              ...video,
              transcript: keep.transcript,
              chapters: keep.chapters,
              summary: keep.summary,
              description: keep.description,
            }
          : video,
      );
      renderVideoList();
    }
    await loadCollections();
  } catch (error) {
    setStatus(errorMessage(error));
    return;
  }

  if (activeId === null || !applied.videoIds.includes(activeId)) return;
  if (!state.videos.some((video) => video.id === activeId)) {
    // Wie beim lokalen Loeschen: die Auswahl faellt weg, die Detailansicht raeumt auf.
    forgetChatState(activeId);
    state.activeVideoId = null;
    clearDetail();
    return;
  }
  refreshDetailAfterRun(activeId);
}

async function onSyncButtonClick() {
  if (!config.enabled) {
    showModal("#settingsModal");
    activateSettingsTab("sync");
    await loadSyncConfig();
    return;
  }
  await syncNow();
}

export async function loadSyncConfig(): Promise<SyncConfigView> {
  try {
    config = await invoke<SyncConfigView>("sync_config_get");
  } catch (error) {
    setStatus(errorMessage(error));
    config = { ...EMPTY_CONFIG };
  }
  renderConfig();
  renderSyncButton();
  return config;
}

async function loadSyncStatus() {
  try {
    status = await invoke<SyncStatus>("sync_status");
  } catch {
    status = null;
  }
  renderStatus();
  renderSyncButton();
}

export function bindSyncSettingsEvents() {
  document.querySelector<HTMLButtonElement>("#settings-tab-sync")?.addEventListener("click", () => {
    activateSettingsTab("sync");
    void loadSyncConfig();
  });
  $("#syncEnabled").addEventListener("change", () => void saveFromInputs());
  $("#syncServerUrl").addEventListener("change", () => void saveFromInputs());
  $("#syncToken").addEventListener("change", () => void saveFromInputs());
  $("#syncNewVideosLocal").addEventListener("change", () => void saveFromInputs());
  $("#syncTest").addEventListener("click", () => void testConnection());
  $("#syncNow").addEventListener("click", () => void syncNow());
  $("#syncRebaseline").addEventListener("click", () => void rebaseline());
  $("#syncBtn").addEventListener("click", () => void onSyncButtonClick());

  void listen<SyncStatus>("sync://status", (event) => {
    status = event.payload;
    renderStatus();
    renderSyncButton();
  }).catch((error) => console.error("sync://status konnte nicht abonniert werden", error));

  void listen<SyncApplied>("sync://applied", (event) => {
    void onApplied(event.payload);
  }).catch((error) => console.error("sync://applied konnte nicht abonniert werden", error));

  void loadSyncConfig();
  void loadSyncStatus();
}
