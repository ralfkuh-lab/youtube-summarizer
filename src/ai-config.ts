import { invoke } from "@tauri-apps/api/core";
import {
  $,
  confirmDialog,
  errorMessage,
  escapeHtml,
  hideModal,
  showModal,
} from "./dom-utils";

export type AiConfig = {
  provider: string;
  api_key: string;
  model: string;
  endpoint_override?: string | null;
  providers: AiProviderConfig[];
};

export type AiProviderConfig = {
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

export type AccountTier = "free" | "pro" | "max";

export type AiModel = {
  id: string;
  name: string;
  tags: string[];
  free: boolean;
  availability?: "free" | "subscription_required" | "unknown" | null;
};

export type AiProviderInfo = {
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

let aiConfig: AiConfig | null = null;
let aiProviders: AiProviderInfo[] = [];
let selectedSettingsProviderId = "opencode_go";
let settingsSection: "providers" | "models" = "providers";
const FREE_MODELS_ONLY_KEY = "settings.freeModelsOnly";
let showOnlyFreeModels = localStorage.getItem(FREE_MODELS_ONLY_KEY) === "true";
const revealedApiKeys = new Set<string>();
let chatTestTarget: { providerId: string; modelId: string } | null = null;
let chatTestMessages: ChatTestMessage[] = [];

let statusModelEl: HTMLElement | null = null;
let setStatusFn: (message: string) => void = () => {};

export type AiConfigInit = {
  statusModelEl: HTMLElement;
  setStatus: (message: string) => void;
};

export function initAiConfig(deps: AiConfigInit) {
  statusModelEl = deps.statusModelEl;
  setStatusFn = deps.setStatus;
}

export function getAiConfig(): AiConfig | null {
  return aiConfig;
}

export function setProviders(providers: AiProviderInfo[]) {
  aiProviders = providers;
}

export function bindAiConfigEvents() {
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
          setStatusFn("Konfiguration gespeichert");
          renderSettings();
        })
        .catch((err) => setStatusFn(errorMessage(err)));
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
        .then(() => setStatusFn("Konfiguration gespeichert"))
        .catch((err) => setStatusFn(errorMessage(err)));
    }
  });
  $("#chatTestSend").addEventListener("click", () => void sendChatTest());
  $("#chatTestClose").addEventListener("click", () => hideModal("#chatTestModal"));
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
    setStatusFn(errorMessage(error));
  }
}

export function applyConfig(config: AiConfig) {
  aiConfig = config;
  const provider = getProviderInfo(config.provider);
  if (statusModelEl) {
    statusModelEl.textContent = `${provider?.name ?? config.provider} / ${config.model || "kein Modell"}`;
  }
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
  setStatusFn("Provider deleted");
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

export async function refreshModelsForProvider(providerId: string, silent = false, forceReprobe = false) {
  try {
    if (!silent) setStatusFn(forceReprobe ? "Re-probing model availability..." : "Loading models...");
    if (!$("#settingsModal").hidden && document.querySelector("#configModel") && providerId === selectedSettingsProviderId) {
      await persistVisibleProviderForm();
    }
    const config = await invoke<AiConfig>("refresh_provider_models", { providerId, forceReprobe });
    applyConfig(config);
    if (settingsSection === "models") {
      renderSettingsModelList();
    }
    if (!silent) setStatusFn(forceReprobe ? "Availability re-probed" : "Models refreshed");
  } catch (error) {
    if (!silent) setStatusFn(errorMessage(error));
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
  setStatusFn("Refreshing models...");
  for (const provider of aiConfig.providers) {
    const info = getProviderInfo(provider.id);
    if (info?.supports_model_refresh || isCustomProvider(provider.id)) {
      await refreshModelsForProvider(provider.id, true);
    }
  }
  renderSettings();
  setStatusFn("Models refreshed");
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
    setStatusFn("Model chat test succeeded");
  } catch (error) {
    $("#chatTestError").hidden = false;
    $("#chatTestError").textContent = errorMessage(error);
    setStatusFn(errorMessage(error));
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
  if (statusModelEl) {
    statusModelEl.textContent = `${providerInfo?.name ?? config.provider} / ${config.model || "kein Modell"}`;
  }
  if (!$("#settingsModal").hidden) {
    $("#settingsSelectedModel").innerHTML = renderSidebarSelectedModel();
    $("#providerSettingsList").innerHTML = renderProviderNavigation(selectedSettingsProviderId);
    updateActiveModelButtons(providerId, modelId);
  }
  setStatusFn(`Model selected: ${modelId}`);
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
