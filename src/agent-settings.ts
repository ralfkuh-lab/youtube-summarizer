// Einstellungen der Agenten-Übergabe (Tab „Agent“). Die Ansicht wird auch vom
// Übergabedialog (`agent-handoff.ts`) verwendet, damit beide dieselbe
// Vorlagenliste sehen.

import { invoke } from "@tauri-apps/api/core";
import { activateSettingsTab } from "./ai-config";
import { $, errorMessage } from "./dom-utils";
import { setStatus } from "./state";
import type { AgentConfig, AgentConfigView, AgentTemplate } from "./types";

let view: AgentConfigView | null = null;
let editingId: string | null = null;
let editorOpen = false;
let previewGeneration = 0;
const listeners: Array<() => void> = [];

export function getAgentView(): AgentConfigView | null {
  return view;
}

export function onAgentViewChanged(listener: () => void) {
  listeners.push(listener);
}

function notify() {
  for (const listener of listeners) listener();
}

export async function loadAgentView(): Promise<AgentConfigView | null> {
  try {
    view = await invoke<AgentConfigView>("agent_config_get");
    render();
    notify();
  } catch (error) {
    setStatus(errorMessage(error));
  }
  return view;
}

function applyView(next: AgentConfigView) {
  view = next;
  render();
  notify();
}

function setError(selector: string, message: string | null) {
  const element = $<HTMLParagraphElement>(selector);
  element.hidden = !message;
  element.textContent = message ?? "";
}

function render() {
  if (!view) return;
  $<HTMLInputElement>("#agentWorkdirBase").value = view.config.workdirBase;
  $("#agentDefaultWorkdir").textContent = view.defaultWorkdirBase;
  $<HTMLSelectElement>("#agentShell").value = view.config.shell;
  $<HTMLSelectElement>("#agentSummaries").value = view.config.summaries;
  $<HTMLInputElement>("#agentIncludeChats").checked = view.config.includeChats;
  $<HTMLTextAreaElement>("#agentPrompt").value = view.config.prompt;
  renderActiveTemplates();
  renderCustomTemplates();
  if (!editorOpen) void updatePreview();
}

function renderActiveTemplates() {
  if (!view) return;
  const select = $<HTMLSelectElement>("#agentActiveTemplate");
  const templates = [...view.builtinTemplates, ...view.config.customTemplates];
  select.replaceChildren();
  for (const template of templates) {
    const option = document.createElement("option");
    option.value = template.id;
    option.textContent = template.name;
    select.appendChild(option);
  }
  if (!templates.some((template) => template.id === view?.config.activeTemplate)) {
    const option = document.createElement("option");
    option.value = view.config.activeTemplate;
    option.textContent = `${view.config.activeTemplate} (nicht gefunden)`;
    select.appendChild(option);
  }
  select.value = view.config.activeTemplate;
}

function renderCustomTemplates() {
  if (!view) return;
  const list = $("#agentTemplateList");
  list.replaceChildren();
  const custom = view.config.customTemplates;
  if (!custom.length) {
    const empty = document.createElement("p");
    empty.className = "settings-hint";
    empty.textContent = "Keine eigenen Vorlagen";
    list.appendChild(empty);
    return;
  }
  for (const template of custom) {
    const row = document.createElement("div");
    row.className = "agent-template-row";
    const label = document.createElement("span");
    label.className = "agent-template-name";
    label.textContent = `${template.name} (${template.id})`;
    const edit = document.createElement("button");
    edit.type = "button";
    edit.textContent = "Bearbeiten";
    edit.dataset.templateId = template.id;
    edit.addEventListener("click", () => openEditor(template));
    const remove = document.createElement("button");
    remove.type = "button";
    remove.textContent = "Löschen";
    remove.dataset.deleteTemplateId = template.id;
    remove.addEventListener("click", () => void removeTemplate(template.id));
    row.append(label, edit, remove);
    list.appendChild(row);
  }
}

function openEditor(template: AgentTemplate | null) {
  editorOpen = true;
  editingId = template?.id ?? null;
  $("#agentTemplateEdit").hidden = false;
  const id = $<HTMLInputElement>("#agentTemplateId");
  id.value = template?.id ?? "";
  id.disabled = template !== null;
  $<HTMLInputElement>("#agentTemplateName").value = template?.name ?? "";
  $<HTMLInputElement>("#agentTemplateCommand").value = template?.command ?? "";
  setError("#agentTemplateError", null);
  void updatePreview();
}

function closeEditor() {
  editorOpen = false;
  $("#agentTemplateEdit").hidden = true;
  editingId = null;
  setError("#agentTemplateError", null);
  void updatePreview();
}

function collectConfig(overrides: Partial<AgentConfig> = {}): AgentConfig {
  return {
    workdirBase: $<HTMLInputElement>("#agentWorkdirBase").value,
    shell: $<HTMLSelectElement>("#agentShell").value as AgentConfig["shell"],
    summaries: $<HTMLSelectElement>("#agentSummaries").value as AgentConfig["summaries"],
    includeChats: $<HTMLInputElement>("#agentIncludeChats").checked,
    prompt: $<HTMLTextAreaElement>("#agentPrompt").value,
    activeTemplate: $<HTMLSelectElement>("#agentActiveTemplate").value,
    customTemplates: view?.config.customTemplates ?? [],
    ...overrides,
  };
}

async function saveConfig(config: AgentConfig, errorSelector: string): Promise<boolean> {
  try {
    const next = await invoke<AgentConfigView>("agent_config_set", { config });
    applyView(next);
    setError(errorSelector, null);
    setStatus("Agent-Einstellungen gespeichert");
    return true;
  } catch (error) {
    setError(errorSelector, errorMessage(error));
    return false;
  }
}

async function saveFromForm(errorSelector: string) {
  if (!view) await loadAgentView();
  if (!view) return;
  await saveConfig(collectConfig(), errorSelector);
}

async function saveTemplate() {
  if (!view && !(await loadAgentView())) return;
  if (!view) return;
  const template: AgentTemplate = {
    id: $<HTMLInputElement>("#agentTemplateId").value.trim(),
    name: $<HTMLInputElement>("#agentTemplateName").value,
    command: $<HTMLInputElement>("#agentTemplateCommand").value,
  };
  const custom = [...view.config.customTemplates];
  const index = editingId === null ? -1 : custom.findIndex((item) => item.id === editingId);
  if (index >= 0) custom[index] = template;
  else custom.push(template);
  const saved = await saveConfig(collectConfig({ customTemplates: custom }), "#agentTemplateError");
  if (saved) closeEditor();
}

async function removeTemplate(id: string) {
  if (!view) return;
  const custom = view.config.customTemplates.filter((template) => template.id !== id);
  await saveConfig(collectConfig({ customTemplates: custom }), "#agentSettingsError");
}

/// Live-Vorschau über `agent_preview`: im Editor das getippte Kommando, sonst
/// das der aktiven Vorlage.
async function updatePreview() {
  const preview = $("#agentPreview");
  const command = editorOpen
    ? $<HTMLInputElement>("#agentTemplateCommand").value
    : activeTemplateCommand();
  if (!command.trim()) {
    preview.textContent = "";
    return;
  }
  const generation = ++previewGeneration;
  const shell = $<HTMLSelectElement>("#agentShell").value;
  try {
    const text = await invoke<string>("agent_preview", { command, shell });
    if (generation !== previewGeneration) return;
    preview.textContent = text;
    preview.classList.remove("settings-ai-error");
    preview.classList.add("settings-hint");
  } catch (error) {
    if (generation !== previewGeneration) return;
    preview.textContent = errorMessage(error);
    preview.classList.add("settings-ai-error");
    preview.classList.remove("settings-hint");
  }
}

function activeTemplateCommand(): string {
  if (!view) return "";
  const active = $<HTMLSelectElement>("#agentActiveTemplate").value || view.config.activeTemplate;
  const templates = [...view.builtinTemplates, ...view.config.customTemplates];
  return templates.find((template) => template.id === active)?.command ?? "";
}

export function bindAgentSettingsEvents() {
  const tab = document.querySelector<HTMLButtonElement>("#settings-tab-agent");
  tab?.addEventListener("click", () => {
    activateSettingsTab("agent");
    void loadAgentView();
  });

  ["#agentWorkdirBase", "#agentShell", "#agentSummaries", "#agentIncludeChats", "#agentActiveTemplate", "#agentPrompt"].forEach(
    (selector) => {
      $(selector).addEventListener("change", () => void saveFromForm("#agentSettingsError"));
    },
  );
  $("#agentPromptReset").addEventListener("click", () => {
    // Leerer Prompt = Standard-Prompt des Backends.
    $<HTMLTextAreaElement>("#agentPrompt").value = "";
    void saveFromForm("#agentSettingsError");
  });
  $("#agentTemplateNew").addEventListener("click", () => openEditor(null));
  $("#agentTemplateCancel").addEventListener("click", closeEditor);
  $("#agentTemplateSave").addEventListener("click", () => void saveTemplate());
  $("#agentTemplateCommand").addEventListener("input", () => void updatePreview());
  $("#agentShell").addEventListener("change", () => void updatePreview());
}
