import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { marked } from "marked";
import DOMPurify from "dompurify";
import "./styles.css";

marked.setOptions({ gfm: true, breaks: false });

type AiConfig = {
  provider: string;
  api_key: string;
  model: string;
  endpoint_override?: string | null;
  providers: AiProviderConfig[];
};

type AiProviderConfig = {
  id: string;
  name?: string | null;
  enabled: boolean;
  api_key_required?: boolean;
  api_key: string;
  model: string;
  endpoint_override?: string | null;
  models: AiModel[];
  models_updated_at?: string | null;
  last_error?: string | null;
  account_tier?: AccountTier | null;
};

type AccountTier = "free" | "pro" | "max";

type AiModel = {
  id: string;
  name: string;
  tags: string[];
  free: boolean;
  availability?: "free" | "subscription_required" | "unknown" | null;
};

type AiProviderInfo = {
  id: string;
  name: string;
  description: string;
  badge: string;
  homepage_url?: string | null;
  default_endpoint?: string | null;
  requires_api_key: boolean;
  supports_model_refresh: boolean;
  endpoint_editable: boolean;
  recommended: boolean;
};

type ModelEntry = {
  provider: AiProviderConfig;
  providerInfo?: AiProviderInfo;
  model: AiModel;
};

type ChatTestMessage = {
  role: "user" | "assistant";
  content: string;
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
let aiConfig: AiConfig | null = null;
let aiProviders: AiProviderInfo[] = [];
let selectedSettingsProviderId = "opencode_go";
let settingsSection: "providers" | "models" = "providers";
const FREE_MODELS_ONLY_KEY = "settings.freeModelsOnly";
let showOnlyFreeModels = localStorage.getItem(FREE_MODELS_ONLY_KEY) === "true";
const revealedApiKeys = new Set<string>();
let chatTestTarget: { providerId: string; modelId: string } | null = null;
let chatTestMessages: ChatTestMessage[] = [];

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

const $ = <T extends HTMLElement>(selector: string): T => {
  const element = document.querySelector<T>(selector);
  if (!element) {
    throw new Error(`Missing element: ${selector}`);
  }
  return element;
};

type ConfirmOptions = {
  title?: string;
  okLabel?: string;
  cancelLabel?: string;
};

function confirmDialog(message: string, options: ConfirmOptions = {}): Promise<boolean> {
  const modal = $<HTMLDivElement>("#confirmModal");
  const titleEl = $<HTMLHeadingElement>("#confirmTitle");
  const messageEl = $<HTMLParagraphElement>("#confirmMessage");
  const okBtn = $<HTMLButtonElement>("#confirmOk");
  const cancelBtn = $<HTMLButtonElement>("#confirmCancel");

  titleEl.textContent = options.title ?? "Bestätigen";
  messageEl.textContent = message;
  okBtn.textContent = options.okLabel ?? "OK";
  cancelBtn.textContent = options.cancelLabel ?? "Abbrechen";

  return new Promise<boolean>((resolve) => {
    const cleanup = (result: boolean) => {
      modal.hidden = true;
      okBtn.removeEventListener("click", onOk);
      cancelBtn.removeEventListener("click", onCancel);
      modal.removeEventListener("click", onBackdrop);
      document.removeEventListener("keydown", onKey);
      resolve(result);
    };
    const onOk = () => cleanup(true);
    const onCancel = () => cleanup(false);
    const onBackdrop = (e: MouseEvent) => {
      if (e.target === modal) cleanup(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") cleanup(false);
      else if (e.key === "Enter") cleanup(true);
    };

    okBtn.addEventListener("click", onOk);
    cancelBtn.addEventListener("click", onCancel);
    modal.addEventListener("click", onBackdrop);
    document.addEventListener("keydown", onKey);

    modal.hidden = false;
    queueMicrotask(() => okBtn.focus());
  });
}

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
  $("#configClose").addEventListener("click", () => hideModal("#settingsModal"));
  document.querySelectorAll<HTMLButtonElement>(".settings-nav-item").forEach((button) => {
    button.addEventListener("click", () => {
      const section = button.dataset.settingsSection;
      if (section === "providers" || section === "models") {
        void persistVisibleProviderForm();
        settingsSection = section;
        renderSettings();
      }
    });
  });
  $("#providerSettingsList").addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const addProvider = target.closest<HTMLElement>("[data-add-provider]");
    if (addProvider) {
      void addCustomProvider();
      return;
    }
    if (target.closest(".provider-toggle")) {
      return;
    }
    const deleteButton = target.closest<HTMLElement>("[data-delete-provider-id]");
    if (deleteButton?.dataset.deleteProviderId) {
      void deleteCustomProvider(deleteButton.dataset.deleteProviderId);
      return;
    }
    const item = target.closest<HTMLElement>("[data-provider-id]");
    if (item?.dataset.providerId) {
      selectSettingsProvider(item.dataset.providerId);
    }
  });
  $("#providerSettingsList").addEventListener("change", (event) => {
    const target = event.target;
    if (!(target instanceof HTMLInputElement) || !target.dataset.toggleProviderId) return;
    void setProviderEnabled(target.dataset.toggleProviderId, target.checked);
  });
  $("#providerSettingsBody").addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    if (target.closest("#openModelPickerBtn")) {
      settingsSection = "models";
      renderSettings();
    }
    if (target.closest("#settingsRefreshModelsBtn")) {
      void refreshModelsForProvider(selectedSettingsProviderId);
    }
    if (target.closest("#settingsReprobeBtn")) {
      void refreshModelsForProvider(selectedSettingsProviderId, false, true);
    }
    if (target.closest("#toggleApiKeyVisibility")) {
      toggleApiKeyVisibility(selectedSettingsProviderId);
    }
    if (target.closest("#refreshAllModelsBtn")) {
      void refreshAllModels();
    }
    const chatButton = target.closest<HTMLElement>("[data-test-chat-model-id][data-test-chat-provider-id]");
    if (chatButton?.dataset.testChatModelId && chatButton.dataset.testChatProviderId) {
      openChatTestDialog(chatButton.dataset.testChatProviderId, chatButton.dataset.testChatModelId);
      return;
    }
    const modelItem = target.closest<HTMLElement>("[data-model-id][data-model-provider-id]");
    if (modelItem?.dataset.modelId && modelItem.dataset.modelProviderId) {
      const { modelId, modelProviderId } = modelItem.dataset;
      void (async () => {
        if (!$("#settingsModal").hidden && document.querySelector("#configModel")) {
          await persistVisibleProviderForm();
        }
        await selectGlobalModel(modelProviderId, modelId);
      })();
    }
  });
  $("#providerSettingsBody").addEventListener("input", (event) => {
    const target = event.target;
    if (!(target instanceof HTMLInputElement)) return;
    if (target.id === "globalModelSearch") renderSettingsModelList();
  });
  $("#providerSettingsBody").addEventListener("change", (event) => {
    const target = event.target;
    if (target instanceof HTMLSelectElement && target.id === "configAccountTier") {
      void persistVisibleProviderForm(true)
        .then(() => {
          setStatus("Konfiguration gespeichert");
          renderSettings();
        })
        .catch((err) => setStatus(errorMessage(err)));
      return;
    }
    if (!(target instanceof HTMLInputElement)) return;
  if (target.id === "freeModelsOnly") {
      showOnlyFreeModels = target.checked;
      localStorage.setItem(FREE_MODELS_ONLY_KEY, String(showOnlyFreeModels));
      renderSettingsModelList();
      return;
    }
    if (
      target.id === "configApiKey" ||
      target.id === "configEndpoint" ||
      target.id === "configProviderName" ||
      target.id === "configApiKeyRequired"
    ) {
      void persistVisibleProviderForm(true)
        .then(() => setStatus("Konfiguration gespeichert"))
        .catch((err) => setStatus(errorMessage(err)));
    }
  });

  $("#summarizeBtn").addEventListener("click", openSummaryDialog);
  $("#reloadTranscriptBtn").addEventListener("click", () => void refreshActiveTranscript());
  $("#summaryStart").addEventListener("click", () => void startSummary());
  $("#summaryCancel").addEventListener("click", () => hideModal("#summaryModal"));
  $("#chatTestSend").addEventListener("click", () => void sendChatTest());
  $("#chatTestClose").addEventListener("click", () => hideModal("#chatTestModal"));

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
    aiProviders = providers;
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

async function openSettings() {
  try {
    const [config, providers] = await Promise.all([
      invoke<AiConfig>("get_config"),
      invoke<AiProviderInfo[]>("get_ai_providers"),
    ]);
    aiProviders = providers;
    applyConfig(config);
    selectedSettingsProviderId = config.provider;
    renderSettings();
    showModal("#settingsModal");
    const active = getProviderConfig(config.provider);
    if (!active?.models.length) {
      void refreshModelsForProvider(config.provider, true);
    }
  } catch (error) {
    setStatus(errorMessage(error));
  }
}

function applyConfig(config: AiConfig) {
  aiConfig = config;
  const provider = getProviderInfo(config.provider);
  statusModel.textContent = `${provider?.name ?? config.provider} / ${config.model || "kein Modell"}`;
  if (!$("#settingsModal").hidden) {
    renderSettings();
  }
}

function renderSettings() {
  if (!aiConfig) return;
  document.querySelectorAll<HTMLButtonElement>(".settings-nav-item").forEach((button) => {
    button.classList.toggle("active", button.dataset.settingsSection === settingsSection);
  });
  $("#settingsSelectedModel").innerHTML = renderSidebarSelectedModel();
  $("#providerSettingsList").innerHTML = renderProviderNavigation(selectedSettingsProviderId);

  if (settingsSection === "models") {
    if (document.querySelector("#globalModelSearch")) {
      renderSettingsModelList();
    } else {
      renderModelSettings();
    }
    return;
  }

  const selected = getProviderConfig(selectedSettingsProviderId) ?? getProviderConfig(aiConfig.provider);
  if (!selected) return;
  const info = getProviderInfo(selected.id);
  const apiKeyVisible = revealedApiKeys.has(selected.id);
  const apiKeyRequired = providerRequiresApiKey(selected);

  $("#providerSettingsBody").innerHTML = `
    <div class="settings-fixed-panel">
      <div class="settings-provider-head">
        <div>
          <h2>${escapeHtml(providerDisplayName(selected))}</h2>
          <p>${escapeHtml(info?.description ?? "OpenAI-compatible custom provider.")}</p>
        </div>
        <span class="provider-head-actions">
          ${info?.homepage_url && info.recommended ? `<a class="provider-home-link" href="${escapeHtml(info.homepage_url)}">Provider website</a>` : ""}
          ${info?.recommended ? '<span class="provider-badge">Recommended</span>' : ""}
        </span>
      </div>
      <label class="field-row" ${isCustomProvider(selected.id) ? "" : "hidden"}><span class="field-label">Name</span>
        <input id="configProviderName" type="text" value="${escapeHtml(providerDisplayName(selected))}" placeholder="Custom provider name" />
      </label>
      <label class="field-row toggle-row provider-api-key-required" ${isUserManagedProvider(selected.id) ? "" : "hidden"}>
        <span class="field-label">API key required</span>
        <span class="field-control"><input id="configApiKeyRequired" type="checkbox" ${apiKeyRequired ? "checked" : ""} /></span>
      </label>
      <label class="field-row"><span class="field-label">API key</span>
        <span class="secret-input-row">
          <input id="configApiKey" type="${apiKeyVisible ? "text" : "password"}" value="${escapeHtml(selected.api_key)}" placeholder="${apiKeyRequired ? "Required" : "Optional"}" />
          <button id="toggleApiKeyVisibility" class="icon-action-btn" type="button" title="${apiKeyVisible ? "Hide API key" : "Show API key"}" aria-label="${apiKeyVisible ? "Hide API key" : "Show API key"}">${apiKeyVisible ? "🙈" : "👁"}</button>
        </span>
      </label>
      <input id="configModel" type="hidden" value="${escapeHtml(selected.model)}" />
      <label class="field-row" ${info?.endpoint_editable || isCustomProvider(selected.id) ? "" : "hidden"}><span class="field-label">Chat endpoint</span>
        <input id="configEndpoint" type="text" value="${escapeHtml(selected.endpoint_override ?? "")}" placeholder="${escapeHtml(info?.default_endpoint ?? "https://example.com/v1/chat/completions")}" />
      </label>
      ${renderAccountTierField(selected)}
      <div class="provider-actions">
        <button id="settingsRefreshModelsBtn" type="button">Refresh models</button>
        ${selected.id === "ollama_cloud" && providerAccountTier(selected) === "free" ? '<button id="settingsReprobeBtn" type="button" title="Re-run availability probe for all models">Re-probe availability</button>' : ""}
        <span>${renderModelRefreshState(selected)}</span>
      </div>
      ${selected.last_error ? `<p class="settings-error">${escapeHtml(selected.last_error)}</p>` : ""}
    </div>
    <div class="settings-model-preview settings-scroll-list">
      ${renderModelPreview(selected)}
    </div>
  `;
}

function renderProviderNavigation(selectedId: string): string {
  const recommended = aiProviders.filter((provider) => provider.recommended);
  const customAndLocal = (aiConfig?.providers ?? []).filter((provider) => {
    const info = getProviderInfo(provider.id);
    return isCustomProvider(provider.id) || info?.id === "ollama" || !info;
  });
  return `
    <div class="provider-section-title">Recommended</div>
    ${recommended.map((provider) => renderProviderNavItem(provider.id, selectedId)).join("")}
    <div class="provider-section-divider"></div>
    <div class="provider-section-title">Custom / local</div>
    ${customAndLocal.map((provider) => renderProviderNavItem(provider.id, selectedId)).join("")}
    <button class="add-provider-card" data-add-provider="custom" type="button">
      <span>+</span>
      <strong>Add custom provider</strong>
    </button>
  `;
}

function renderProviderNavItem(providerId: string, selectedId: string): string {
  const config = getProviderConfig(providerId);
  const info = getProviderInfo(providerId);
  const active = selectedId === providerId ? " active" : "";
  const configured = config ? isProviderConfigured(config) : false;
  const enabled = !!config?.enabled && configured;
  const title = configured ? "Enable / disable provider" : "Configure provider first";
  return `
    <div class="provider-nav-row${active}">
      <button class="provider-nav-item" data-provider-id="${escapeHtml(providerId)}">
        <span class="provider-name-row">
          <span class="provider-status-dot ${providerStatusClass(config)}" title="${escapeHtml(providerStatusLabel(config))}"></span>
          <span class="provider-name">${escapeHtml(config ? providerDisplayName(config) : info?.name ?? providerId)}</span>
        </span>
        <span class="provider-meta">${escapeHtml(info?.badge ?? "Custom")}${!enabled ? " · disabled" : ""}${configured ? " · configured" : ""}</span>
      </button>
      ${config ? `<label class="provider-toggle nav-provider-toggle" title="${escapeHtml(title)}">
        <input type="checkbox" data-toggle-provider-id="${escapeHtml(providerId)}" ${enabled ? "checked" : ""} ${configured ? "" : "disabled"} />
        <span class="provider-toggle-track"><span class="provider-toggle-thumb"></span></span>
      </label>` : ""}
      ${isUserManagedProvider(providerId) ? `<button class="delete-icon-btn" data-delete-provider-id="${escapeHtml(providerId)}" title="Delete provider" aria-label="Delete provider">🗑</button>` : ""}
    </div>
  `;
}

function providerStatusClass(provider?: AiProviderConfig): string {
  if (!provider) return "unconfigured";
  if (provider.last_error) return "error";
  if (!isProviderConfigured(provider)) return "unconfigured";
  if (!provider.enabled) return "disabled";
  return "ready";
}

function providerStatusLabel(provider?: AiProviderConfig): string {
  if (!provider) return "Not configured";
  if (provider.last_error) return "Last check failed";
  if (!isProviderConfigured(provider)) return "Not configured";
  if (!provider.enabled) return "Configured but disabled";
  return "Ready";
}

async function addCustomProvider() {
  const config = await invoke<AiConfig>("add_custom_provider", { localOllama: false });
  applyConfig(config);
  const customProviders = config.providers.filter((provider) => isCustomProvider(provider.id));
  selectedSettingsProviderId = customProviders.at(-1)?.id ?? selectedSettingsProviderId;
  settingsSection = "providers";
  renderSettings();
}

async function setProviderEnabled(providerId: string, enabled: boolean) {
  const provider = getProviderConfig(providerId);
  if (!provider || !isProviderConfigured(provider)) return;
  const config = await invoke<AiConfig>("save_provider_config", {
    providerId,
    name: provider.name ?? null,
    enabled,
    apiKeyRequired: provider.api_key_required ?? false,
    apiKey: provider.api_key,
    model: provider.model,
    endpointOverride: provider.endpoint_override ?? "",
    activate: false,
  });
  applyConfig(config);
}

async function deleteCustomProvider(providerId: string) {
  if (!(await confirmDialog("Diesen Custom-Provider wirklich löschen?", { title: "Provider löschen", okLabel: "Löschen" }))) return;
  const config = await invoke<AiConfig>("delete_custom_provider", { providerId });
  applyConfig(config);
  selectedSettingsProviderId = config.provider;
  settingsSection = "providers";
  renderSettings();
  setStatus("Provider deleted");
}

function selectSettingsProvider(providerId: string) {
  void persistVisibleProviderForm();
  settingsSection = "providers";
  selectedSettingsProviderId = providerId;
  renderSettings();
}

async function persistVisibleProviderForm(reportErrors = false) {
  if (!aiConfig || $("#settingsModal").hidden || !document.querySelector("#configModel")) return;
  const apiKeyInput = document.querySelector<HTMLInputElement>("#configApiKey");
  const modelInput = document.querySelector<HTMLInputElement>("#configModel");
  const endpointInput = document.querySelector<HTMLInputElement>("#configEndpoint");
  const apiKeyRequiredInput = document.querySelector<HTMLInputElement>("#configApiKeyRequired");
  if (!apiKeyInput || !modelInput) return;
  const currentEnabled = getProviderConfig(selectedSettingsProviderId)?.enabled ?? true;
  const currentApiKeyRequired = isUserManagedProvider(selectedSettingsProviderId)
    ? (apiKeyRequiredInput?.checked ?? false)
    : providerRequiresApiKey(getProviderConfig(selectedSettingsProviderId));

  const tierSelect = document.querySelector<HTMLSelectElement>("#configAccountTier");
  const config = await invoke<AiConfig>("save_provider_config", {
    providerId: selectedSettingsProviderId,
    name: document.querySelector<HTMLInputElement>("#configProviderName")?.value ?? null,
    enabled: currentEnabled,
    apiKeyRequired: currentApiKeyRequired,
    apiKey: apiKeyInput.value,
    model: modelInput.value,
    endpointOverride: endpointInput?.value ?? "",
    activate: false,
    accountTier: tierSelect?.value ?? null,
  }).catch((error) => {
    // Keep navigation responsive; explicit save still reports errors.
    if (reportErrors) throw error;
    return null;
  });
  if (config) applyConfig(config);
}

async function refreshModelsForProvider(providerId: string, silent = false, forceReprobe = false) {
  try {
    if (!silent) setStatus(forceReprobe ? "Re-probing model availability..." : "Loading models...");
    if (!$("#settingsModal").hidden && document.querySelector("#configModel") && providerId === selectedSettingsProviderId) {
      await persistVisibleProviderForm();
    }
    const config = await invoke<AiConfig>("refresh_provider_models", { providerId, forceReprobe });
    applyConfig(config);
    if (settingsSection === "models") {
      renderSettingsModelList();
    }
    if (!silent) setStatus(forceReprobe ? "Availability re-probed" : "Models refreshed");
  } catch (error) {
    if (!silent) setStatus(errorMessage(error));
  }
}

function toggleApiKeyVisibility(providerId: string) {
  if (revealedApiKeys.has(providerId)) {
    revealedApiKeys.delete(providerId);
  } else {
    revealedApiKeys.add(providerId);
  }
  renderSettings();
  queueMicrotask(() => document.querySelector<HTMLInputElement>("#configApiKey")?.focus());
}

async function refreshAllModels() {
  if (!aiConfig) return;
  setStatus("Refreshing models...");
  for (const provider of aiConfig.providers) {
    const info = getProviderInfo(provider.id);
    if (info?.supports_model_refresh || isCustomProvider(provider.id)) {
      await refreshModelsForProvider(provider.id, true);
    }
  }
  renderSettings();
  setStatus("Models refreshed");
}

function renderModelSettings() {
  if (!aiConfig) return;
  $("#providerSettingsBody").innerHTML = `
    <div class="settings-fixed-panel">
      <div class="settings-provider-head">
        <div>
          <h2>All Models</h2>
          <p>Search all loaded models from enabled providers and choose the model used for summaries.</p>
        </div>
        <button id="refreshAllModelsBtn" type="button">Refresh all</button>
      </div>
      <div class="model-toolbar">
        <input id="globalModelSearch" type="text" placeholder="Search models" />
        <label class="toggle-row">
          <input id="freeModelsOnly" type="checkbox" ${showOnlyFreeModels ? "checked" : ""} />
          Free only
        </label>
      </div>
    </div>
    <div class="settings-model-preview settings-scroll-list">
      <div id="globalModelList"></div>
    </div>
  `;
  renderSettingsModelList();
}

function renderSidebarSelectedModel(): string {
  if (!aiConfig?.model) {
    return `
      <div class="sidebar-selected-model">
        <span class="sidebar-label">Selected for summaries</span>
        <strong>No model selected</strong>
      </div>
    `;
  }
  const config = aiConfig;
  const provider = getProviderConfig(config.provider);
  const model = provider?.models.find((item) => item.id === config.model);
  return `
    <div class="sidebar-selected-model">
      <span class="sidebar-label">Selected for summaries</span>
      <strong>${escapeHtml(model?.name ?? config.model)}</strong>
      <small>${escapeHtml(provider ? providerDisplayName(provider) : config.provider)}</small>
      <span class="model-tags">${model && provider ? renderModelTagsForEntry({ provider, providerInfo: getProviderInfo(provider.id), model }) : ""}</span>
    </div>
  `;
}

function openChatTestDialog(providerId: string, modelId: string) {
  chatTestTarget = { providerId, modelId };
  chatTestMessages = [];
  const provider = getProviderConfig(providerId);
  const model = provider?.models.find((item) => item.id === modelId);
  $("#chatTestTitle").textContent = `Test ${model?.name ?? modelId}`;
  $("#chatTestMeta").textContent = provider ? `${providerDisplayName(provider)} / ${modelId}` : modelId;
  const error = $("#chatTestError");
  error.hidden = true;
  error.textContent = "";
  renderChatTestMessages();
  showModal("#chatTestModal");
  queueMicrotask(() => {
    const input = $<HTMLTextAreaElement>("#chatTestMessage");
    input.value = "Hi";
    input.focus();
    input.select();
  });
}

async function sendChatTest() {
  if (!chatTestTarget) return;
  const error = $("#chatTestError");
  const sendButton = $<HTMLButtonElement>("#chatTestSend");
  const input = $<HTMLTextAreaElement>("#chatTestMessage");
  const message = input.value.trim();
  if (!message) return;
  chatTestMessages.push({ role: "user", content: message });
  input.value = "";
  error.hidden = true;
  error.textContent = "";
  renderChatTestMessages(true);
  sendButton.disabled = true;
  try {
    if (!$("#settingsModal").hidden && document.querySelector("#configModel") && chatTestTarget.providerId === selectedSettingsProviderId) {
      await persistVisibleProviderForm(true);
    }
    const response = await invoke<string>("test_provider_model_chat", {
      providerId: chatTestTarget.providerId,
      modelId: chatTestTarget.modelId,
      messages: chatTestMessages,
    });
    chatTestMessages.push({ role: "assistant", content: response });
    renderChatTestMessages();
    setStatus("Model chat test succeeded");
  } catch (error) {
    $("#chatTestError").hidden = false;
    $("#chatTestError").textContent = errorMessage(error);
    setStatus(errorMessage(error));
  } finally {
    sendButton.disabled = false;
    input.focus();
  }
}

function renderChatTestMessages(loading = false) {
  const list = $("#chatTestMessages");
  const messages = chatTestMessages.length
    ? chatTestMessages
        .map((message) => `
          <div class="chat-message ${message.role}">
            <span>${message.role === "user" ? "You" : "Model"}</span>
            <p>${escapeHtml(message.content)}</p>
          </div>
        `)
        .join("")
    : '<p class="empty">Send a short prompt to test this model.</p>';
  list.innerHTML = loading
    ? `${messages}<div class="chat-message assistant"><span>Model</span><p>Thinking...</p></div>`
    : messages;
  list.scrollTop = list.scrollHeight;
}

function renderSettingsModelList() {
  const list = document.querySelector<HTMLDivElement>("#globalModelList");
  if (!list) return;
  const query = document.querySelector<HTMLInputElement>("#globalModelSearch")?.value.trim().toLowerCase() ?? "";
  const entries = getAllModelEntries()
    .filter((entry) => {
      if (showOnlyFreeModels && !isFreeModelEntry(entry)) return false;
      const haystack = `${entry.model.id} ${entry.model.name} ${normalizeModelTags(entry.model.tags).join(" ")} ${providerDisplayName(entry.provider)}`.toLowerCase();
      return !query || haystack.includes(query);
    })
    .sort((a, b) => {
      const providerCompare = providerDisplayName(a.provider).localeCompare(providerDisplayName(b.provider));
      return providerCompare || a.model.name.localeCompare(b.model.name);
    });

  list.innerHTML = entries.length
    ? entries.map(renderGlobalModelItem).join("")
    : '<p class="empty">No models found. Refresh providers first or change the filter.</p>';
}

function renderGlobalModelItem(entry: ModelEntry): string {
  const active = aiConfig?.provider === entry.provider.id && aiConfig.model === entry.model.id ? " active" : "";
  return `
    <div class="settings-model-row${active}">
      <span class="settings-model-main">
        <strong>${escapeHtml(entry.model.name)}</strong>
        <small>${escapeHtml(entry.model.id)}</small>
      </span>
      <span class="settings-model-provider">${escapeHtml(providerDisplayName(entry.provider))}</span>
      <span class="model-tags">${renderModelTagsForEntry(entry)}</span>
      <span class="settings-model-actions">
        <button type="button" data-model-provider-id="${escapeHtml(entry.provider.id)}" data-model-id="${escapeHtml(entry.model.id)}" ${active ? "disabled" : ""}>Use</button>
        <button type="button" data-test-chat-provider-id="${escapeHtml(entry.provider.id)}" data-test-chat-model-id="${escapeHtml(entry.model.id)}">Test chat</button>
      </span>
    </div>
  `;
}

async function selectGlobalModel(providerId: string, modelId: string) {
  const provider = getProviderConfig(providerId);
  if (!provider) return;
  const config = await invoke<AiConfig>("save_provider_config", {
    providerId,
    name: provider.name ?? null,
    enabled: true,
    apiKeyRequired: provider.api_key_required ?? false,
    apiKey: provider.api_key,
    model: modelId,
    endpointOverride: provider.endpoint_override ?? "",
    activate: true,
  });
  aiConfig = config;
  const providerInfo = getProviderInfo(config.provider);
  statusModel.textContent = `${providerInfo?.name ?? config.provider} / ${config.model || "kein Modell"}`;
  if (!$("#settingsModal").hidden) {
    $("#settingsSelectedModel").innerHTML = renderSidebarSelectedModel();
    $("#providerSettingsList").innerHTML = renderProviderNavigation(selectedSettingsProviderId);
    updateActiveModelButtons(providerId, modelId);
  }
  setStatus(`Model selected: ${modelId}`);
}

function updateActiveModelButtons(providerId: string, modelId: string) {
  document
    .querySelectorAll<HTMLButtonElement>("#providerSettingsBody button[data-model-id][data-model-provider-id]")
    .forEach((button) => {
      const isActive =
        button.dataset.modelProviderId === providerId && button.dataset.modelId === modelId;
      button.disabled = isActive;
      const row = button.closest<HTMLElement>(".settings-model-row");
      if (row) row.classList.toggle("active", isActive);
    });
}

function getAllModelEntries(): ModelEntry[] {
  if (!aiConfig) return [];
  return aiConfig.providers.flatMap((provider) =>
    provider.enabled && isProviderConfigured(provider)
      ? provider.models.map((model) => ({
          provider,
          providerInfo: getProviderInfo(provider.id),
          model,
        }))
      : [],
  );
}

function renderModelPreview(provider: AiProviderConfig): string {
  if (!provider.models.length) {
    return '<p class="empty">No models loaded yet.</p>';
  }
  return provider.models
    .map((model) => {
      const active = aiConfig?.provider === provider.id && aiConfig.model === model.id ? " active" : "";
      return `
        <div class="settings-model-row${active}">
          <span class="settings-model-main">
            <strong>${escapeHtml(model.name)}</strong>
            <small>${escapeHtml(model.id)}</small>
          </span>
          <span class="model-tags">${renderModelTags(model, provider)}</span>
          <span class="settings-model-actions">
            <button type="button" data-model-provider-id="${escapeHtml(provider.id)}" data-model-id="${escapeHtml(model.id)}" ${active ? "disabled" : ""}>Use</button>
            <button type="button" data-test-chat-provider-id="${escapeHtml(provider.id)}" data-test-chat-model-id="${escapeHtml(model.id)}">Test chat</button>
          </span>
        </div>
      `;
    })
    .join("");
}

function renderModelTags(model: AiModel, provider?: AiProviderConfig): string {
  const tags = modelDisplayTags(model, provider);
  return normalizeModelTags(tags).map((tag) => `<span class="model-tag">${escapeHtml(tag)}</span>`).join("");
}

function renderModelTagsForEntry(entry: ModelEntry): string {
  const tags = modelDisplayTags(entry.model, entry.provider);
  return normalizeModelTags(tags).map((tag) => `<span class="model-tag">${escapeHtml(tag)}</span>`).join("");
}

function isFreeModelEntry(entry: ModelEntry): boolean {
  if (suppressAvailabilityTags(entry.provider)) return false;
  return entry.model.free;
}

function suppressAvailabilityTags(provider: AiProviderConfig): boolean {
  return provider.id === "ollama_cloud" && providerAccountTier(provider) !== "free";
}

function providerAccountTier(provider: AiProviderConfig): AccountTier {
  return (provider.account_tier as AccountTier | null | undefined) ?? "free";
}

function modelDisplayTags(model: AiModel, provider?: AiProviderConfig): string[] {
  const tags = [...model.tags];
  const suppress = provider ? suppressAvailabilityTags(provider) : false;
  if (suppress) {
    return tags.filter((tag) => tag !== "Free" && tag !== "Subscription required" && tag !== "Probe unknown");
  }
  if (model.free && !tags.includes("Free")) tags.unshift("Free");
  if (model.availability === "subscription_required" && !tags.includes("Subscription required")) {
    tags.push("Subscription required");
  }
  if (model.availability === "unknown" && !tags.includes("Probe unknown")) {
    tags.push("Probe unknown");
  }
  return tags;
}

function renderAccountTierField(provider: AiProviderConfig): string {
  if (provider.id !== "ollama_cloud") return "";
  const tier = providerAccountTier(provider);
  const option = (value: AccountTier, label: string) =>
    `<option value="${value}" ${tier === value ? "selected" : ""}>${label}</option>`;
  return `
    <label class="field-row"><span class="field-label">Plan</span>
      <select id="configAccountTier">
        ${option("free", "Free")}
        ${option("pro", "Pro")}
        ${option("max", "Max")}
      </select>
    </label>
  `;
}

function renderModelRefreshState(provider: AiProviderConfig): string {
  if (!provider.models_updated_at) return "Not refreshed yet";
  const absolute = new Date(provider.models_updated_at).toLocaleString();
  return `<span title="${escapeHtml(absolute)}">Refreshed ${escapeHtml(formatRelativeTime(provider.models_updated_at))}</span>`;
}

const relativeTimeFormatter = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

function formatRelativeTime(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return iso;
  const diffSeconds = Math.round((then - Date.now()) / 1000);
  const abs = Math.abs(diffSeconds);
  if (abs < 60) return relativeTimeFormatter.format(diffSeconds, "second");
  if (abs < 3600) return relativeTimeFormatter.format(Math.round(diffSeconds / 60), "minute");
  if (abs < 86_400) return relativeTimeFormatter.format(Math.round(diffSeconds / 3600), "hour");
  if (abs < 7 * 86_400) return relativeTimeFormatter.format(Math.round(diffSeconds / 86_400), "day");
  return new Date(iso).toLocaleDateString();
}

function normalizeModelTags(tags: string[]): string[] {
  const tagNames: Record<string, string> = {
    Kostenlos: "Free",
    Schnell: "Fast",
    Lokal: "Local",
    "Günstig": "Low cost",
  };
  return [...new Set(tags.map((tag) => tagNames[tag] ?? tag).filter((tag) => !["Low cost", "Fast"].includes(tag)))];
}

function getProviderConfig(providerId: string): AiProviderConfig | undefined {
  return aiConfig?.providers.find((provider) => provider.id === providerId);
}

function getProviderInfo(providerId: string): AiProviderInfo | undefined {
  return aiProviders.find((provider) => provider.id === providerId);
}

function isCustomProvider(providerId: string): boolean {
  return providerId === "custom" || providerId.startsWith("custom_");
}

function isUserManagedProvider(providerId: string): boolean {
  return providerId === "ollama" || isCustomProvider(providerId);
}

function isProviderConfigured(provider: AiProviderConfig): boolean {
  if (providerRequiresApiKey(provider) && !provider.api_key.trim()) return false;
  if (isUserManagedProvider(provider.id) && !provider.endpoint_override?.trim()) return false;
  if (!provider.models.length) return false;
  return true;
}

function providerRequiresApiKey(provider?: AiProviderConfig): boolean {
  if (!provider) return false;
  const info = getProviderInfo(provider.id);
  return info?.requires_api_key ?? !!provider.api_key_required;
}

function providerDisplayName(provider: AiProviderConfig): string {
  return provider.name || getProviderInfo(provider.id)?.name || provider.id;
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

function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleDateString(undefined, { year: "numeric", month: "long", day: "numeric" });
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
