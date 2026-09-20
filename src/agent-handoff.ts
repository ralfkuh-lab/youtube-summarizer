// Button „An Agent übergeben“ und der zugehörige Dialog. Stufe 1: Kontextdatei
// schreiben, Kommando in die Zwischenablage kopieren — die App startet keinen
// Prozess. Revision 3: Was in die Kontextdatei kommt (Transkript,
// Zusammenfassungs-Versionen, Chats), wählt der Benutzer pro Übergabe im Dialog.

import { invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { getAgentView, loadAgentView, onAgentViewChanged } from "./agent-settings";
import { labelFor } from "./chat-context";
import { $, errorMessage, hideModal, showModal } from "./dom-utils";
import { getActiveVideo, setBusy, setStatus, state } from "./state";
import { formatShortDateTime } from "./utils";
import type {
  AgentAvailable,
  AgentChatOption,
  AgentHandoff,
  AgentHandoffSelection,
  AgentSummaryOption,
  AgentTemplate,
} from "./types";

/// „copy“ löst neu auf und kopiert (Öffnen, Vorlagenwechsel), „update“ schreibt
/// nur die Kontextdatei neu (Auswahländerung).
type HandoffMode = "copy" | "update";

type PrepareOptions = {
  mode: HandoffMode;
  /// `null` = die aktive Vorlage aus den Einstellungen.
  templateId: string | null;
  /// `null` = Vorbelegung des Backends (nur beim ersten Öffnen).
  selection: AgentHandoffSelection | null;
  openDialog?: boolean;
};

let handoff: AgentHandoff | null = null;
let handoffVideoId: number | null = null;
/// Zaehlt jede Vorbereitung. Nur die juengste Antwort darf Anzeige, Auswahl,
/// Zwischenablage und Status aendern (H4b/G11/G19).
let prepareGeneration = 0;
/// Zuletzt wirksame Auswahl je Video; geht beim naechsten Oeffnen mit.
const rememberedSelections = new Map<number, AgentHandoffSelection>();
/// Die **gewuenschte** Auswahl im Dialog: einzige Quelle der Wahrheit. Jeder
/// Klick aendert sie (und die Anzeige) sofort, jede Anfrage sendet sie
/// vollstaendig; erst eine gueltige, aktuelle Antwort uebernimmt die wirksame
/// Auswahl.
let pendingSelection: AgentHandoffSelection | null = null;

function currentSelection(): AgentHandoffSelection {
  return pendingSelection ?? { transcript: true, summaryIds: null, chatIds: [] };
}

async function startHandoff() {
  const video = getActiveVideo();
  if (!video || state.busy) return;
  setBusy(true, "Kontext wird vorbereitet...");
  try {
    if (!getAgentView()) await loadAgentView();
    await prepare({
      mode: "copy",
      templateId: null,
      selection: rememberedSelections.get(video.id) ?? null,
      openDialog: true,
    });
  } finally {
    setBusy(false);
  }
}

/// Vorbereiten und danach entweder kopieren und den Dialog öffnen oder nur die
/// Anzeige aktualisieren. Fehler landen im Status; die Anzeige bleibt dabei auf
/// dem letzten gültigen Stand.
async function prepare(options: PrepareOptions) {
  const video = getActiveVideo();
  if (!video) return;
  const videoId = video.id;
  const generation = ++prepareGeneration;
  const args: Record<string, unknown> = { videoId, templateId: options.templateId };
  if (options.selection) args.selection = options.selection;

  let prepared: AgentHandoff;
  try {
    prepared = await invoke<AgentHandoff>("agent_prepare", args);
  } catch (error) {
    if (!isCurrent(generation, videoId)) return;
    setStatus(errorMessage(error));
    if (handoff && handoffVideoId === videoId) {
      // Die Anzeige bleibt auf dem letzten gueltigen Stand.
      pendingSelection = handoff.selection;
      renderDialog(handoff, false);
    }
    return;
  }
  // (a) Videowechsel waehrend `agent_prepare` oder (b) schnellere juengere
  // Antwort: Ergebnis verwerfen.
  if (!isCurrent(generation, videoId)) return;

  if (options.mode === "copy") {
    const copied = await copyText(prepared.command);
    if (!isCurrent(generation, videoId)) return;
    applyHandoff(prepared, videoId, options.templateId, true);
    if (options.openDialog) showModal("#agentModal");
    updateCopyHint(copied);
    setStatus(copyStatus(copied, prepared.workdir));
  } else {
    applyHandoff(prepared, videoId, options.templateId, false);
    setStatus(`Kontextdatei aktualisiert – ${prepared.contextChars.toLocaleString("de-DE")} Zeichen`);
  }
}

function isCurrent(generation: number, videoId: number): boolean {
  return generation === prepareGeneration && getActiveVideo()?.id === videoId;
}

/// Übernimmt eine gültige Antwort in Anzeige und gemerkte Auswahl. Beim reinen
/// Aktualisieren der Auswahl darf das Markieren des Kommandofelds den Fokus
/// nicht vom Kontextwähler nehmen.
function applyHandoff(
  prepared: AgentHandoff,
  videoId: number,
  templateId: string | null,
  selectCommand: boolean,
) {
  handoff = prepared;
  handoffVideoId = videoId;
  pendingSelection = prepared.selection;
  rememberedSelections.set(videoId, prepared.selection);
  renderTemplateOptions(templateId ?? getAgentView()?.config.activeTemplate ?? null);
  renderDialog(prepared, selectCommand);
}

/// Vorlagenwechsel im Dialog: neu auflösen und erneut kopieren, mit der aktuell
/// eingestellten Auswahl.
async function reprepare() {
  if (state.busy || !handoff) return;
  await prepare({
    mode: "copy",
    templateId: currentTemplateId(),
    selection: currentSelection(),
  });
}

function currentTemplateId(): string {
  return $<HTMLSelectElement>("#agentTemplate").value;
}

/// Ein Klick auf den Kontextwaehler: Wunsch und Anzeige sofort nachziehen und
/// den vollstaendigen Stand senden. Nicht kopieren — das Kommando aendert sich
/// durch die Auswahl nicht.
function chooseSelection(next: AgentHandoffSelection) {
  pendingSelection = next;
  if (handoff) renderContext(handoff.available, next);
  void sendSelection(next);
}

async function sendSelection(selection: AgentHandoffSelection) {
  if (!handoff) return;
  await prepare({ mode: "update", templateId: currentTemplateId(), selection });
}

function toggleId(ids: number[], id: number, checked: boolean): number[] {
  const next = new Set(ids);
  if (checked) next.add(id);
  else next.delete(id);
  return [...next];
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

function renderDialog(prepared: AgentHandoff, selectCommand: boolean) {
  const field = $<HTMLTextAreaElement>("#agentCommand");
  field.value = prepared.command;
  if (selectCommand) field.select();
  $("#agentContextPath").textContent = prepared.contextFile;
  $("#agentContextChars").textContent = ` · ≈ ${prepared.contextChars.toLocaleString("de-DE")} Zeichen`;
  renderContext(prepared.available, currentSelection());
}

function renderShellHint() {
  const shell = getAgentView()?.effectiveShell ?? "posix";
  const hint =
    shell === "fish" ? "(für fish)" : shell === "powershell" ? "(für PowerShell)" : "(für bash/zsh)";
  $("#agentShellHint").textContent = hint;
}

// -------------------------------------------------------------- Auswahl --

/// Zeichnet den Kontextbereich komplett aus der uebergebenen Auswahl; es gibt
/// keinen zweiten Zustand im DOM, aus dem gelesen wuerde. Weil dabei alle
/// Regler ersetzt werden, wandert der Tastaturfokus danach auf denselben Regler
/// zurueck.
function renderContext(available: AgentAvailable, selection: AgentHandoffSelection) {
  const focusId = focusedContextId();
  const transcript = $<HTMLInputElement>("#agentCtxTranscript");
  transcript.checked = selection.transcript && available.hasTranscript;
  transcript.disabled = !available.hasTranscript;
  $("#agentCtxTranscriptLabel").textContent = available.hasTranscript
    ? "Transkript"
    : "Transkript (nicht vorhanden)";
  renderSummaries(available, selection);
  renderChats(available, selection);
  restoreContextFocus(focusId);
}

/// `id` des Reglers, der vor dem Neuzeichnen im Kontextbereich den Fokus hatte;
/// sonst `null` (der Fokus bleibt dann, wo er ist).
function focusedContextId(): string | null {
  const active = document.activeElement;
  const context = document.querySelector("#agentContext");
  if (!(active instanceof HTMLElement) || !context || !context.contains(active)) return null;
  return active.id || null;
}

/// Denselben Regler nach dem Neuzeichnen wieder fokussieren.
function restoreContextFocus(id: string | null) {
  if (!id) return;
  const element = document.getElementById(id);
  if (!(element instanceof HTMLElement)) return;
  if (element instanceof HTMLInputElement && element.disabled) return;
  element.focus();
}

/// Angehakte Versionen, wie der Dialog sie zeigt: `null` („neueste“) erscheint
/// als angehakte neueste Version.
function shownSummaryIds(available: AgentAvailable, selection: AgentHandoffSelection): number[] {
  if (selection.summaryIds !== null) return selection.summaryIds;
  const newest = available.summaries[0];
  return newest ? [newest.id] : [];
}

function renderSummaries(available: AgentAvailable, selection: AgentHandoffSelection) {
  const container = $("#agentCtxSummaries");
  container.replaceChildren(groupTitle("Zusammenfassungen"));
  if (!available.hasLatestSummary && !available.summaries.length) {
    container.appendChild(hintText("Keine Zusammenfassung vorhanden"));
    return;
  }
  if (!available.summaries.length) {
    // Altbestand ohne Versionsliste: nur die aktuelle Zusammenfassung.
    container.appendChild(
      checkboxRow(
        "agentCtxSummaryLatest",
        "Aktuelle Zusammenfassung",
        selection.summaryIds === null,
        (checked) => chooseSelection({ ...currentSelection(), summaryIds: checked ? null : [] }),
      ),
    );
    return;
  }
  const shown = shownSummaryIds(available, selection);
  for (const version of available.summaries) {
    container.appendChild(versionRow(version, shown, available));
  }
}

function renderChats(available: AgentAvailable, selection: AgentHandoffSelection) {
  const container = $("#agentCtxChats");
  container.replaceChildren(groupTitle("Chats"));
  if (!available.chats.length) {
    container.appendChild(hintText("Keine Chats vorhanden"));
    return;
  }
  for (const chat of available.chats) {
    container.appendChild(chatRow(chat, selection.chatIds.includes(chat.id)));
  }
}

/// Ein Klick macht aus „neueste“ eine ausdrückliche Liste: angezeigter Stand
/// plus/minus diese Version. Eine leere Liste heißt „keine Zusammenfassung“.
function versionRow(
  version: AgentSummaryOption,
  shown: number[],
  available: AgentAvailable,
): HTMLLabelElement {
  const label = labelFor({
    created_at: version.createdAt,
    model: version.model,
    provider: version.provider,
    options: version.options,
  });
  return checkboxRow(
    `agentCtxSummary-${version.id}`,
    label,
    shown.includes(version.id),
    (checked) => {
      const current = currentSelection();
      const ids = toggleId(shownSummaryIds(available, current), version.id, checked);
      chooseSelection({ ...current, summaryIds: ids });
    },
  );
}

/// Der Titel wird einzeilig gekürzt; die vollständige erste Frage steht im
/// Tooltip. Datum und Nachrichtenzahl bleiben immer sichtbar.
function chatRow(chat: AgentChatOption, selected: boolean): HTMLLabelElement {
  const meta = `· ${formatShortDateTime(chat.createdAt)} · ${messageCountLabel(chat.messageCount)}`;
  const row = checkboxRow(`agentCtxChat-${chat.id}`, chat.title, selected, (checked) => {
    const current = currentSelection();
    chooseSelection({ ...current, chatIds: toggleId(current.chatIds, chat.id, checked) });
  });
  row.title = chat.firstQuestion?.trim() || chat.title;
  const text = row.querySelector("span");
  if (text) {
    text.className = "agent-context-text";
    const title = document.createElement("span");
    title.className = "agent-context-title";
    title.textContent = chat.title;
    const metaSpan = document.createElement("span");
    metaSpan.className = "agent-context-meta";
    metaSpan.textContent = ` ${meta}`;
    text.replaceChildren(title, metaSpan);
  }
  return row;
}

function messageCountLabel(count: number): string {
  return count === 1 ? "1 Nachricht" : `${count} Nachrichten`;
}

function checkboxRow(
  id: string,
  label: string,
  selected: boolean,
  onChange: (checked: boolean) => void,
): HTMLLabelElement {
  const row = document.createElement("label");
  row.className = "agent-context-row";
  const input = document.createElement("input");
  input.type = "checkbox";
  input.id = id;
  input.checked = selected;
  input.addEventListener("change", () => onChange(input.checked));
  const text = document.createElement("span");
  text.textContent = label;
  row.append(input, text);
  return row;
}

function groupTitle(text: string): HTMLParagraphElement {
  const title = document.createElement("p");
  title.className = "agent-context-group";
  title.textContent = text;
  return title;
}

function hintText(text: string): HTMLParagraphElement {
  const hint = document.createElement("p");
  hint.className = "settings-hint";
  hint.textContent = text;
  return hint;
}

// ---------------------------------------------------------------- Zustand --

/// Beim Verlassen der Detailansicht darf kein Kommando eines anderen Videos
/// stehen bleiben.
export function resetAgentHandoff() {
  handoff = null;
  handoffVideoId = null;
  pendingSelection = null;
  prepareGeneration += 1;
  const field = document.querySelector<HTMLTextAreaElement>("#agentCommand");
  if (field) field.value = "";
  const path = document.querySelector<HTMLElement>("#agentContextPath");
  if (path) path.textContent = "";
  const chars = document.querySelector<HTMLElement>("#agentContextChars");
  if (chars) chars.textContent = "";
  for (const selector of ["#agentCtxSummaries", "#agentCtxChats"]) {
    document.querySelector<HTMLElement>(selector)?.replaceChildren();
  }
  const transcript = document.querySelector<HTMLInputElement>("#agentCtxTranscript");
  if (transcript) {
    transcript.checked = false;
    transcript.disabled = true;
  }
  const transcriptLabel = document.querySelector<HTMLElement>("#agentCtxTranscriptLabel");
  if (transcriptLabel) transcriptLabel.textContent = "Transkript";
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
  $("#agentCtxTranscript").addEventListener("change", (event) => {
    const target = event.target;
    if (!(target instanceof HTMLInputElement)) return;
    chooseSelection({ ...currentSelection(), transcript: target.checked });
  });
  $("#agentCopy").addEventListener("click", () => void copyFromDialog());
  $("#agentReveal").addEventListener("click", () => void revealContext());
  onAgentViewChanged(() => renderShellHint());
}
