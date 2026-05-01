import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

type AiConfig = {
  provider: string;
  api_key: string;
  model: string;
  endpoint_override?: string | null;
};

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
  created_at: string;
  updated_at: string;
};

type TabName = "transcript" | "summary" | "video";

let videos: Video[] = [];
let activeVideoId: number | null = null;
let activeTab: TabName = "transcript";
let busy = false;

const app = document.querySelector<HTMLDivElement>("#app");
if (!app) {
  throw new Error("App container not found");
}

app.innerHTML = `
  <header>
    <h1>YouTube Summarizer</h1>
    <button id="settingsBtn" class="icon-btn" title="Einstellungen" aria-label="Einstellungen">⚙</button>
  </header>

  <div id="addBar">
    <input id="urlInput" type="text" placeholder="YouTube-URL oder Video-ID eingeben..." />
    <button id="addBtn">Hinzufügen</button>
  </div>

  <main>
    <aside>
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
          </div>
          <button id="deleteBtn" class="icon-btn danger" title="Video entfernen" aria-label="Video entfernen">×</button>
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
            <iframe id="videoPlayer" title="YouTube Video" allow="autoplay; encrypted-media; picture-in-picture; fullscreen"></iframe>
          </div>
        </div>
      </div>

      <div id="chaptersPanel" class="chapters-panel" hidden>
        <h3>Kapitel</h3>
        <div id="chaptersList"></div>
      </div>
    </section>
  </main>

  <footer>
    <span id="statusText">Bereit</span>
    <span id="statusModel"></span>
  </footer>

  <div id="settingsModal" class="modal" hidden>
    <div class="modal-content">
      <h2>KI-Einstellungen</h2>
      <label>Provider
        <select id="configProvider">
          <option value="opencode_go">OpenCode Go</option>
          <option value="opencode_zen">OpenCode Zen</option>
          <option value="openrouter">OpenRouter</option>
          <option value="ollama">Ollama lokal</option>
        </select>
      </label>
      <label>API-Key
        <input type="password" id="configApiKey" />
      </label>
      <label>Modell
        <input type="text" id="configModel" placeholder="z.B. qwen3.5-plus" />
      </label>
      <label>Endpoint Override
        <input type="text" id="configEndpoint" placeholder="Optional" />
      </label>
      <div class="modal-actions">
        <button id="configSave">Speichern</button>
        <button id="configCancel">Abbrechen</button>
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
`;

const $ = <T extends HTMLElement>(selector: string): T => {
  const element = document.querySelector<T>(selector);
  if (!element) {
    throw new Error(`Missing element: ${selector}`);
  }
  return element;
};

const videoList = $<HTMLDivElement>("#videoList");
const detailPlaceholder = $<HTMLDivElement>("#detailPlaceholder");
const detailContent = $<HTMLDivElement>("#detailContent");
const chaptersPanel = $<HTMLDivElement>("#chaptersPanel");
const chaptersList = $<HTMLDivElement>("#chaptersList");
const statusText = $<HTMLSpanElement>("#statusText");
const statusModel = $<HTMLSpanElement>("#statusModel");

bindEvents();
void loadInitialData();

function bindEvents() {
  $("#addBtn").addEventListener("click", () => void addVideo());
  $("#urlInput").addEventListener("keydown", (event) => {
    if (event instanceof KeyboardEvent && event.key === "Enter") {
      void addVideo();
    }
  });

  $("#settingsBtn").addEventListener("click", () => void openSettings());
  $("#configSave").addEventListener("click", () => void saveSettings());
  $("#configCancel").addEventListener("click", () => hideModal("#settingsModal"));

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
    const [loadedVideos, config] = await Promise.all([
      invoke<Video[]>("get_videos"),
      invoke<AiConfig>("get_config"),
    ]);
    videos = loadedVideos;
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
  if (activeVideoId === null || !confirm("Video wirklich löschen?")) return;
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

async function openSettings() {
  try {
    applyConfig(await invoke<AiConfig>("get_config"));
    showModal("#settingsModal");
  } catch (error) {
    setStatus(errorMessage(error));
  }
}

async function saveSettings() {
  try {
    const config = await invoke<AiConfig>("save_config", {
      provider: $<HTMLSelectElement>("#configProvider").value,
      apiKey: $<HTMLInputElement>("#configApiKey").value,
      model: $<HTMLInputElement>("#configModel").value,
      endpointOverride: $<HTMLInputElement>("#configEndpoint").value,
    });
    applyConfig(config);
    hideModal("#settingsModal");
    setStatus("Konfiguration gespeichert");
  } catch (error) {
    setStatus(errorMessage(error));
  }
}

function applyConfig(config: AiConfig) {
  $<HTMLSelectElement>("#configProvider").value = config.provider;
  $<HTMLInputElement>("#configApiKey").value = config.api_key;
  $<HTMLInputElement>("#configModel").value = config.model;
  $<HTMLInputElement>("#configEndpoint").value = config.endpoint_override ?? "";
  statusModel.textContent = `Provider: ${config.provider} / Modell: ${config.model}`;
}

function openSummaryDialog() {
  const video = getActiveVideo();
  if (!video) return;
  if (!video.transcript) {
    setStatus("Kein Transkript vorhanden - bitte Video neu hinzufügen");
    return;
  }
  updateSummaryPrompt();
  showModal("#summaryModal");
}

async function startSummary() {
  const video = getActiveVideo();
  if (!video || busy) return;

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

  videoList.innerHTML = videos
    .map((video) => {
      const activeClass = activeVideoId === video.id ? " active" : "";
      const transcript = video.transcript ? "T" : "";
      const summary = video.summary ? "Z" : "";
      const thumb = video.thumbnail || video.thumbnail_url;
      return `
        <button class="video-item${activeClass}" data-id="${video.id}">
          <img src="${escapeHtml(thumb)}" alt="" loading="lazy" />
          <span class="info">
            <span class="title">${escapeHtml(video.title)}</span>
            <span class="meta">${escapeHtml(transcript)} ${escapeHtml(summary)}</span>
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

function showDetail(video: Video) {
  detailPlaceholder.hidden = true;
  detailContent.hidden = false;
  $<HTMLImageElement>("#detailThumb").src = video.thumbnail || video.thumbnail_url;
  $("#detailTitle").textContent = video.title;
  const detailUrl = $<HTMLAnchorElement>("#detailUrl");
  detailUrl.href = video.url;
  detailUrl.textContent = video.url;
  $("#tabTranscript").innerHTML = renderTranscript(video.transcript, video.chapters);
  $("#tabSummary").innerHTML = video.summary
    ? markdownToHtml(video.summary)
    : '<p class="empty">Noch keine Zusammenfassung - klicke auf "Zusammenfassen lassen"</p>';
  $<HTMLIFrameElement>("#videoPlayer").src = `https://www.youtube-nocookie.com/embed/${encodeURIComponent(video.video_id)}`;
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
    detailContent.classList.remove("with-chapters");
    return;
  }

  chaptersPanel.hidden = false;
  detailContent.classList.add("with-chapters");
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
  $<HTMLIFrameElement>("#videoPlayer").src = `https://www.youtube-nocookie.com/embed/${encodeURIComponent(video.video_id)}?start=${Math.floor(seconds)}&autoplay=1`;
  switchTab("video");
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
  const lines = escapeHtml(markdown).split(/\r?\n/);
  let html = "";
  let listType: "ul" | "ol" | null = null;

  const closeList = () => {
    if (listType) {
      html += `</${listType}>`;
      listType = null;
    }
  };

  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed) {
      closeList();
      continue;
    }

    const heading = /^(#{1,4})\s+(.+)$/.exec(trimmed);
    if (heading) {
      closeList();
      const level = heading[1].length;
      html += `<h${level}>${inlineMarkdown(heading[2])}</h${level}>`;
      continue;
    }

    const bullet = /^[-*]\s+(.+)$/.exec(trimmed);
    if (bullet) {
      if (listType !== "ul") {
        closeList();
        html += "<ul>";
        listType = "ul";
      }
      html += `<li>${inlineMarkdown(bullet[1])}</li>`;
      continue;
    }

    const numbered = /^\d+\.\s+(.+)$/.exec(trimmed);
    if (numbered) {
      if (listType !== "ol") {
        closeList();
        html += "<ol>";
        listType = "ol";
      }
      html += `<li>${inlineMarkdown(numbered[1])}</li>`;
      continue;
    }

    closeList();
    html += `<p>${inlineMarkdown(trimmed)}</p>`;
  }

  closeList();
  return html;
}

function inlineMarkdown(value: string): string {
  return value
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/\*([^*]+)\*/g, "<em>$1</em>");
}

function showModal(selector: string) {
  $(selector).hidden = false;
}

function hideModal(selector: string) {
  $(selector).hidden = true;
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

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function escapeHtml(value: unknown): string {
  return String(value ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

function capitalize(value: string): string {
  return value.charAt(0).toUpperCase() + value.slice(1);
}
