// Button „An Agent übergeben“ und der zugehörige Dialog. Stufe 1: Kontextdatei
// schreiben, Kommando in die Zwischenablage kopieren — die App startet keinen
// Prozess.

import { invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { getAgentView, loadAgentView, onAgentViewChanged } from "./agent-settings";
import { $, errorMessage, hideModal, showModal } from "./dom-utils";
import { getActiveVideo, setBusy, setStatus, state } from "./state";
import type { AgentHandoff, AgentTemplate } from "./types";

let handoff: AgentHandoff | null = null;
let handoffVideoId: number | null = null;
/// Zaehlt jede Vorbereitung. Nur die juengste Antwort darf Feld, Zwischenablage
/// und Status aendern (H4b).
let prepareGeneration = 0;

async function startHandoff() {
  const video = getActiveVideo();
  if (!video || state.busy) return;
  setBusy(true, "Kontext wird vorbereitet...");
  try {
    if (!getAgentView()) await loadAgentView();
    await prepareAndCopy(null, true);
  } catch (error) {
    setStatus(errorMessage(error));
  } finally {
    setBusy(false);
  }
}

/// Vorbereiten, kopieren, Dialog oeffnen. Der Dialog oeffnet **immer**, auch wenn
/// beide Kopierwege scheitern.
async function prepareAndCopy(templateId: string | null, openDialog: boolean) {
  const video = getActiveVideo();
  if (!video) return;
  const videoId = video.id;
  const generation = ++prepareGeneration;
  const prepared = await invoke<AgentHandoff>("agent_prepare", {
    videoId,
    templateId,
  });
  // (a) Videowechsel waehrend `agent_prepare`: Ergebnis verwerfen.
  if (generation !== prepareGeneration || getActiveVideo()?.id !== videoId) return;
  const copied = await copyText(prepared.command);
  // (b) Schneller Vorlagenwechsel: die juengere Antwort gewinnt.
  if (generation !== prepareGeneration || getActiveVideo()?.id !== videoId) return;
  handoff = prepared;
  handoffVideoId = videoId;
  if (openDialog) {
    showModal("#agentModal");
  }
  renderTemplateOptions(templateId ?? getAgentView()?.config.activeTemplate ?? null);
  updateDialog();
  updateCopyHint(copied);
  setStatus(copyStatus(copied, prepared.workdir));
}

async function reprepare() {
  if (state.busy) return;
  const templateId = $<HTMLSelectElement>("#agentTemplate").value;
  try {
    await prepareAndCopy(templateId, false);
  } catch (error) {
    setStatus(errorMessage(error));
  }
}

async function copyFromDialog() {
  if (!handoff) return;
  const copied = await copyText(handoff.command);
  updateCopyHint(copied);
  setStatus(copyStatus(copied, handoff.workdir));
}

function copyStatus(copied: boolean, workdir: string): string {
  return copied
    ? `Kommando kopiert – Kontext liegt in ${workdir}`
    : `Kontext liegt in ${workdir} – Kommando im Dialog markieren und kopieren`;
}

/// Die Hinweiszeile im Dialog haengt am Kopiererfolg (H3).
function updateCopyHint(copied: boolean) {
  $("#agentCopyHint").textContent = copied
    ? "Das Kommando wurde in die Zwischenablage kopiert; die App startet nichts selbst."
    : "Kommando bitte markieren und kopieren; die App startet nichts selbst.";
}

async function revealContext() {
  if (!handoff) return;
  try {
    await revealItemInDir(handoff.contextFile);
    setStatus(`Ordner geöffnet: ${handoff.workdir}`);
  } catch (error) {
    setStatus(errorMessage(error));
  }
}

/// Die Zwischenablage-API; schlägt sie fehl, wird ein unsichtbares Textfeld
/// markiert und `execCommand("copy")` verwendet.
async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return fallbackCopy(text);
  }
}

function fallbackCopy(text: string): boolean {
  const helper = document.createElement("textarea");
  helper.value = text;
  helper.setAttribute("readonly", "");
  helper.style.position = "fixed";
  helper.style.top = "-1000px";
  helper.style.left = "-1000px";
  helper.style.opacity = "0";
  document.body.appendChild(helper);
  try {
    helper.focus();
    helper.select();
    return document.execCommand("copy");
  } catch {
    return false;
  } finally {
    helper.remove();
  }
}

function renderTemplateOptions(selected: string | null) {
  const select = $<HTMLSelectElement>("#agentTemplate");
  const view = getAgentView();
  const templates: AgentTemplate[] = view
    ? [...view.builtinTemplates, ...view.config.customTemplates]
    : [];
  const want = selected ?? select.value ?? view?.config.activeTemplate ?? "";
  select.replaceChildren();
  for (const template of templates) {
    const option = document.createElement("option");
    option.value = template.id;
    option.textContent = template.name;
    select.appendChild(option);
  }
  if (want && !templates.some((template) => template.id === want)) {
    const option = document.createElement("option");
    option.value = want;
    option.textContent = `${want} (nicht gefunden)`;
    select.appendChild(option);
  }
  if (want) select.value = want;
  renderShellHint();
}

function updateDialog() {
  if (!handoff) return;
  const field = $<HTMLTextAreaElement>("#agentCommand");
  field.value = handoff.command;
  field.select();
  $("#agentContextPath").textContent = handoff.contextFile;
}

function renderShellHint() {
  const shell = getAgentView()?.effectiveShell ?? "posix";
  const hint = shell === "fish" ? "Für fish" : shell === "powershell" ? "Für PowerShell" : "Für bash/zsh";
  $("#agentShellHint").textContent = hint;
}

/// Beim Verlassen der Detailansicht darf kein Kommando eines anderen Videos
/// stehen bleiben.
export function resetAgentHandoff() {
  handoff = null;
  handoffVideoId = null;
  prepareGeneration += 1;
  const field = document.querySelector<HTMLTextAreaElement>("#agentCommand");
  if (field) field.value = "";
  const path = document.querySelector<HTMLElement>("#agentContextPath");
  if (path) path.textContent = "";
}

/// Beim Wechsel auf ein anderes Video gehoert ein noch offener Dialog zum
/// alten Video: schliessen und zuruecksetzen, damit „Kopieren“/„Ordner oeffnen“
/// nie mit einer fremden Uebergabe arbeiten.
export function syncAgentHandoffVideo(videoId: number) {
  if (handoffVideoId === null || handoffVideoId === videoId) return;
  hideModal("#agentModal");
  resetAgentHandoff();
}

export function bindAgentHandoffEvents() {
  $("#agentHandoffBtn").addEventListener("click", () => void startHandoff());
  $("#agentModalClose").addEventListener("click", () => hideModal("#agentModal"));
  $("#agentTemplate").addEventListener("change", () => void reprepare());
  $("#agentCopy").addEventListener("click", () => void copyFromDialog());
  $("#agentReveal").addEventListener("click", () => void revealContext());
  onAgentViewChanged(() => renderShellHint());
}
