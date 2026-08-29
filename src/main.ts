import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { marked } from "marked";
import DOMPurify from "dompurify";
import "./styles.css";
import "./settings-ai.css";
import {
  applyConfig,
  bindAiConfigEvents,
  ensureAiData,
  fillModelPicker,
  initAiConfig,
  type AiConfig,
} from "./ai-config";
import { $, confirmDialog, errorMessage, escapeHtml, hideModal, showModal } from "./dom-utils";

marked.setOptions({ gfm: true, breaks: false });

type Chapter = {
  time: string;
  start: number;
  title: string;
};

type TranscriptSnippet = {
  text: string;
  start: number;
  time: string;
};

type Video = {
  id: number;
  video_id: string;
  url: string;
  title: string;
  thumbnail_url: string;
  thumbnail?: string | null;
  transcript?: string | null;
  chapters?: Chapter[] | null;
  summary?: string | null;
  summary_provider?: string | null;
  summary_model?: string | null;
  published_at?: string | null;
  collection_ids: number[];
  created_at: string;
  updated_at: string;
};

type Collection = {
  id: number;
  name: string;
  video_count: number;
  created_at: string;
  updated_at: string;
};

type TabName = "transcript" | "summary" | "video";

let videos: Video[] = [];
let collections: Collection[] = [];
let activeVideoId: number | null = null;
let activeCollectionId: number | null = null;
let activeTab: TabName = "transcript";
let busy = false;
let videoSearchQuery = "";
let videoStatusFilter: VideoStatusFilter = "all";
let editingCollectionId: number | null = null;

type VideoStatusFilter = "all" | "transcript" | "missing-transcript" | "summary" | "missing-summary";

const app = document.querySelector<HTMLDivElement>("#app");
if (!app) {
  throw new Error("App container not found");
}

app.innerHTML = `
  <header>
    <div class="app-brand">
      <h1>YouTube Summarizer</h1>
    </div>
    <div id="addBar">
      <input id="urlInput" type="text" placeholder="YouTube-URL oder Video-ID eingeben..." />
      <button id="addBtn">Hinzufügen</button>
    </div>
    <div class="toolbar-actions">
      <button id="settingsBtn" class="icon-btn" title="Einstellungen" aria-label="Einstellungen">⚙</button>
    </div>
  </header>

  <main>
    <aside id="libraryPanel">
      <div class="collection-tools">
        <div class="library-section-title">
          <span>Sammlungen</span>
          <button id="addCollectionBtn" class="mini-icon-btn" title="Sammlung erstellen" aria-label="Sammlung erstellen">+</button>
        </div>
        <div id="collectionList"></div>
      </div>
      <div class="library-tools">
        <div class="library-search">
          <input id="videoSearchInput" type="search" placeholder="Videos suchen..." autocomplete="off" />
        </div>
        <div class="library-filters" aria-label="Videofilter">
          <button class="filter-chip active" data-video-filter="all">Alle</button>
          <button class="filter-chip" data-video-filter="transcript">Transkript</button>
          <button class="filter-chip" data-video-filter="missing-transcript">Ohne T</button>
          <button class="filter-chip" data-video-filter="summary">Zusammenfassung</button>
          <button class="filter-chip" data-video-filter="missing-summary">Ohne Z</button>
        </div>
      </div>
      <div id="videoList"></div>
    </aside>
    <section id="detail">
      <div id="detailPlaceholder">Wähle ein Video aus der Liste</div>
      <div id="detailContent" hidden>
        <div id="detailHeader">
          <img id="detailThumb" alt="" />
          <div class="detail-title-block">
            <h2 id="detailTitle"></h2>
            <a id="detailUrl" href="#" target="_blank" rel="noreferrer"></a>
            <span id="detailPublishedMeta" class="detail-summary-meta" hidden></span>
            <span id="detailSummaryMeta" class="detail-summary-meta" hidden></span>
          </div>
          <button id="deleteBtn" class="delete-icon-btn" title="Video entfernen" aria-label="Video entfernen">🗑</button>
        </div>

        <div id="collectionAssignment" class="collection-assignment"></div>

        <div id="tabBar">
          <button class="tab active" data-tab="transcript">Transkript</button>
          <button class="tab" data-tab="summary">Zusammenfassung</button>
          <button class="tab" data-tab="video">Video</button>
          <button id="reloadTranscriptBtn">Transkript laden</button>
          <button id="summarizeBtn">Zusammenfassen lassen</button>
        </div>

        <div id="tabContent">
          <div id="tabTranscript" class="tabPanel active"></div>
          <div id="tabSummary" class="tabPanel"></div>
          <div id="tabVideo" class="tabPanel">
            <div id="videoCodecNotice" class="video-codec-notice" hidden></div>
            <div class="video-player-shell">
              <iframe
                id="videoPlayer"
                title="YouTube Video"
                allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share; fullscreen"
                allowfullscreen
                referrerpolicy="strict-origin-when-cross-origin"
              ></iframe>
            </div>
            <div class="video-fallback">
              <a id="videoFallbackLink" href="#" target="_blank" rel="noreferrer">Video auf YouTube öffnen</a>
            </div>
          </div>
        </div>
      </div>

    </section>
    <aside id="chaptersPanel" class="chapters-panel" hidden>
      <h3>Kapitel</h3>
      <div id="chaptersList"></div>
    </aside>
  </main>

  <footer>
    <span id="statusText">Bereit</span>
    <span id="statusModel"></span>
  </footer>

  <div id="settingsModal" class="modal" hidden>
    <div class="modal-content settings-content">
      <div class="settings-dialog__panel">
        <div class="settings-dialog__tabs" role="tablist" aria-label="KI Einstellungen">
          <button id="settings-tab-ki-anbieter" role="tab" aria-selected="true" aria-controls="settings-panel-ki-anbieter" tabindex="0" class="settings-dialog__tab settings-dialog__tab--active">KI-Anbieter</button>
          <button id="settings-tab-ki-modelle" role="tab" aria-selected="false" aria-controls="settings-panel-ki-modelle" tabindex="-1" class="settings-dialog__tab">KI-Modelle</button>
        </div>
        <div class="settings-dialog__tabpanel settings-ai-panel" id="settings-panel-ki-anbieter" role="tabpanel" aria-labelledby="settings-tab-ki-anbieter" data-settings-tab="ki-anbieter" hidden>
          <section class="settings-section">
            <h3 class="settings-section__title">KI-Anbieter</h3>
            <p class="settings-hint">Schlüssel liegen im Klartext in <code>auth.json</code> im Config-Verzeichnis (Dateirechte 0600).</p>
            <div class="settings-ai-toolbar">
              <input type="search" id="ai-provider-search" class="settings-input" placeholder="Anbieter suchen…" autocomplete="off" />
              <button type="button" id="ai-custom-add" class="settings-ai-button">Anbieter hinzufügen</button>
            </div>
            <p class="settings-hint">Die vordefinierten Anbieter stammen aus dem <code>models.dev</code>-Katalog; aktualisieren lässt er sich im Reiter „KI-Modelle“. Eigene (OpenAI-kompatible) Anbieter lassen sich über „Anbieter hinzufügen“ ergänzen.</p>
            <p id="ai-providers-error" class="settings-ai-error" hidden></p>
            <div id="ai-provider-list" class="settings-ai-list" aria-live="polite"></div>
          </section>
          <div id="ai-custom-dialog" class="settings-ai-overlay" hidden>
            <form id="ai-custom-form" class="settings-ai-dialog" role="dialog" aria-modal="true" aria-labelledby="ai-custom-title">
              <h3 id="ai-custom-title">Anbieter hinzufügen</h3>
              <label for="ai-custom-id">ID</label>
              <input type="text" id="ai-custom-id" class="settings-input" autocomplete="off" spellcheck="false" />
              <p class="settings-hint">Nur Kleinbuchstaben, Zahlen, - und _</p>
              <label for="ai-custom-name">Anzeigename</label>
              <input type="text" id="ai-custom-name" class="settings-input" autocomplete="off" />
              <label for="ai-custom-base-url">Basis-URL</label>
              <input type="url" id="ai-custom-base-url" class="settings-input" placeholder="http://localhost:11434/v1" autocomplete="off" spellcheck="false" />
              <p class="settings-hint">OpenAI-kompatibler Endpoint.</p>
              <label for="ai-custom-key">Schlüssel (optional)</label>
              <input type="password" id="ai-custom-key" class="settings-input" autocomplete="new-password" />
              <p id="ai-custom-error" class="settings-ai-error" hidden></p>
              <div class="settings-ai-dialog__actions">
                <button type="button" id="ai-custom-cancel">Abbrechen</button>
                <button type="submit" id="ai-custom-save" class="primary">Speichern</button>
              </div>
            </form>
          </div>
        </div>
        <div class="settings-dialog__tabpanel settings-ai-panel" id="settings-panel-ki-modelle" role="tabpanel" aria-labelledby="settings-tab-ki-modelle" data-settings-tab="ki-modelle" hidden>
          <section class="settings-section">
            <div class="settings-ai-toolbar">
              <input type="search" id="ai-model-search" class="settings-input" placeholder="Provider oder Modell suchen…" autocomplete="off" />
              <button type="button" id="ai-catalog-refresh" class="settings-ai-button" title="Lädt den Anbieter- und Modellkatalog der vordefinierten Cloud-Provider neu von models.dev.">Anbieter-/Modellkatalog aktualisieren</button>
            </div>
            <p class="settings-hint">Lädt Anbieter- und Modellliste der vordefinierten Cloud-Provider von <code>models.dev</code>. Modelle eigener Anbieter holst du im jeweiligen Anbieter über „Modelle abrufen“.</p>
            <p id="ai-catalog-updated" class="settings-hint"></p>
            <p id="ai-models-error" class="settings-ai-error" hidden></p>
            <div class="settings-row">
              <label for="ai-default-model">Default-Modell</label>
              <select id="ai-default-model" class="settings-input">
                <option value="">(keins)</option>
              </select>
            </div>
          </section>
          <section class="settings-section">
            <div id="ai-model-list" class="settings-ai-model-list" aria-live="polite"></div>
          </section>
        </div>
      </div>
      <div class="modal-actions">
        <button id="configClose">Schließen</button>
      </div>
    </div>
  </div>

  <div id="summaryModal" class="modal" hidden>
    <div class="modal-content modal-wide">
      <h2>Zusammenfassung konfigurieren</h2>
      <label>Modell
        <select id="summaryModel"></select>
      </label>
      <div class="summary-row">
        <label>Detailgrad
          <select id="summaryDetail">
            <option value="short">Kurz</option>
            <option value="medium" selected>Mittel</option>
            <option value="detailed">Ausführlich</option>
          </select>
        </label>
        <label>Sprache
          <select id="summaryLang">
            <option value="original">Original</option>
            <option value="german">Deutsch</option>
            <option value="english">English</option>
            <option value="french">Français</option>
            <option value="spanish">Español</option>
            <option value="italian">Italiano</option>
          </select>
        </label>
        <label>Kapitel nutzen
          <select id="summaryUseChapters">
            <option value="yes">Ja</option>
            <option value="no">Nein</option>
          </select>
        </label>
      </div>
      <label>Prompt
        <textarea id="summaryPrompt" rows="8"></textarea>
      </label>
      <div class="modal-actions">
        <button id="summaryStart">Zusammenfassen</button>
        <button id="summaryCancel">Abbrechen</button>
      </div>
    </div>
  </div>

  <div id="collectionModal" class="modal" hidden>
    <div class="modal-content collection-modal-content">
      <h2 id="collectionModalTitle">Sammlung</h2>
      <label>Name
        <input id="collectionNameInput" type="text" maxlength="80" />
      </label>
      <div class="modal-actions">
        <button id="collectionSave">Speichern</button>
        <button id="collectionCancel">Abbrechen</button>
      </div>
    </div>
  </div>

  <div id="confirmModal" class="modal" hidden>
    <div class="modal-content confirm-content">
      <h2 id="confirmTitle">Bestätigen</h2>
      <p id="confirmMessage"></p>
      <div class="modal-actions">
        <button id="confirmOk">OK</button>
        <button id="confirmCancel">Abbrechen</button>
      </div>
    </div>
  </div>

  <div id="chatTestModal" class="modal" hidden>
    <div class="modal-content chat-test-content">
      <div class="chat-test-head">
        <div>
          <h2 id="chatTestTitle">Test chat</h2>
          <p id="chatTestMeta"></p>
        </div>
      </div>
      <div id="chatTestMessages" class="chat-test-messages"></div>
      <div id="chatTestError" class="chat-test-error" hidden></div>
      <div class="chat-test-composer">
        <textarea id="chatTestMessage" rows="3">Say "ok" in one short sentence.</textarea>
        <button id="chatTestSend">Send</button>
      </div>
      <div class="modal-actions">
        <button id="chatTestClose">Close</button>
      </div>
    </div>
  </div>
`;

const videoList = $<HTMLDivElement>("#videoList");
const collectionList = $<HTMLDivElement>("#collectionList");
const detailPlaceholder = $<HTMLDivElement>("#detailPlaceholder");
const detailContent = $<HTMLDivElement>("#detailContent");
const chaptersPanel = $<HTMLDivElement>("#chaptersPanel");
const chaptersList = $<HTMLDivElement>("#chaptersList");
const statusText = $<HTMLSpanElement>("#statusText");
const statusModel = $<HTMLSpanElement>("#statusModel");

initAiConfig({ statusModelEl: statusModel, setStatus });
bindEvents();
void loadInitialData();

function bindEvents() {
  $("#addBtn").addEventListener("click", () => void addVideo());
  $("#urlInput").addEventListener("keydown", (event) => {
    if (event instanceof KeyboardEvent && event.key === "Enter") {
      void addVideo();
    }
  });

  bindAiConfigEvents();

  $("#addCollectionBtn").addEventListener("click", () => openCollectionDialog());
  $("#collectionSave").addEventListener("click", () => void saveCollection());
  $("#collectionCancel").addEventListener("click", () => hideModal("#collectionModal"));
  $("#collectionNameInput").addEventListener("keydown", (event) => {
    if (!(event instanceof KeyboardEvent)) return;
    if (event.key === "Enter") void saveCollection();
  });

  $("#videoSearchInput").addEventListener("input", (event) => {
    const target = event.target;
    if (!(target instanceof HTMLInputElement)) return;
    videoSearchQuery = target.value.trim();
    renderVideoList();
  });

  document.querySelectorAll<HTMLButtonElement>(".filter-chip").forEach((button) => {
    button.addEventListener("click", () => {
      const filter = button.dataset.videoFilter;
      if (!isVideoStatusFilter(filter)) return;
      videoStatusFilter = filter;
      renderVideoFilters();
      renderVideoList();
    });
  });

  $("#summarizeBtn").addEventListener("click", () => void openSummaryDialog());
  $("#reloadTranscriptBtn").addEventListener("click", () => void refreshActiveTranscript());
  $("#summaryStart").addEventListener("click", () => void startSummary());
  $("#summaryCancel").addEventListener("click", () => hideModal("#summaryModal"));

  ["#summaryDetail", "#summaryLang", "#summaryUseChapters"].forEach((selector) => {
    $(selector).addEventListener("change", updateSummaryPrompt);
  });

  $("#deleteBtn").addEventListener("click", () => void deleteActiveVideo());

  bindEscapeToCloseModals();
  updateVideoCodecNotice();

  document.querySelectorAll<HTMLButtonElement>(".tab").forEach((tab) => {
    tab.addEventListener("click", () => switchTab(tab.dataset.tab as TabName));
  });

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

  $("#tabTranscript").addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const chapter = target.closest<HTMLElement>(".ts-chapter");
    if (!chapter) return;
    const start = Number(chapter.dataset.start);
    if (!Number.isNaN(start)) {
      seekVideo(start);
    }
  });
}

// Escape schliesst den obersten offenen Dialog. Die Reihenfolge bildet die
// Stapelung ab: liegt ein Dialog ueber einem anderen, geht zuerst der obere zu.
const ESCAPE_CLOSABLE_MODALS = [
  { modal: "#chatTestModal", close: "#chatTestClose" },
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


// YouTube liefert seine Streams als H.264/MP4 oder VP8/VP9/WebM ueber die
// MediaSource-API aus. WebKitGTK reicht die Dekodierung an das GStreamer des
// Systems weiter; fehlen dort die Plugins, meldet der eingebettete Player nur
// "Your browser can't play this video" und nennt den Grund nicht. Auf Windows
// und macOS stellt sich die Frage nicht - WebView2 und WKWebView bringen ihre
// Decoder mit.
function hasPlayableVideoCodec(): boolean {
  // Bewusst lokal: die Pruefung laeuft schon beim Verdrahten der Events, eine
  // Modulkonstante waere zu diesem Zeitpunkt noch nicht initialisiert.
  const streamTypes = [
    'video/mp4; codecs="avc1.42E01E"',
    'video/webm; codecs="vp9"',
    'video/webm; codecs="vp8"',
  ];
  const probe = document.createElement("video");
  if (streamTypes.some((type) => probe.canPlayType(type) !== "")) return true;
  const mediaSource = window.MediaSource;
  if (!mediaSource) return false;
  return streamTypes.some((type) => mediaSource.isTypeSupported(type));
}

function updateVideoCodecNotice() {
  const notice = $("#videoCodecNotice");
  // Nur warnen, wenn wirklich kein einziger Codec gemeldet wird - ein
  // Fehlalarm waere schlimmer als gar kein Hinweis.
  const missing = navigator.userAgent.includes("Linux") && !hasPlayableVideoCodec();
  $("#tabVideo").classList.toggle("has-codec-notice", missing);
  if (!missing) {
    notice.hidden = true;
    return;
  }
  notice.innerHTML = `
    <strong>Video kann hier nicht abgespielt werden</strong>
    <p>Diesem System fehlen die GStreamer-Codecs, mit denen Videos dekodiert
    werden. Über den Link unter dem Player lässt sich das Video weiterhin
    direkt auf YouTube ansehen.</p>
    <p>Abhilfe: Codecs installieren und die Anwendung neu starten.<br>
    <code>sudo pacman -S gst-libav gst-plugins-good gst-plugins-bad</code><br>
    <code>sudo apt install gstreamer1.0-libav gstreamer1.0-plugins-good</code></p>
  `;
  notice.hidden = false;
}

async function loadInitialData() {
  setBusy(true, "Videos werden geladen...");
  try {
    const [loadedVideos, loadedCollections, config] = await Promise.all([
      invoke<Video[]>("get_videos"),
      invoke<Collection[]>("get_collections"),
      invoke<AiConfig>("ai_config_get"),
    ]);
    videos = loadedVideos;
    collections = loadedCollections;
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

async function addVideo() {
  if (busy) return;
  const input = $<HTMLInputElement>("#urlInput");
  const url = input.value.trim();
  if (!url) return;

  setBusy(true, "Video wird hinzugefügt...");
  try {
    const video = await invoke<Video>("add_video", { url });
    videos = [video, ...videos.filter((item) => item.id !== video.id)];
    input.value = "";
    renderVideoList();
    await selectVideo(video.id);
    setStatus(video.transcript ? "Video hinzugefügt und Transkript geladen" : "Video hinzugefügt, aber kein Transkript gefunden");
  } catch (error) {
    setStatus(errorMessage(error));
  } finally {
    setBusy(false);
  }
}

async function selectVideo(id: number) {
  activeVideoId = id;
  renderVideoList();
  try {
    const video = await invoke<Video>("get_video_detail", { id });
    videos = videos.map((item) => (item.id === id ? video : item));
    showDetail(video);
  } catch (error) {
    setStatus(errorMessage(error));
  }
}

async function deleteActiveVideo() {
  if (activeVideoId === null || busy) return;
  if (!(await confirmDialog("Video wirklich löschen?", { title: "Video löschen", okLabel: "Löschen" }))) return;
  const id = activeVideoId;
  setBusy(true, "Video wird gelöscht...");
  try {
    await invoke<void>("delete_video", { id });
    videos = videos.filter((video) => video.id !== id);
    await loadCollections();
    activeVideoId = null;
    renderVideoList();
    detailContent.hidden = true;
    detailPlaceholder.hidden = false;
    chaptersPanel.hidden = true;
    setStatus("Video gelöscht");
  } catch (error) {
    setStatus(errorMessage(error));
  } finally {
    setBusy(false);
  }
}

async function refreshActiveTranscript() {
  const video = getActiveVideo();
  if (!video || busy) return;

  setBusy(true, "Transkript wird geladen...");
  try {
    const updated = await invoke<Video>("refresh_transcript", { id: video.id });
    videos = videos.map((item) => (item.id === updated.id ? updated : item));
    showDetail(updated);
    switchTab("transcript");
    setStatus("Transkript geladen");
  } catch (error) {
    setStatus(errorMessage(error));
  } finally {
    setBusy(false);
  }
}


async function openSummaryDialog() {
  const video = getActiveVideo();
  if (!video) return;
  if (!video.transcript) {
    setStatus("Kein Transkript vorhanden - bitte Video neu hinzufügen");
    return;
  }
  const saved = loadSummarySettings();
  updateSummaryPrompt();
  showModal("#summaryModal");
  try {
    await ensureAiData();
  } catch (error) {
    setStatus(errorMessage(error));
  }
  fillModelPicker($<HTMLSelectElement>("#summaryModel"), saved?.model);
}

const SUMMARY_SETTINGS_KEY = "summarySettings";

type SummarySettings = {
  detail: string;
  lang: string;
  useChapters: string;
  // JSON-kodiertes [providerId, modelId] wie im Modell-Auswahlfeld
  model?: string;
};

function loadSummarySettings(): Partial<SummarySettings> | null {
  try {
    const raw = localStorage.getItem(SUMMARY_SETTINGS_KEY);
    if (!raw) return null;
    const saved = JSON.parse(raw) as Partial<SummarySettings>;
    if (saved.detail) $<HTMLSelectElement>("#summaryDetail").value = saved.detail;
    if (saved.lang) $<HTMLSelectElement>("#summaryLang").value = saved.lang;
    if (saved.useChapters) $<HTMLSelectElement>("#summaryUseChapters").value = saved.useChapters;
    return saved;
  } catch {
    // ignore corrupt entries
    return null;
  }
}

function saveSummarySettings() {
  const settings: SummarySettings = {
    detail: $<HTMLSelectElement>("#summaryDetail").value,
    lang: $<HTMLSelectElement>("#summaryLang").value,
    useChapters: $<HTMLSelectElement>("#summaryUseChapters").value,
    model: $<HTMLSelectElement>("#summaryModel").value || undefined,
  };
  localStorage.setItem(SUMMARY_SETTINGS_KEY, JSON.stringify(settings));
}

// Das Auswahlfeld transportiert die Modellwahl als JSON-Paar. Ein leerer Wert
// bedeutet "keine Auswahl" - dann entscheidet das Backend per Default-Modell.
function parseModelValue(value: string): [string | null, string | null] {
  if (!value) return [null, null];
  try {
    const [providerId, modelId] = JSON.parse(value) as [string, string];
    return [providerId, modelId];
  } catch {
    return [null, null];
  }
}

async function startSummary() {
  const video = getActiveVideo();
  if (!video || busy) return;

  saveSummarySettings();
  const [providerId, modelId] = parseModelValue($<HTMLSelectElement>("#summaryModel").value);
  const videoId = video.id;
  setBusy(true, "Zusammenfassung wird erstellt...");
  hideModal("#summaryModal");
  switchTab("summary");
  $("#tabSummary").innerHTML = '<p class="empty">Zusammenfassung wird erstellt…</p>';

  let unlisten: (() => void) | undefined;
  try {
    unlisten = await listen<{ videoId: number; text: string; chars: number }>(
      "ai:summarize_stream",
      (event) => {
        if (event.payload.videoId !== videoId) return;
        const active = getActiveVideo();
        if (!active || active.id !== videoId) return;
        $("#tabSummary").innerHTML = markdownToHtml(event.payload.text);
        setStatus(`Zusammenfassung läuft – ${event.payload.chars} Zeichen`);
      },
    );
    const updated = await invoke<Video>("summarize_video", {
      id: videoId,
      systemPrompt: $<HTMLTextAreaElement>("#summaryPrompt").value.trim(),
      providerId,
      modelId,
    });
    videos = videos.map((item) => (item.id === updated.id ? updated : item));
    renderVideoList();
    if (getActiveVideo()?.id === videoId) {
      showDetail(updated);
      switchTab("summary");
    }
    setStatus("Zusammenfassung fertig");
  } catch (error) {
    if (getActiveVideo()?.id === videoId) {
      const current = videos.find((item) => item.id === videoId);
      if (current) {
        showDetail(current);
      }
    }
    setStatus(errorMessage(error));
  } finally {
    unlisten?.();
    setBusy(false);
  }
}

function renderCollectionList() {
  collectionList.innerHTML = `
    <button class="collection-item${activeCollectionId === null ? " active" : ""}" data-collection-id="all">
      <span class="collection-name">Alle Videos</span>
      <span class="collection-count">${videos.length}</span>
    </button>
    ${
      collections.length
        ? collections.map(renderCollectionItem).join("")
        : '<p class="empty-list compact">Noch keine Sammlungen</p>'
    }
  `;

  collectionList.querySelectorAll<HTMLButtonElement>(".collection-item").forEach((button) => {
    button.addEventListener("click", () => {
      const id = button.dataset.collectionId;
      activeCollectionId = id === "all" ? null : Number(id);
      if (Number.isNaN(activeCollectionId)) activeCollectionId = null;
      renderCollectionList();
      renderVideoList();
    });
  });

  collectionList.querySelectorAll<HTMLButtonElement>(".collection-action").forEach((button) => {
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      const id = Number(button.dataset.collectionId);
      const collection = collections.find((item) => item.id === id);
      if (!collection) return;
      if (button.dataset.action === "rename") {
        openCollectionDialog(collection);
      } else if (button.dataset.action === "delete") {
        void deleteCollection(collection);
      }
    });
  });
}

function renderCollectionItem(collection: Collection): string {
  const activeClass = activeCollectionId === collection.id ? " active" : "";
  return `
    <div class="collection-row${activeClass}">
      <button class="collection-item" data-collection-id="${collection.id}">
        <span class="collection-name">${escapeHtml(collection.name)}</span>
        <span class="collection-count">${collection.video_count}</span>
      </button>
      <div class="collection-actions">
        <button class="collection-action" data-action="rename" data-collection-id="${collection.id}" title="Sammlung umbenennen" aria-label="Sammlung umbenennen">✎</button>
        <button class="collection-action danger" data-action="delete" data-collection-id="${collection.id}" title="Sammlung löschen" aria-label="Sammlung löschen">×</button>
      </div>
    </div>
  `;
}

function openCollectionDialog(collection?: Collection) {
  editingCollectionId = collection?.id ?? null;
  $("#collectionModalTitle").textContent = collection ? "Sammlung umbenennen" : "Sammlung erstellen";
  const input = $<HTMLInputElement>("#collectionNameInput");
  input.value = collection?.name ?? "";
  showModal("#collectionModal");
  queueMicrotask(() => {
    input.focus();
    input.select();
  });
}

async function saveCollection() {
  if (busy) return;
  const input = $<HTMLInputElement>("#collectionNameInput");
  const name = input.value.trim();
  if (!name) {
    setStatus("Sammlungsname darf nicht leer sein");
    return;
  }

  setBusy(true, editingCollectionId === null ? "Sammlung wird erstellt..." : "Sammlung wird umbenannt...");
  try {
    if (editingCollectionId === null) {
      const collection = await invoke<Collection>("create_collection", { name });
      collections = [...collections, collection].sort(compareCollections);
      activeCollectionId = collection.id;
    } else {
      const updated = await invoke<Collection>("update_collection", { id: editingCollectionId, name });
      collections = collections.map((collection) => (collection.id === updated.id ? updated : collection)).sort(compareCollections);
    }
    hideModal("#collectionModal");
    renderCollectionList();
    renderVideoList();
    setStatus("Sammlung gespeichert");
  } catch (error) {
    setStatus(errorMessage(error));
  } finally {
    setBusy(false);
  }
}

async function deleteCollection(collection: Collection) {
  if (!(await confirmDialog(`Sammlung "${collection.name}" löschen? Die Videos bleiben erhalten.`, {
    title: "Sammlung löschen",
    okLabel: "Löschen",
  }))) {
    return;
  }

  setBusy(true, "Sammlung wird gelöscht...");
  try {
    await invoke<void>("delete_collection", { id: collection.id });
    collections = collections.filter((item) => item.id !== collection.id);
    videos = videos.map((video) => ({
      ...video,
      collection_ids: video.collection_ids.filter((id) => id !== collection.id),
    }));
    if (activeCollectionId === collection.id) activeCollectionId = null;
    renderCollectionList();
    renderVideoList();
    const active = getActiveVideo();
    if (active) renderVideoCollections(active);
    setStatus("Sammlung gelöscht");
  } catch (error) {
    setStatus(errorMessage(error));
  } finally {
    setBusy(false);
  }
}

async function loadCollections() {
  collections = await invoke<Collection[]>("get_collections");
  renderCollectionList();
}

function renderVideoList() {
  if (!videos.length) {
    videoList.innerHTML = '<p class="empty-list">Noch keine Videos</p>';
    return;
  }

  const filteredVideos = getFilteredVideos();
  if (!filteredVideos.length) {
    videoList.innerHTML = '<p class="empty-list">Keine passenden Videos</p>';
    return;
  }

  videoList.innerHTML = filteredVideos
    .map((video) => {
      const activeClass = activeVideoId === video.id ? " active" : "";
      const thumb = video.thumbnail || video.thumbnail_url;
      return `
        <button class="video-item${activeClass}" data-id="${video.id}">
          <img src="${escapeHtml(thumb)}" alt="" loading="lazy" />
          <span class="info">
            <span class="title">${escapeHtml(video.title)}</span>
            <span class="meta">
              ${renderVideoStatusChip("T", !!video.transcript, "Transkript")}
              ${renderVideoStatusChip("Z", !!video.summary, "Zusammenfassung")}
            </span>
          </span>
        </button>
      `;
    })
    .join("");

  videoList.querySelectorAll<HTMLButtonElement>(".video-item").forEach((item) => {
    item.addEventListener("click", () => {
      const id = Number(item.dataset.id);
      if (!Number.isNaN(id)) {
        void selectVideo(id);
      }
    });
  });
}

function renderVideoFilters() {
  document.querySelectorAll<HTMLButtonElement>(".filter-chip").forEach((button) => {
    button.classList.toggle("active", button.dataset.videoFilter === videoStatusFilter);
  });
}

function getFilteredVideos(): Video[] {
  const normalizedQuery = normalizeSearch(videoSearchQuery);
  return videos.filter(
    (video) =>
      matchesActiveCollection(video) && matchesVideoStatusFilter(video) && matchesVideoSearch(video, normalizedQuery),
  );
}

function matchesActiveCollection(video: Video): boolean {
  return activeCollectionId === null || video.collection_ids.includes(activeCollectionId);
}

function matchesVideoStatusFilter(video: Video): boolean {
  switch (videoStatusFilter) {
    case "transcript":
      return !!video.transcript;
    case "missing-transcript":
      return !video.transcript;
    case "summary":
      return !!video.summary;
    case "missing-summary":
      return !video.summary;
    case "all":
      return true;
  }
}

function matchesVideoSearch(video: Video, normalizedQuery: string): boolean {
  if (!normalizedQuery) return true;
  return [video.title, video.url, video.video_id, video.published_at]
    .filter((value): value is string => !!value)
    .some((value) => normalizeSearch(value).includes(normalizedQuery));
}

function renderVideoStatusChip(label: string, available: boolean, title: string): string {
  const stateClass = available ? " available" : "";
  const status = available ? "vorhanden" : "fehlt";
  return `<span class="status-chip${stateClass}" title="${title} ${status}">${label}</span>`;
}

function showDetail(video: Video) {
  detailPlaceholder.hidden = true;
  detailContent.hidden = false;
  $<HTMLImageElement>("#detailThumb").src = video.thumbnail || video.thumbnail_url;
  $("#detailTitle").textContent = video.title;
  const detailUrl = $<HTMLAnchorElement>("#detailUrl");
  detailUrl.href = video.url;
  detailUrl.textContent = video.url;
  const publishedMeta = $("#detailPublishedMeta");
  if (video.published_at) {
    publishedMeta.textContent = `Veröffentlicht: ${formatDate(video.published_at)}`;
    publishedMeta.hidden = false;
  } else {
    publishedMeta.textContent = "";
    publishedMeta.hidden = true;
  }
  const summaryMeta = $("#detailSummaryMeta");
  if (video.summary && (video.summary_provider || video.summary_model)) {
    const parts = [video.summary_provider, video.summary_model].filter((part): part is string => !!part);
    summaryMeta.textContent = `Zusammengefasst mit: ${parts.join(" / ")}`;
    summaryMeta.hidden = false;
  } else {
    summaryMeta.textContent = "";
    summaryMeta.hidden = true;
  }
  const videoFallbackLink = $<HTMLAnchorElement>("#videoFallbackLink");
  videoFallbackLink.href = video.url;
  $("#tabTranscript").innerHTML = renderTranscript(video.transcript, video.chapters);
  $("#tabSummary").innerHTML = video.summary
    ? markdownToHtml(video.summary)
    : '<p class="empty">Noch keine Zusammenfassung - klicke auf "Zusammenfassen lassen"</p>';
  $<HTMLIFrameElement>("#videoPlayer").src = buildYouTubeEmbedUrl(video.video_id);
  $<HTMLButtonElement>("#reloadTranscriptBtn").hidden = !!video.transcript;
  renderVideoCollections(video);
  renderChapters(video.chapters);
  switchTab(activeTab);
}

function renderVideoCollections(video: Video) {
  const container = $("#collectionAssignment");
  if (!collections.length) {
    container.innerHTML = `
      <span class="collection-assignment-label">Sammlungen</span>
      <button class="inline-action" id="detailCreateCollection">Erste Sammlung erstellen</button>
    `;
    $("#detailCreateCollection").addEventListener("click", () => openCollectionDialog());
    return;
  }

  const selected = new Set(video.collection_ids);
  container.innerHTML = `
    <span class="collection-assignment-label">Sammlungen</span>
    <div class="collection-checkboxes">
      ${collections
        .map(
          (collection) => `
            <label class="collection-checkbox">
              <input type="checkbox" value="${collection.id}" ${selected.has(collection.id) ? "checked" : ""} />
              <span>${escapeHtml(collection.name)}</span>
            </label>
          `,
        )
        .join("")}
    </div>
  `;

  container.querySelectorAll<HTMLInputElement>('input[type="checkbox"]').forEach((input) => {
    input.addEventListener("change", () => void updateActiveVideoCollections());
  });
}

async function updateActiveVideoCollections() {
  const video = getActiveVideo();
  if (!video || busy) return;
  const ids = Array.from(document.querySelectorAll<HTMLInputElement>('#collectionAssignment input[type="checkbox"]:checked'))
    .map((input) => Number(input.value))
    .filter((id) => !Number.isNaN(id));

  setBusy(true, "Sammlungen werden gespeichert...");
  try {
    const updated = await invoke<Video>("set_video_collections", {
      videoId: video.id,
      collectionIds: ids,
    });
    videos = videos.map((item) => (item.id === updated.id ? updated : item));
    await loadCollections();
    renderVideoList();
    renderVideoCollections(updated);
    setStatus("Sammlungen gespeichert");
  } catch (error) {
    setStatus(errorMessage(error));
    renderVideoCollections(video);
  } finally {
    setBusy(false);
  }
}

function renderTranscript(raw?: string | null, chapters?: Chapter[] | null): string {
  if (!raw) return '<p class="empty">Kein Transkript verfügbar</p>';
  let snippets: TranscriptSnippet[];
  try {
    snippets = JSON.parse(raw) as TranscriptSnippet[];
  } catch {
    return `<p>${escapeHtml(raw).replace(/\n/g, "<br>")}</p>`;
  }

  let chapterIndex = 0;
  let html = "";
  for (const snippet of snippets) {
    while (chapters && chapterIndex < chapters.length && chapters[chapterIndex].start <= snippet.start) {
      const chapter = chapters[chapterIndex];
      html += `<button class="ts-chapter" data-start="${chapter.start}">${escapeHtml(chapter.title)}</button>`;
      chapterIndex += 1;
    }
    html += `<div class="ts-line"><span class="ts-time">${escapeHtml(snippet.time)}</span>${escapeHtml(snippet.text)}</div>`;
  }
  return html || '<p class="empty">Transkript ist leer</p>';
}

function renderChapters(chapters?: Chapter[] | null) {
  if (!chapters || chapters.length === 0) {
    chaptersPanel.hidden = true;
    return;
  }

  chaptersPanel.hidden = false;
  chaptersList.innerHTML = chapters
    .map((chapter) => `
      <button class="chapter-item" data-start="${chapter.start}">
        <span class="ts-time">${escapeHtml(chapter.time)}</span>
        ${escapeHtml(chapter.title)}
      </button>
    `)
    .join("");

  chaptersList.querySelectorAll<HTMLButtonElement>(".chapter-item").forEach((item) => {
    item.addEventListener("click", () => {
      const start = Number(item.dataset.start);
      if (!Number.isNaN(start)) {
        seekVideo(start);
      }
    });
  });
}

function seekVideo(seconds: number) {
  const video = getActiveVideo();
  if (!video) return;
  $<HTMLIFrameElement>("#videoPlayer").src = buildYouTubeEmbedUrl(video.video_id, seconds);
  switchTab("video");
}

function buildYouTubeEmbedUrl(videoId: string, startSeconds?: number): string {
  const url = new URL(`https://www.youtube.com/embed/${encodeURIComponent(videoId)}`);
  url.searchParams.set("rel", "0");

  if (window.location.origin.startsWith("http")) {
    url.searchParams.set("origin", window.location.origin);
    url.searchParams.set("widget_referrer", window.location.origin);
  }

  if (startSeconds !== undefined) {
    url.searchParams.set("start", Math.floor(startSeconds).toString());
    url.searchParams.set("autoplay", "1");
  }

  return url.toString();
}

function switchTab(tab: TabName) {
  activeTab = tab;
  document.querySelectorAll<HTMLButtonElement>(".tab").forEach((button) => {
    button.classList.toggle("active", button.dataset.tab === tab);
  });
  document.querySelectorAll<HTMLDivElement>(".tabPanel").forEach((panel) => {
    panel.classList.toggle("active", panel.id === `tab${capitalize(tab)}`);
  });
}

function buildSummaryPrompt(): string {
  const detail = $<HTMLSelectElement>("#summaryDetail").value;
  const lang = $<HTMLSelectElement>("#summaryLang").value;
  const useChapters = $<HTMLSelectElement>("#summaryUseChapters").value;
  const languageNames: Record<string, string> = {
    original: "the same language as the transcript",
    german: "German",
    english: "English",
    french: "French",
    spanish: "Spanish",
    italian: "Italian",
  };

  const lines = ["You are a helpful assistant that summarizes YouTube video transcripts.", ""];
  if (detail === "short") {
    lines.push("Provide a very concise summary: just 3-5 bullet points with the key takeaways.");
  } else if (detail === "detailed") {
    lines.push("Provide a comprehensive and detailed summary.");
    lines.push("Include all main topics, key arguments, facts, insights, conclusions and takeaways.");
  } else {
    lines.push("Provide a clear, structured summary with overview, key points and takeaways.");
  }
  lines.push("", `Write the summary in ${languageNames[lang]}.`);
  if (useChapters === "yes") {
    lines.push("If chapter markers are provided, structure the summary by chapter.");
  }
  lines.push("", "Format your response as Markdown.");
  return lines.join("\n");
}

function updateSummaryPrompt() {
  $<HTMLTextAreaElement>("#summaryPrompt").value = buildSummaryPrompt();
}

// Some models wrap their entire Markdown reply in a single ```markdown ... ```
// code fence. marked would then render the whole summary as one <pre><code>
// block, showing the raw Markdown source. Strip such a wrapping fence before
// parsing. Only strips when the fence wraps the complete text and the content
// itself contains no further fences, so genuine code blocks stay intact.
function stripWrappingCodeFence(markdown: string): string {
  const trimmed = markdown.trim();
  const match = /^```([^\n]*)\n([\s\S]*?)\n?```$/.exec(trimmed);
  if (!match) return markdown;
  const info = match[1].trim().toLowerCase();
  if (info !== "" && info !== "markdown" && info !== "md") return markdown;
  const inner = match[2];
  if (/^[ \t]*```/m.test(inner)) return markdown;
  return inner;
}

function markdownToHtml(markdown: string): string {
  const rendered = marked.parse(stripWrappingCodeFence(markdown), { async: false }) as string;
  return DOMPurify.sanitize(rendered, {
    ADD_ATTR: ["target", "rel"],
  });
}

function getActiveVideo(): Video | null {
  return videos.find((video) => video.id === activeVideoId) ?? null;
}

function setBusy(value: boolean, message?: string) {
  busy = value;
  $<HTMLButtonElement>("#addBtn").disabled = value;
  $<HTMLButtonElement>("#summarizeBtn").disabled = value;
  $<HTMLButtonElement>("#reloadTranscriptBtn").disabled = value;
  $<HTMLButtonElement>("#deleteBtn").disabled = value;
  document.querySelectorAll<HTMLButtonElement>(".collection-action, #addCollectionBtn, #collectionSave").forEach((button) => {
    button.disabled = value;
  });
  document.querySelectorAll<HTMLInputElement>('#collectionAssignment input[type="checkbox"]').forEach((input) => {
    input.disabled = value;
  });
  if (message) {
    setStatus(message);
  }
}

function setStatus(message: string) {
  statusText.textContent = message;
}

function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleDateString(undefined, { year: "numeric", month: "long", day: "numeric" });
}

function normalizeSearch(value: string): string {
  return value.toLocaleLowerCase().normalize("NFKD").replace(/[\u0300-\u036f]/g, "");
}

function isVideoStatusFilter(value: string | undefined): value is VideoStatusFilter {
  return (
    value === "all" ||
    value === "transcript" ||
    value === "missing-transcript" ||
    value === "summary" ||
    value === "missing-summary"
  );
}

function compareCollections(a: Collection, b: Collection): number {
  return a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
}

function capitalize(value: string): string {
  return value.charAt(0).toUpperCase() + value.slice(1);
}
