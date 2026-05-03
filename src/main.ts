import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { marked } from "marked";
import DOMPurify from "dompurify";
import "./styles.css";
import {
  applyConfig,
  bindAiConfigEvents,
  initAiConfig,
  refreshModelsForProvider,
  setProviders,
  type AiConfig,
  type AiProviderInfo,
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
  created_at: string;
  updated_at: string;
};

type TabName = "transcript" | "summary" | "video";

let videos: Video[] = [];
let activeVideoId: number | null = null;
let activeTab: TabName = "transcript";
let busy = false;
let videoSearchQuery = "";
let videoStatusFilter: VideoStatusFilter = "all";

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
      <div class="settings-sidebar">
        <div class="settings-title">
          <h2>KI</h2>
          <p>Connect providers and choose the model used for summaries.</p>
        </div>
        <div id="settingsSelectedModel"></div>
        <div id="providerSettingsList"></div>
      </div>
      <div class="settings-main">
        <div class="settings-nav">
          <button class="settings-nav-item active" data-settings-section="providers">Provider Details</button>
          <button class="settings-nav-item" data-settings-section="models">All Models</button>
        </div>
        <div id="providerSettingsBody"></div>
      </div>
      <div class="modal-actions">
        <button id="configClose">Schließen</button>
      </div>
    </div>
  </div>

  <div id="summaryModal" class="modal" hidden>
    <div class="modal-content modal-wide">
      <h2>Zusammenfassung konfigurieren</h2>
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

  $("#summarizeBtn").addEventListener("click", openSummaryDialog);
  $("#reloadTranscriptBtn").addEventListener("click", () => void refreshActiveTranscript());
  $("#summaryStart").addEventListener("click", () => void startSummary());
  $("#summaryCancel").addEventListener("click", () => hideModal("#summaryModal"));

  ["#summaryDetail", "#summaryLang", "#summaryUseChapters"].forEach((selector) => {
    $(selector).addEventListener("change", updateSummaryPrompt);
  });

  $("#deleteBtn").addEventListener("click", () => void deleteActiveVideo());

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

async function loadInitialData() {
  setBusy(true, "Videos werden geladen...");
  try {
    const [loadedVideos, config, providers] = await Promise.all([
      invoke<Video[]>("get_videos"),
      invoke<AiConfig>("get_config"),
      invoke<AiProviderInfo[]>("get_ai_providers"),
    ]);
    videos = loadedVideos;
    setProviders(providers);
    renderVideoList();
    applyConfig(config);
    void refreshModelsForProvider(config.provider, true);
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
  if (activeVideoId === null) return;
  if (!(await confirmDialog("Video wirklich löschen?", { title: "Video löschen", okLabel: "Löschen" }))) return;
  const id = activeVideoId;
  setBusy(true, "Video wird gelöscht...");
  try {
    await invoke<void>("delete_video", { id });
    videos = videos.filter((video) => video.id !== id);
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


function openSummaryDialog() {
  const video = getActiveVideo();
  if (!video) return;
  if (!video.transcript) {
    setStatus("Kein Transkript vorhanden - bitte Video neu hinzufügen");
    return;
  }
  loadSummarySettings();
  updateSummaryPrompt();
  showModal("#summaryModal");
}

const SUMMARY_SETTINGS_KEY = "summarySettings";

type SummarySettings = {
  detail: string;
  lang: string;
  useChapters: string;
};

function loadSummarySettings() {
  try {
    const raw = localStorage.getItem(SUMMARY_SETTINGS_KEY);
    if (!raw) return;
    const saved = JSON.parse(raw) as Partial<SummarySettings>;
    if (saved.detail) $<HTMLSelectElement>("#summaryDetail").value = saved.detail;
    if (saved.lang) $<HTMLSelectElement>("#summaryLang").value = saved.lang;
    if (saved.useChapters) $<HTMLSelectElement>("#summaryUseChapters").value = saved.useChapters;
  } catch {
    // ignore corrupt entries
  }
}

function saveSummarySettings() {
  const settings: SummarySettings = {
    detail: $<HTMLSelectElement>("#summaryDetail").value,
    lang: $<HTMLSelectElement>("#summaryLang").value,
    useChapters: $<HTMLSelectElement>("#summaryUseChapters").value,
  };
  localStorage.setItem(SUMMARY_SETTINGS_KEY, JSON.stringify(settings));
}

async function startSummary() {
  const video = getActiveVideo();
  if (!video || busy) return;

  saveSummarySettings();
  setBusy(true, "Zusammenfassung wird erstellt...");
  hideModal("#summaryModal");
  try {
    const updated = await invoke<Video>("summarize_video", {
      id: video.id,
      systemPrompt: $<HTMLTextAreaElement>("#summaryPrompt").value.trim(),
    });
    videos = videos.map((item) => (item.id === updated.id ? updated : item));
    showDetail(updated);
    switchTab("summary");
    setStatus("Zusammenfassung fertig");
  } catch (error) {
    setStatus(errorMessage(error));
  } finally {
    setBusy(false);
  }
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
  return videos.filter((video) => matchesVideoStatusFilter(video) && matchesVideoSearch(video, normalizedQuery));
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
  renderChapters(video.chapters);
  switchTab(activeTab);
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

function markdownToHtml(markdown: string): string {
  const rendered = marked.parse(markdown, { async: false }) as string;
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

function capitalize(value: string): string {
  return value.charAt(0).toUpperCase() + value.slice(1);
}
