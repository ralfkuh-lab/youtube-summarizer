// Kontext-Wähler des Chats: Auswahl (Transkript + Zusammenfassungs-Versionen),
// Popover-Bedienung und Anzeige. Kein DOM auf Modulebene, alle Texte per
// `textContent`.

import { invoke } from "@tauri-apps/api/core";
import { $, errorMessage } from "./dom-utils";
import { getActiveVideo, setStatus, state } from "./state";
import { formatShortDateTime } from "./utils";
import type { ChatContextOptions, SummaryRecord } from "./types";

const DEFAULT_OPTIONS: ChatContextOptions = { transcript: true, summaryIds: null };

/// Beschriftung einer Zusammenfassungs-Version; auch der Übergabe-Dialog nutzt
/// sie (Revision 3). Die Felder sind bewusst Serde-Form (`created_at`), damit
/// `SummaryRecord` unverändert bleibt.
export type SummaryLabelInput = {
  created_at: string;
  model?: string | null;
  provider?: string | null;
  options?: string | null;
};
/// Wie im Backend: hoechstens so viele Versionen im Kontext.
const MAX_VERSIONS = 5;
const TOO_MANY_VERSIONS = "Höchstens 5 Zusammenfassungen";

/// Vorbelegung fuer neue Chats: Auswahl des zuletzt benutzten Chats je Video.
const lastUsed = new Map<number, ChatContextOptions>();

let options: ChatContextOptions = { ...DEFAULT_OPTIONS };
let versions: SummaryRecord[] = [];
let hasTranscript = false;
/// Hat das Video mindestens eine Zusammenfassung (fuer „Neueste“ ohne Transkript)?
let hasSummary = false;
let open = false;

export function getChatContextOptions(): ChatContextOptions {
  return { ...options, summaryIds: options.summaryIds === null ? null : [...options.summaryIds] };
}

export function setChatContextOptions(next: ChatContextOptions) {
  options = {
    transcript: next.transcript,
    summaryIds: next.summaryIds === null ? null : [...next.summaryIds],
  };
  renderContextButton();
  renderMenu();
}

export function setChatContextVersions(summaries: SummaryRecord[]) {
  versions = summaries;
  hasSummary = hasSummary || summaries.length > 0;
  renderMenu();
}

export function setChatContextHasTranscript(value: boolean) {
  hasTranscript = value;
  renderMenu();
}

/// Video-Zustand fuer die Gueltigkeitspruefung: Transkript und mindestens eine
/// Zusammenfassung.
export function setChatContextAvailability(transcript: boolean, summary: boolean) {
  hasTranscript = transcript;
  hasSummary = summary;
  renderMenu();
}

/// Die Auswahl ist nur dann ungueltig, wenn weder Transkript noch eine
/// Zusammenfassung im Kontext landet.
/// G1: gueltig, wenn ein Transkript mitgeht, „Neueste“ mit vorhandener
/// Zusammenfassung gewaehlt ist oder mindestens eine Version angehakt ist.
/// `summaryIds: []` ohne Transkript bleibt ungueltig.
export function isChatContextValid(): boolean {
  if (options.transcript && hasTranscript) return true;
  if (options.summaryIds === null) return hasSummary || versions.length > 0;
  return options.summaryIds.length > 0;
}

export function labelFor(row: SummaryLabelInput): string {
  const model = row.model?.trim() || row.provider?.trim() || "unbekannt";
  const date = formatShortDateTime(row.created_at);
  const preset = presetName(row.options);
  return preset ? `${date} – ${model} · ${preset}` : `${date} – ${model}`;
}

function presetName(raw?: string | null): string | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as { presetId?: string };
    return parsed.presetId?.trim() || null;
  } catch {
    return null;
  }
}

/// Kompakte Beschriftung; der vollstaendige Text steht im `title`.
export function contextSummaryText(): string {
  const count = (options.summaryIds ?? []).length;
  const withTranscript = options.transcript && hasTranscript;
  const summaries =
    options.summaryIds === null
      ? "neueste"
      : count === 1
        ? "1 Zus."
        : `${count} Zus.`;
  if (withTranscript) return `Transkript + ${summaries}`;
  return count === 0 && options.summaryIds !== null
    ? "keine Zusammenfassung"
    : `${summaries} (ohne Transkript)`;
}

/// Vollstaendiger Text fuer den Tooltip.
export function contextSummaryTitle(): string {
  const count = (options.summaryIds ?? []).length;
  const withTranscript = options.transcript && hasTranscript;
  const summaries =
    options.summaryIds === null
      ? "neueste Zusammenfassung"
      : count === 1
        ? "1 Zusammenfassung"
        : `${count} Zusammenfassungen`;
  if (withTranscript) return `Transkript + ${summaries}`;
  return count === 0 && options.summaryIds !== null
    ? "keine Zusammenfassung"
    : `${summaries}, ohne Transkript`;
}

function renderContextButton() {
  const label = $("#chatContextLabel");
  if (label) label.textContent = `Kontext: ${contextSummaryText()}`;
  const button = document.querySelector<HTMLButtonElement>("#chatContextBtn");
  if (button) {
    button.disabled = !!activeRunForCurrentVideo();
    button.title = `Kontext: ${contextSummaryTitle()}`;
  }
}

function activeRunForCurrentVideo(): boolean {
  const video = getActiveVideo();
  return !!video && state.chatRuns.has(video.id);
}

function renderMenu() {
  const menu = document.querySelector<HTMLDivElement>("#chatContextMenu");
  if (!menu) return;
  const transcript = document.querySelector<HTMLInputElement>("#chatContextTranscript");
  const hint = document.querySelector<HTMLParagraphElement>("#chatContextHint");
  const invalid = document.querySelector<HTMLParagraphElement>("#chatContextInvalid");
  if (transcript) {
    transcript.checked = options.transcript;
    transcript.disabled = !hasTranscript;
  }
  if (hint) {
    hint.hidden = hasTranscript;
    hint.textContent = hasTranscript ? "" : "Dieses Video hat kein Transkript";
  }

  const list = document.querySelector<HTMLDivElement>("#chatContextVersions");
  if (!list) return;
  list.textContent = "";
  setContextNotice("");
  const newest = options.summaryIds === null;
  const none = options.summaryIds !== null && options.summaryIds.length === 0;
  list.append(
    choiceRow("context-newest", "Neueste (automatisch)", newest, () => {
      // Leert die Einzelauswahl.
      setChatContextOptions({ transcript: options.transcript, summaryIds: null });
      void persist();
    }),
    choiceRow("context-none", "Keine", none, () => {
      setChatContextOptions({ transcript: options.transcript, summaryIds: [] });
      void persist();
    }),
  );

  const selectedIds = options.summaryIds ?? [];
  for (const row of versions) {
    const selected = selectedIds.includes(row.id);
    list.append(
      versionRow(row, selected, selectedIds.length >= MAX_VERSIONS, () => {
        if (selected) {
          const next = selectedIds.filter((id) => id !== row.id);
          // Letzte Checkbox abgewaehlt -> wieder "Neueste".
          setChatContextOptions({
            transcript: options.transcript,
            summaryIds: next.length ? next : null,
          });
        } else {
          if (selectedIds.length >= MAX_VERSIONS) {
            setContextNotice(TOO_MANY_VERSIONS);
            return;
          }
          setChatContextOptions({
            transcript: options.transcript,
            summaryIds: [...selectedIds, row.id],
          });
        }
        void persist();
      }),
    );
  }

  if (invalid) {
    const show = !isChatContextValid();
    invalid.hidden = !show;
    invalid.textContent = show
      ? "Kein Kontext gewählt – bitte Transkript oder eine Zusammenfassung aktivieren"
      : "";
  }
  updateSendButton();
}

function choiceRow(
  id: string,
  label: string,
  selected: boolean,
  onSelect: () => void,
): HTMLLabelElement {
  return inputRow(id, label, "radio", selected, onSelect, false);
}

/// Einzelne Version: Checkbox (Mehrfachauswahl).
function versionRow(
  summary: SummaryRecord,
  selected: boolean,
  atLimit: boolean,
  onToggle: () => void,
): HTMLLabelElement {
  return inputRow(
    `context-version-${summary.id}`,
    labelFor(summary),
    "checkbox",
    selected,
    onToggle,
    !selected && atLimit,
  );
}

function inputRow(
  id: string,
  label: string,
  type: "radio" | "checkbox",
  selected: boolean,
  onChange: () => void,
  disabled: boolean,
): HTMLLabelElement {
  const row = document.createElement("label");
  row.className = "chat-context-choice";
  const input = document.createElement("input");
  input.type = type;
  if (type === "radio") input.name = "chatContextChoice";
  input.id = id;
  input.checked = selected;
  input.disabled = disabled;
  input.addEventListener("change", onChange);
  const text = document.createElement("span");
  text.textContent = label;
  row.append(input, text);
  return row;
}

/// Hinweiszeile (z. B. Limit) setzen; wird beim Neuaufbau geloescht.
function setContextNotice(message: string) {
  const notice = document.querySelector<HTMLParagraphElement>("#chatContextNotice");
  if (!notice) return;
  notice.hidden = !message;
  notice.textContent = message;
}

function updateSendButton() {
  const send = document.querySelector<HTMLButtonElement>("#chatSend");
  if (!send) return;
  const run = activeRunForCurrentVideo();
  send.disabled = run ? false : !isChatContextValid();
}

async function persist() {
  const chatId = state.activeChatId;
  const video = getActiveVideo();
  if (video) lastUsed.set(video.id, getChatContextOptions());
  if (chatId === null) return;
  try {
    await invoke("chat_context_set", { chatId, options: getChatContextOptions() });
  } catch (error) {
    setStatus(errorMessage(error));
  }
}

export function toggleChatContextMenu(force?: boolean) {
  const menu = document.querySelector<HTMLDivElement>("#chatContextMenu");
  if (!menu) return;
  open = force ?? !open;
  menu.hidden = !open;
  const button = document.querySelector<HTMLButtonElement>("#chatContextBtn");
  if (button) button.setAttribute("aria-expanded", open ? "true" : "false");
  if (!open) return;
  renderMenu();
  positionMenu();
}

/// Popover unter dem Kontext-Button, linksbuendig zu ihm und im Fenster
/// bleibend (nicht unter der Modellauswahl).
function positionMenu() {
  const menu = document.querySelector<HTMLDivElement>("#chatContextMenu");
  const button = document.querySelector<HTMLButtonElement>("#chatContextBtn");
  const toolbar = document.querySelector<HTMLDivElement>(".chat-toolbar");
  if (!menu || !button || !toolbar) return;
  const buttonBox = button.getBoundingClientRect();
  const toolbarBox = toolbar.getBoundingClientRect();
  const width = menu.offsetWidth;
  const margin = 8;
  const maxLeft = window.innerWidth - width - margin;
  const left = Math.max(margin, Math.min(buttonBox.left, maxLeft));
  menu.style.left = `${left}px`;
  menu.style.right = "auto";
  menu.style.top = `${toolbarBox.bottom - 2}px`;
  menu.style.position = "fixed";
  menu.style.maxHeight = `${Math.max(160, window.innerHeight - toolbarBox.bottom - margin)}px`;
}

export function closeChatContextMenu() {
  toggleChatContextMenu(false);
}

export function bindChatContextEvents() {
  document.querySelector<HTMLButtonElement>("#chatContextBtn")?.addEventListener("click", (event) => {
    event.stopPropagation();
    toggleChatContextMenu();
  });
  document.querySelector<HTMLInputElement>("#chatContextTranscript")?.addEventListener("change", (event) => {
    const target = event.target;
    if (!(target instanceof HTMLInputElement)) return;
    setChatContextOptions({ transcript: target.checked, summaryIds: options.summaryIds });
    void persist();
  });
  window.addEventListener("resize", () => {
    if (open) positionMenu();
  });
  document.addEventListener("click", (event) => {
    if (!open) return;
    const target = event.target;
    if (target instanceof Element && target.closest("#chatContextMenu, #chatContextBtn")) return;
    closeChatContextMenu();
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && open) {
      event.preventDefault();
      closeChatContextMenu();
    }
  });
}

/// Beim Wechsel auf einen Chat (oder „Neuer Chat“) die Auswahl uebernehmen.
export function applyChatContextFor(chatId: number | null, stored: ChatContextOptions | null) {
  const video = getActiveVideo();
  if (stored) {
    setChatContextOptions(stored);
    return;
  }
  if (chatId === null && video) {
    const remembered = lastUsed.get(video.id);
    setChatContextOptions(remembered ?? DEFAULT_OPTIONS);
    return;
  }
  setChatContextOptions(DEFAULT_OPTIONS);
}

/// Wird der Schalter waehrend einer Anfrage gesperrt/entsperrt.
export function refreshChatContextButton() {
  renderContextButton();
  updateSendButton();
}
