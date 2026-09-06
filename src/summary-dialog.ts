import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ensureAiData, fillModelPicker } from "./ai-config";
import { $, confirmDialog, errorMessage, escapeHtml, hideModal, showModal } from "./dom-utils";
import { renderVideoList } from "./library";
import { showDetail, switchTab } from "./detail";
import { getActiveVideo, setBusy, setStatus, state } from "./state";
import { hideSummaryHistoryBar, renderSummaryMarkdown } from "./summary-view";
import type { SummaryModules, SummaryPreset, SummarySettings, Video } from "./types";
import { coerceBool } from "./utils";

const SUMMARY_SETTINGS_KEY = "summarySettings";
const MANAGE_PRESET_VALUE = "__manage__";
const DEFAULT_PRESET_ID = "standard";
const STANDARD_PRESET_PROMPT =
  "You are an expert assistant that turns YouTube video transcripts into clear, well-structured Markdown summaries. Start with a 1-2 sentence overview. Organize the key points under short headings and use bullet points. End with the main conclusions or takeaways. Ground every statement in the transcript; do not invent facts.";
const DEFAULT_SUMMARY_MODULES: SummaryModules = {
  tables: true,
  mermaid: false,
  assessment: false,
  verify: false,
  timestamps: false,
  links: false,
};

const LANGUAGE_NAMES: Record<string, string> = {
  original: "the same language as the transcript",
  german: "German",
  english: "English",
  french: "French",
  spanish: "Spanish",
  italian: "Italian",
};

const MODULE_PROMPT_ORDER = [
  "tables",
  "mermaid",
  "assessment",
  "verify",
  "timestamps",
  "links",
] as const;

const MODULE_PROMPTS: Record<(typeof MODULE_PROMPT_ORDER)[number], string> = {
  tables:
    "Use Markdown tables for comparisons, numbers, rankings or other data where a table is clearer than prose.",
  mermaid:
    "Where a process, architecture, timeline or set of relationships is complex, add a Mermaid diagram in a ```mermaid code block. Keep diagrams small and syntactically valid; prefer flowchart or sequenceDiagram.",
  assessment:
    "After the summary, add a section titled 'Einordnung' (in the summary language) with your own assessment: distinguish facts from opinions and claims, note how strong the presented evidence is, and mention notable counterarguments.",
  verify:
    "Critically check the video's central claims against your own knowledge: explicitly flag statements that are outdated, disputed or likely wrong, and briefly say why.",
  timestamps:
    "Prefix each major section or key point with the timestamp [mm:ss] of the transcript passage it is based on. Use exactly the bracketed format [mm:ss] or [h:mm:ss].",
  links:
    "If the video description contains helpful links (documentation, tools, sources, mentioned projects), end the summary with a section titled 'Ressourcen' (in the summary language) listing them as Markdown links with a short label each. Skip sponsor, affiliate, social media and channel self-promotion links. Omit the section entirely if no helpful links exist.",
};

let summaryPresets: SummaryPreset[] = [];
let presetEditMode: "new" | "rename" | "prompt" | null = null;
let presetEditTarget: SummaryPreset | null = null;
let presetIdManual = false;

function defaultSummarySettings(): SummarySettings {
  return {
    detail: "medium",
    lang: "original",
    useChapters: "yes",
    presetId: DEFAULT_PRESET_ID,
    modules: { ...DEFAULT_SUMMARY_MODULES },
  };
}

function parseSummaryModules(value: unknown): SummaryModules {
  const raw = value && typeof value === "object" ? (value as Record<string, unknown>) : {};
  return {
    tables: coerceBool(raw.tables, DEFAULT_SUMMARY_MODULES.tables),
    mermaid: coerceBool(raw.mermaid, DEFAULT_SUMMARY_MODULES.mermaid),
    assessment: coerceBool(raw.assessment, DEFAULT_SUMMARY_MODULES.assessment),
    verify: coerceBool(raw.verify, DEFAULT_SUMMARY_MODULES.verify),
    timestamps: coerceBool(raw.timestamps, DEFAULT_SUMMARY_MODULES.timestamps),
    links: coerceBool(raw.links, DEFAULT_SUMMARY_MODULES.links),
  };
}

function parseSummarySettings(value: unknown): SummarySettings {
  const defaults = defaultSummarySettings();
  if (!value || typeof value !== "object") return defaults;
  const saved = value as Record<string, unknown>;
  const detail = typeof saved.detail === "string" && saved.detail ? saved.detail : defaults.detail;
  const lang = typeof saved.lang === "string" && saved.lang ? saved.lang : defaults.lang;
  const useChapters =
    typeof saved.useChapters === "string" && saved.useChapters ? saved.useChapters : defaults.useChapters;
  const model = typeof saved.model === "string" && saved.model ? saved.model : undefined;
  const presetId =
    typeof saved.presetId === "string" && saved.presetId.trim() ? saved.presetId.trim() : defaults.presetId;
  return {
    detail,
    lang,
    useChapters,
    model,
    presetId,
    modules: parseSummaryModules(saved.modules),
  };
}

function readSummaryModules(): SummaryModules {
  return {
    tables: $<HTMLInputElement>("#summaryModTables").checked,
    mermaid: $<HTMLInputElement>("#summaryModMermaid").checked,
    assessment: $<HTMLInputElement>("#summaryModAssessment").checked,
    verify: $<HTMLInputElement>("#summaryModVerify").checked,
    timestamps: $<HTMLInputElement>("#summaryModTimestamps").checked,
    links: $<HTMLInputElement>("#summaryModLinks").checked,
  };
}

function applySummarySettings(settings: SummarySettings) {
  const detailSelect = $<HTMLSelectElement>("#summaryDetail");
  const langSelect = $<HTMLSelectElement>("#summaryLang");
  const chaptersSelect = $<HTMLSelectElement>("#summaryUseChapters");
  if ([...detailSelect.options].some((option) => option.value === settings.detail)) {
    detailSelect.value = settings.detail;
  }
  if ([...langSelect.options].some((option) => option.value === settings.lang)) {
    langSelect.value = settings.lang;
  }
  if ([...chaptersSelect.options].some((option) => option.value === settings.useChapters)) {
    chaptersSelect.value = settings.useChapters;
  }
  $<HTMLInputElement>("#summaryModTables").checked = settings.modules.tables;
  $<HTMLInputElement>("#summaryModMermaid").checked = settings.modules.mermaid;
  $<HTMLInputElement>("#summaryModAssessment").checked = settings.modules.assessment;
  $<HTMLInputElement>("#summaryModVerify").checked = settings.modules.verify;
  $<HTMLInputElement>("#summaryModTimestamps").checked = settings.modules.timestamps;
  $<HTMLInputElement>("#summaryModLinks").checked = settings.modules.links;
}

function loadSummarySettings(): SummarySettings {
  let settings = defaultSummarySettings();
  try {
    const raw = localStorage.getItem(SUMMARY_SETTINGS_KEY);
    if (raw) settings = parseSummarySettings(JSON.parse(raw));
  } catch {
    // ignore corrupt entries
  }
  applySummarySettings(settings);
  return settings;
}

function saveSummarySettings() {
  const settings: SummarySettings = {
    detail: $<HTMLSelectElement>("#summaryDetail").value,
    lang: $<HTMLSelectElement>("#summaryLang").value,
    useChapters: $<HTMLSelectElement>("#summaryUseChapters").value,
    model: $<HTMLSelectElement>("#summaryModel").value || undefined,
    presetId: selectedPresetId(),
    modules: readSummaryModules(),
  };
  localStorage.setItem(SUMMARY_SETTINGS_KEY, JSON.stringify(settings));
}

function parseModelValue(value: string): [string | null, string | null] {
  if (!value) return [null, null];
  try {
    const [providerId, modelId] = JSON.parse(value) as [string, string];
    return [providerId, modelId];
  } catch {
    return [null, null];
  }
}

function detailParagraph(detail: string): string {
  if (detail === "short") {
    return "Provide a very concise summary: just 3-5 bullet points with the key takeaways.";
  }
  if (detail === "detailed") {
    return "Provide a comprehensive and detailed summary.\nInclude all main topics, key arguments, facts, insights, conclusions and takeaways.";
  }
  return "Provide a clear, structured summary with overview, key points and takeaways.";
}

function selectedPresetId(): string {
  const value = $<HTMLSelectElement>("#summaryPreset").value;
  if (!value || value === MANAGE_PRESET_VALUE) return DEFAULT_PRESET_ID;
  return value;
}

function selectedPresetPrompt(): string {
  const fromList = summaryPresets.find((preset) => preset.id === selectedPresetId())?.prompt.trim();
  if (fromList) return fromList;
  return STANDARD_PRESET_PROMPT;
}

function buildSummaryPrompt(): string {
  const detail = $<HTMLSelectElement>("#summaryDetail").value;
  const lang = $<HTMLSelectElement>("#summaryLang").value;
  const useChapters = $<HTMLSelectElement>("#summaryUseChapters").value;
  const modules = readSummaryModules();
  const language = LANGUAGE_NAMES[lang] ?? LANGUAGE_NAMES.original;
  const parts = [selectedPresetPrompt(), detailParagraph(detail), `Write the summary in ${language}.`];
  if (useChapters === "yes") {
    parts.push("If chapter markers are provided, structure the summary by chapter.");
  }
  for (const key of MODULE_PROMPT_ORDER) {
    if (modules[key]) parts.push(MODULE_PROMPTS[key]);
  }
  return parts.filter((part) => part.length > 0).join("\n\n");
}

function updateSummaryPromptEditedBadge() {
  const composed = buildSummaryPrompt();
  const current = $<HTMLTextAreaElement>("#summaryPrompt").value;
  $<HTMLSpanElement>("#summaryPromptEdited").hidden = current === composed;
}

function recomposeSummaryPrompt() {
  $<HTMLTextAreaElement>("#summaryPrompt").value = buildSummaryPrompt();
  updateSummaryPromptEditedBadge();
}

function onSummaryPresetChange() {
  const select = $<HTMLSelectElement>("#summaryPreset");
  if (select.value === MANAGE_PRESET_VALUE) {
    const last = select.dataset.lastPresetId;
    select.value =
      last && summaryPresets.some((preset) => preset.id === last) ? last : DEFAULT_PRESET_ID;
    void openPresetManager();
    return;
  }
  select.dataset.lastPresetId = select.value;
  recomposeSummaryPrompt();
}

export async function loadSummaryPresets() {
  summaryPresets = await invoke<SummaryPreset[]>("summary_presets_list");
}

function fillPresetPicker(selectedId: string) {
  const select = $<HTMLSelectElement>("#summaryPreset");
  const fallback = summaryPresets.some((preset) => preset.id === DEFAULT_PRESET_ID)
    ? DEFAULT_PRESET_ID
    : (summaryPresets[0]?.id ?? MANAGE_PRESET_VALUE);
  const resolved = summaryPresets.some((preset) => preset.id === selectedId) ? selectedId : fallback;
  select.innerHTML = [
    ...summaryPresets.map(
      (preset) => `<option value="${escapeHtml(preset.id)}">${escapeHtml(preset.name)}</option>`,
    ),
    `<option value="${MANAGE_PRESET_VALUE}">Verwalten…</option>`,
  ].join("");
  select.value = resolved;
  if (resolved !== MANAGE_PRESET_VALUE) {
    select.dataset.lastPresetId = resolved;
  }
}

async function openPresetManager() {
  try {
    await loadSummaryPresets();
  } catch (error) {
    setStatus(errorMessage(error));
  }
  fillPresetPicker(selectedPresetId());
  renderPresetList();
  showModal("#presetManageModal");
}

function renderPresetList() {
  const list = $<HTMLDivElement>("#presetList");
  if (!summaryPresets.length) {
    list.innerHTML = '<p class="empty-list compact">Keine Vorlagen</p>';
    return;
  }
  list.innerHTML = summaryPresets.map(renderPresetRow).join("");
  list.querySelectorAll<HTMLButtonElement>("[data-preset-action]").forEach((button) => {
    button.addEventListener("click", () => {
      const id = button.dataset.presetId ?? "";
      const preset = summaryPresets.find((item) => item.id === id);
      if (!preset || preset.builtin) return;
      const action = button.dataset.presetAction;
      if (action === "rename") openPresetEditDialog("rename", preset);
      else if (action === "prompt") openPresetEditDialog("prompt", preset);
      else if (action === "delete") void deletePreset(preset);
    });
  });
}

function renderPresetRow(preset: SummaryPreset): string {
  const actions = preset.builtin
    ? '<span class="collection-count">fest</span>'
    : `
      <div class="collection-actions">
        <button class="inline-action" data-preset-action="rename" data-preset-id="${escapeHtml(preset.id)}">Umbenennen</button>
        <button class="inline-action" data-preset-action="prompt" data-preset-id="${escapeHtml(preset.id)}">Prompt</button>
        <button class="inline-action" data-preset-action="delete" data-preset-id="${escapeHtml(preset.id)}">Löschen</button>
      </div>
    `;
  return `
    <div class="collection-row">
      <div class="collection-item">
        <span class="collection-name">${escapeHtml(preset.name)}</span>
        <span class="collection-count">${escapeHtml(preset.id)}</span>
      </div>
      ${actions}
    </div>
  `;
}

function slugFromName(name: string): string {
  const slug = name
    .trim()
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 32)
    .replace(/-+$/, "");
  return /^[a-z0-9]/.test(slug) ? slug : "";
}

function setPresetEditError(message: string | null) {
  const errorEl = $<HTMLParagraphElement>("#presetEditError");
  errorEl.hidden = !message;
  errorEl.textContent = message ?? "";
}

function openPresetEditDialog(mode: "new" | "rename" | "prompt", preset?: SummaryPreset) {
  presetEditMode = mode;
  presetEditTarget = preset ?? null;
  presetIdManual = mode !== "new";
  setPresetEditError(null);

  const idLabel = $<HTMLLabelElement>("#presetEditIdLabel");
  const idHint = $<HTMLParagraphElement>("#presetEditIdHint");
  const nameLabel = $<HTMLLabelElement>("#presetEditNameLabel");
  const promptLabel = $<HTMLLabelElement>("#presetEditPromptLabel");
  const idInput = $<HTMLInputElement>("#presetEditId");
  const nameInput = $<HTMLInputElement>("#presetEditName");
  const promptInput = $<HTMLTextAreaElement>("#presetEditPrompt");

  idInput.value = preset?.id ?? "";
  nameInput.value = preset?.name ?? "";
  promptInput.value = preset?.prompt ?? "";
  idInput.disabled = mode !== "new";

  const showId = mode === "new";
  const showName = mode === "new" || mode === "rename";
  const showPrompt = mode === "new" || mode === "prompt";
  idLabel.hidden = !showId;
  idHint.hidden = !showId;
  nameLabel.hidden = !showName;
  promptLabel.hidden = !showPrompt;

  $<HTMLHeadingElement>("#presetEditTitle").textContent =
    mode === "new" ? "Vorlage anlegen" : mode === "rename" ? "Vorlage umbenennen" : "Prompt bearbeiten";

  showModal("#presetEditModal");
  queueMicrotask(() => {
    if (showId) nameInput.focus();
    else if (showName) {
      nameInput.focus();
      nameInput.select();
    } else {
      promptInput.focus();
    }
  });
}

function onPresetNameInput() {
  if (presetEditMode !== "new" || presetIdManual) return;
  $<HTMLInputElement>("#presetEditId").value = slugFromName($<HTMLInputElement>("#presetEditName").value);
}

async function savePresetEdit() {
  if (!presetEditMode) return;
  const id =
    presetEditMode === "new"
      ? $<HTMLInputElement>("#presetEditId").value.trim()
      : (presetEditTarget?.id ?? "");
  const name =
    presetEditMode === "prompt"
      ? (presetEditTarget?.name ?? "")
      : $<HTMLInputElement>("#presetEditName").value.trim();
  const prompt =
    presetEditMode === "rename"
      ? (presetEditTarget?.prompt ?? "")
      : $<HTMLTextAreaElement>("#presetEditPrompt").value;
  if (presetEditMode === "new" && !id) {
    setPresetEditError("ID darf nicht leer sein");
    return;
  }
  if ((presetEditMode === "new" || presetEditMode === "rename") && !name) {
    setPresetEditError("Name darf nicht leer sein");
    return;
  }
  if ((presetEditMode === "new" || presetEditMode === "prompt") && !prompt.trim()) {
    setPresetEditError("Prompt darf nicht leer sein");
    return;
  }
  if (presetEditMode === "new" && summaryPresets.some((preset) => preset.id === id)) {
    setPresetEditError("ID ist bereits vergeben");
    return;
  }

  try {
    const saved = await invoke<SummaryPreset>("summary_preset_save", {
      preset: { id, name, prompt, builtin: false },
    });
    hideModal("#presetEditModal");
    await loadSummaryPresets();
    const selectNext = presetEditMode === "new" ? saved.id : selectedPresetId();
    fillPresetPicker(selectNext);
    renderPresetList();
    if (saved.id === selectedPresetId()) recomposeSummaryPrompt();
    setStatus("Vorlage gespeichert");
  } catch (error) {
    setPresetEditError(errorMessage(error));
  }
}

async function deletePreset(preset: SummaryPreset) {
  if (
    !(await confirmDialog(`Vorlage "${preset.name}" löschen?`, {
      title: "Vorlage löschen",
      okLabel: "Löschen",
    }))
  ) {
    return;
  }
  try {
    await invoke<void>("summary_preset_delete", { id: preset.id });
    const wasSelected = selectedPresetId() === preset.id;
    await loadSummaryPresets();
    fillPresetPicker(wasSelected ? DEFAULT_PRESET_ID : selectedPresetId());
    renderPresetList();
    if (wasSelected) recomposeSummaryPrompt();
    setStatus("Vorlage gelöscht");
  } catch (error) {
    setStatus(errorMessage(error));
  }
}

export async function openSummaryDialog() {
  let video = getActiveVideo();
  if (!video) return;
  if (!video.has_transcript) {
    setStatus("Kein Transkript vorhanden – bitte „Transkript laden“ versuchen");
    return;
  }
  const id = video.id;
  if (!video.transcript) {
    try {
      const detail = await invoke<Video>("get_video_detail", { id });
      state.videos = state.videos.map((item) => (item.id === id ? detail : item));
      if (state.activeVideoId !== id) return;
      video = detail;
    } catch (error) {
      if (state.activeVideoId === id) {
        setStatus(errorMessage(error));
      }
      return;
    }
  }
  const saved = loadSummarySettings();
  $<HTMLDetailsElement>("#summaryPromptDetails").open = false;
  try {
    await loadSummaryPresets();
  } catch (error) {
    setStatus(errorMessage(error));
  }
  fillPresetPicker(saved.presetId);
  recomposeSummaryPrompt();
  showModal("#summaryModal");
  try {
    await ensureAiData();
  } catch (error) {
    setStatus(errorMessage(error));
  }
  fillModelPicker($<HTMLSelectElement>("#summaryModel"), saved.model);
}

export async function startSummary() {
  const video = getActiveVideo();
  if (!video || state.busy) return;

  saveSummarySettings();
  const [providerId, modelId] = parseModelValue($<HTMLSelectElement>("#summaryModel").value);
  const videoId = video.id;
  setBusy(true, "Zusammenfassung wird erstellt...");
  hideModal("#presetEditModal");
  hideModal("#presetManageModal");
  hideModal("#summaryModal");
  switchTab("summary");
  state.streamingVideoId = videoId;
  state.summaryRenderGen += 1;
  hideSummaryHistoryBar();
  $("#summaryBody").innerHTML = '<p class="empty">Zusammenfassung wird erstellt…</p>';

  let unlisten: (() => void) | undefined;
  try {
    unlisten = await listen<{ videoId: number; text: string; chars: number }>(
      "ai:summarize_stream",
      (event) => {
        if (event.payload.videoId !== videoId) return;
        const active = getActiveVideo();
        if (!active || active.id !== videoId) return;
        void renderSummaryMarkdown(event.payload.text);
        setStatus(`Zusammenfassung läuft – ${event.payload.chars} Zeichen`);
      },
    );
    const modules = readSummaryModules();
    const updated = await invoke<Video>("summarize_video", {
      id: videoId,
      systemPrompt: $<HTMLTextAreaElement>("#summaryPrompt").value.trim(),
      providerId,
      modelId,
      timestamps: modules.timestamps,
      options: JSON.stringify({
        presetId: selectedPresetId(),
        modules,
        detail: $<HTMLSelectElement>("#summaryDetail").value,
        language: $<HTMLSelectElement>("#summaryLang").value,
      }),
    });
    state.streamingVideoId = null;
    state.videos = state.videos.map((item) => (item.id === updated.id ? updated : item));
    renderVideoList();
    if (getActiveVideo()?.id === videoId) {
      showDetail(updated);
      switchTab("summary");
    }
    setStatus("Zusammenfassung fertig");
  } catch (error) {
    state.streamingVideoId = null;
    if (getActiveVideo()?.id === videoId) {
      const current = state.videos.find((item) => item.id === videoId);
      if (current) {
        showDetail(current);
      }
    }
    setStatus(errorMessage(error));
  } finally {
    state.streamingVideoId = null;
    unlisten?.();
    setBusy(false);
  }
}

export function bindSummaryDialogEvents() {
  $("#summarizeBtn").addEventListener("click", () => void openSummaryDialog());
  $("#summaryStart").addEventListener("click", () => void startSummary());
  $("#summaryCancel").addEventListener("click", () => hideModal("#summaryModal"));
  $("#summaryPreset").addEventListener("change", onSummaryPresetChange);
  $("#summaryPrompt").addEventListener("input", updateSummaryPromptEditedBadge);
  $("#summaryPromptReset").addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    recomposeSummaryPrompt();
  });

  [
    "#summaryDetail",
    "#summaryLang",
    "#summaryUseChapters",
    "#summaryModTables",
    "#summaryModMermaid",
    "#summaryModAssessment",
    "#summaryModVerify",
    "#summaryModTimestamps",
    "#summaryModLinks",
  ].forEach((selector) => {
    $(selector).addEventListener("change", recomposeSummaryPrompt);
  });

  $("#presetNew").addEventListener("click", () => openPresetEditDialog("new"));
  $("#presetManageClose").addEventListener("click", () => hideModal("#presetManageModal"));
  $("#presetEditCancel").addEventListener("click", () => hideModal("#presetEditModal"));
  $("#presetEditSave").addEventListener("click", () => void savePresetEdit());
  $("#presetEditName").addEventListener("input", onPresetNameInput);
  $("#presetEditId").addEventListener("input", () => {
    presetIdManual = true;
  });
  ["#presetEditId", "#presetEditName"].forEach((selector) => {
    $(selector).addEventListener("keydown", (event) => {
      if (event instanceof KeyboardEvent && event.key === "Enter") {
        event.preventDefault();
        void savePresetEdit();
      }
    });
  });
}
