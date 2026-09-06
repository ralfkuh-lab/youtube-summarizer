import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import "./styles.css";
import "./settings-ai.css";
import { applyConfig, bindAiConfigEvents, initAiConfig, type AiConfig } from "./ai-config";
import { bindDetailEvents, initDetail } from "./detail";
import { $, errorMessage } from "./dom-utils";
import {
  addVideo,
  bindLibraryEvents,
  initLibrary,
  renderCollectionList,
  renderVideoFilters,
  renderVideoList,
} from "./library";
import { initState, setBusy, setStatus, state } from "./state";
import { bindSummaryDialogEvents } from "./summary-dialog";
import { bindSummaryViewEvents } from "./summary-view";
import { appTemplate } from "./template";
import type { Collection, Video } from "./types";
import { isVideoStatusFilter } from "./utils";

const app = document.querySelector<HTMLDivElement>("#app");
if (!app) {
  throw new Error("App container not found");
}

app.innerHTML = appTemplate;

const statusText = $<HTMLSpanElement>("#statusText");
const statusModel = $<HTMLSpanElement>("#statusModel");

initState({ statusEl: statusText });
initAiConfig({ statusModelEl: statusModel, setStatus });
initDetail();
initLibrary();
bindEvents();
void loadInitialData();

function bindEvents() {
  $("#addBtn").addEventListener("click", () => void addVideo());
  $("#urlInput").addEventListener("keydown", (event) => {
    if (event instanceof KeyboardEvent && event.key === "Enter") {
      void addVideo();
    }
  });

  $("#videoSearchInput").addEventListener("input", (event) => {
    const target = event.target;
    if (!(target instanceof HTMLInputElement)) return;
    state.videoSearchQuery = target.value.trim();
    renderVideoList();
  });

  document.querySelectorAll<HTMLButtonElement>(".filter-chip").forEach((button) => {
    button.addEventListener("click", () => {
      const filter = button.dataset.videoFilter;
      if (!isVideoStatusFilter(filter)) return;
      state.videoStatusFilter = filter;
      renderVideoFilters();
      renderVideoList();
    });
  });

  bindAiConfigEvents();
  bindLibraryEvents();
  bindDetailEvents();
  bindSummaryDialogEvents();
  bindSummaryViewEvents();

  bindEscapeToCloseModals();

  document.addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const link = target.closest<HTMLAnchorElement>("a[href]");
    if (!link) return;
    const href = link.getAttribute("href");
    if (!href || !/^https?:\/\//i.test(href)) return;
    event.preventDefault();
    void openUrl(href).catch((err) => setStatus(errorMessage(err)));
  });
}

// Escape schliesst den obersten offenen Dialog. Die Reihenfolge bildet die
// Stapelung ab: liegt ein Dialog ueber einem anderen, geht zuerst der obere zu.
const ESCAPE_CLOSABLE_MODALS = [
  { modal: "#chatTestModal", close: "#chatTestClose" },
  { modal: "#presetEditModal", close: "#presetEditCancel" },
  { modal: "#presetManageModal", close: "#presetManageClose" },
  { modal: "#collectionModal", close: "#collectionCancel" },
  { modal: "#summaryModal", close: "#summaryCancel" },
  { modal: "#settingsModal", close: "#configClose" },
];

// Diese beiden behandeln Escape selbst - der Bestaetigungsdialog muss ein
// Ergebnis an seinen Aufrufer liefern, das Custom-Provider-Formular liegt ueber
// den Einstellungen. Solange einer davon offen ist, ruehrt der globale Handler
// nichts an, sonst ginge der darunterliegende Dialog gleich mit zu.
const SELF_HANDLED_MODALS = ["#confirmModal", "#ai-custom-dialog"];

function isModalOpen(selector: string): boolean {
  const element = document.querySelector<HTMLElement>(selector);
  return !!element && !element.hidden;
}

function bindEscapeToCloseModals() {
  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape" || event.defaultPrevented) return;
    if (SELF_HANDLED_MODALS.some(isModalOpen)) return;
    const open = ESCAPE_CLOSABLE_MODALS.find((entry) => isModalOpen(entry.modal));
    if (!open) return;
    event.preventDefault();
    // Ueber den vorhandenen Schliessen-Button, damit Escape denselben Pfad
    // nimmt wie ein Klick - inklusive kuenftiger Aufraeumarbeit dort.
    document.querySelector<HTMLButtonElement>(open.close)?.click();
  });
}

async function loadInitialData() {
  setBusy(true, "Videos werden geladen...");
  try {
    const [loadedVideos, loadedCollections, config] = await Promise.all([
      invoke<Video[]>("get_videos"),
      invoke<Collection[]>("get_collections"),
      invoke<AiConfig>("ai_config_get"),
    ]);
    state.videos = loadedVideos;
    state.collections = loadedCollections;
    // providers/catalog now via ai_catalog_get inside settings
    renderCollectionList();
    renderVideoList();
    applyConfig(config);
    setStatus("Bereit");
  } catch (error) {
    setStatus(errorMessage(error));
  } finally {
    setBusy(false);
  }
}
