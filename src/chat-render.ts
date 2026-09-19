// Rendern des Chat-Verlaufs (DOM). Alle Texte, die kein Markdown sind, gehen
// ausschliesslich ueber `textContent`; nur Assistentenantworten laufen durch
// den Markdown-/DOMPurify-Pfad.

import { $ } from "./dom-utils";
import { state, type ChatRun } from "./state";
import { renderMarkdownInto } from "./summary-view";
import type { ChatMessageRecord } from "./types";

const AUTOSCROLL_TOLERANCE_PX = 40;

export function chatMessagesEl(): HTMLElement {
  return $("#chatMessages");
}

export function isChatAtBottom(root: HTMLElement): boolean {
  return root.scrollHeight - root.scrollTop - root.clientHeight < AUTOSCROLL_TOLERANCE_PX;
}

export function scrollChatToBottom(force = false) {
  const root = chatMessagesEl();
  if (force || isChatAtBottom(root)) {
    root.scrollTop = root.scrollHeight;
  }
}

export function clearChatMessages() {
  chatMessagesEl().textContent = "";
}

export function buildMessageRow(role: "user" | "assistant"): HTMLDivElement {
  const row = document.createElement("div");
  // Eigener Klassenname: `.chat-message` gehoert dem Modell-Testchat in den
  // Einstellungen und stylt dort jeden Absatz als eigenen Kasten.
  row.className = `chat-row chat-row--${role}`;
  return row;
}

export function buildBubble(text: string): HTMLDivElement {
  const bubble = document.createElement("div");
  bubble.className = "chat-bubble";
  // Benutzertext ausschliesslich als Text - niemals als HTML.
  bubble.textContent = text;
  return bubble;
}

export function buildAssistantMessage(
  provider: string | null,
  model: string | null,
): { row: HTMLDivElement; bubble: HTMLDivElement } {
  const row = buildMessageRow("assistant");
  const bubble = document.createElement("div");
  bubble.className = "chat-bubble";
  row.append(bubble);
  const label = [provider, model].filter((part): part is string => !!part && !!part.trim()).join(" · ");
  if (label) {
    const meta = document.createElement("div");
    meta.className = "chat-meta";
    meta.textContent = label;
    row.append(meta);
  }
  return { row, bubble };
}

type ToolCallInfo = { id: string; name: string; arguments: string };

/// Werkzeug-Aufrufe einer Assistant-Nachricht (fuer die Zuordnung der Schritte).
export function toolCallInfos(message: ChatMessageRecord): ToolCallInfo[] {
  if (!Array.isArray(message.toolCalls)) return [];
  return message.toolCalls.map((call) => {
    const entry = call as {
      id?: string;
      function?: { name?: string; arguments?: string };
    };
    return {
      id: entry.id ?? "",
      name: entry.function?.name ?? "",
      arguments: entry.function?.arguments ?? "",
    };
  });
}

function parseArguments(raw: string): Record<string, unknown> {
  try {
    const value = JSON.parse(raw) as unknown;
    return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}

function hostAndPath(url: string): string {
  try {
    const parsed = new URL(url);
    const path = parsed.pathname === "/" ? "" : parsed.pathname;
    return `${parsed.host}${path}`;
  } catch {
    return url;
  }
}

/// Kuerzt nach Codepunkten, damit kein Surrogatpaar zerteilt wird (sonst
/// entstehen Ersatzzeichen im Text).
function truncate(value: string, max: number): string {
  const chars = Array.from(value);
  return chars.length > max ? `${chars.slice(0, max).join("")}…` : value;
}

/// Kopfzeile eines Schritts: gleiche Regel wie das Event-Label (voll, ohne
/// Kuerzung - die Anzeige kuerzt per CSS, der `title` zeigt alles).
function stepHeadline(call: ToolCallInfo | undefined): string {
  if (!call) return "Werkzeug";
  const args = parseArguments(call.arguments);
  if (call.name === "web_search" && typeof args.query === "string") {
    return `Sucht: ${args.query}`;
  }
  if (call.name === "fetch_page" && typeof args.url === "string") {
    return `Liest: ${hostAndPath(args.url)}`;
  }
  return call.name || "Werkzeug";
}

/// Feste Hinweiszeile der App (Schlussrunde) - nur in der Anzeige ausblenden.
const APP_NOTE_PREFIX = "Hinweis der App:";

/// Inhalt ohne Delimiter-Zeilen und ohne die App-Schlusszeile, auf 1 500
/// Zeichen gekuerzt. Gespeichert/gesendet bleibt der volle Inhalt.
function cleanToolContent(content: string): string {
  const lines = content.split("\n").filter((line) => {
    const trimmed = line.trim();
    if (/^===.*===$/.test(trimmed)) return false;
    return !trimmed.startsWith(APP_NOTE_PREFIX);
  });
  return truncate(lines.join("\n").trim(), 1500);
}

/// Ein aufklappbarer Bereich pro Werkzeug-Schritt (F3): `<summary>` ist das
/// Label, der aufgeklappte Bereich enthaelt nur das Ergebnis dieses Schritts.
/// Fehler-Schritte tragen den Fehlertext in der Kopfzeile und keinen Koerper.
export function buildToolStep(
  message: ChatMessageRecord,
  call: ToolCallInfo | undefined,
): HTMLDetailsElement {
  const details = document.createElement("details");
  details.className = "chat-tool chat-tool-step";
  const summary = document.createElement("summary");
  const isError = message.content.trimStart().startsWith("Fehler:");
  const headline = stepHeadline(call);
  const full = isError ? `${headline} – ${errorText(message.content)}` : headline;
  // Anzeige gekuerzt, vollstaendiges Label im title.
  summary.textContent = truncate(full, 120);
  summary.title = full;
  if (isError) details.classList.add("chat-tool-step--error");
  details.append(summary);
  if (!isError) {
    const body = document.createElement("div");
    body.className = "chat-tool-body";
    // Nur Text: Werkzeug-Ausgaben sind untrusted.
    body.textContent = cleanToolContent(message.content);
    details.append(body);
  }
  return details;
}

/// Haengt Schritte an die letzte Gruppe an (oder legt eine neue an). Die
/// Gruppenzeile wird auf die Gesamtzahl aktualisiert.
function appendStepsToGroup(
  root: HTMLElement,
  steps: ChatMessageRecord[],
  calls: Map<string, ToolCallInfo>,
) {
  let group = root.lastElementChild;
  if (!group || !group.classList.contains("chat-tool-steps")) {
    group = document.createElement("div");
    group.className = "chat-tool-steps";
    group.append(buildToolGroup(0));
    root.append(group);
  }
  for (const message of steps) {
    group.append(buildToolStep(message, calls.get(message.toolCallId ?? "")));
  }
  const details = group.querySelectorAll("details.chat-tool-step").length;
  group.querySelector(".chat-tool-group")?.replaceWith(buildToolGroup(details));
}

/// Feste Gruppenzeile vor einer zusammenhaengenden Folge von Schritten.
export function buildToolGroup(count: number): HTMLParagraphElement {
  const group = document.createElement("p");
  group.className = "chat-tool-group";
  group.textContent = `Recherche · ${count} ${count === 1 ? "Schritt" : "Schritte"}`;
  return group;
}

/// Fehlertext fuer die Kopfzeile (ohne Delimiter, gekuerzt).
function errorText(content: string): string {
  return cleanToolContent(content);
}

/// Alle Schritte einer Folge als Liste (mit Gruppenzeile).
export function buildToolSteps(
  messages: ChatMessageRecord[],
  calls: Map<string, ToolCallInfo>,
): HTMLElement {
  const wrapper = document.createElement("div");
  wrapper.className = "chat-tool-steps";
  wrapper.append(buildToolGroup(messages.length));
  for (const message of messages) {
    wrapper.append(buildToolStep(message, calls.get(message.toolCallId ?? "")));
  }
  return wrapper;
}

export function removeProvisionalMessages(requestId: string) {
  chatMessagesEl()
    .querySelectorAll<HTMLElement>(`.chat-row[data-request-id="${requestId}"]`)
    .forEach((node) => node.remove());
}

/// Werkzeug-Aktivitaeten waehrend der Anfrage (Sucht/Liest, Status).
export function buildToolActivity(run: ChatRun): HTMLDivElement | null {
  if (!run.tools.length) return null;
  const list = document.createElement("div");
  list.className = "chat-tool-activity chat-tool-steps";
  list.dataset.toolActivity = run.requestId;
  for (const step of run.tools) {
    const line = document.createElement("div");
    line.className = `chat-tool-live-step chat-tool-live-step--${step.status}`;
    const prefix = step.kind === "search" ? "Sucht" : step.kind === "fetch" ? "Liest" : "";
    line.textContent = prefix ? `${prefix}: ${step.label}` : step.label;
    list.append(line);
  }
  return list;
}

/// Zeigt Frage und (leere) Antwortblase sofort an, waehrend die Anfrage laeuft.
export function appendProvisionalMessages(run: ChatRun) {
  const root = chatMessagesEl();
  root.querySelector(".chat-empty")?.remove();
  const questionRow = buildMessageRow("user");
  questionRow.dataset.requestId = run.requestId;
  questionRow.append(buildBubble(run.question));
  const { row: answerRow, bubble } = buildAssistantMessage(null, null);
  answerRow.dataset.requestId = run.requestId;
  bubble.dataset.streaming = run.requestId;
  root.append(questionRow, answerRow);
  renderToolActivity(run);
  scrollChatToBottom(true);
}

export function renderToolActivity(run: ChatRun) {
  const root = chatMessagesEl();
  const answerRow = root.querySelector<HTMLElement>(`.chat-row[data-request-id="${run.requestId}"]:last-child`);
  const existing = root.querySelector<HTMLElement>(`[data-tool-activity="${run.requestId}"]`);
  existing?.remove();
  const activity = buildToolActivity(run);
  if (!activity) return;
  if (answerRow) {
    answerRow.prepend(activity);
  } else {
    root.append(activity);
  }
}

export async function renderStreamingAnswer(run: ChatRun, mermaid: boolean, gen: number) {
  const bubble = chatMessagesEl().querySelector<HTMLElement>(
    `.chat-bubble[data-streaming="${run.requestId}"]`,
  );
  if (!bubble) return;
  await renderMarkdownInto(bubble, run.answer, {
    mermaid,
    gen,
    stripFence: false,
    target: "chat",
  });
}

export async function renderChatMessages(messages: ChatMessageRecord[], gen: number) {
  const root = chatMessagesEl();
  const wasAtBottom = isChatAtBottom(root);
  root.textContent = "";
  if (!messages.length) {
    const empty = document.createElement("p");
    empty.className = "empty chat-empty";
    empty.textContent = "Noch keine Nachrichten";
    root.append(empty);
    return;
  }
  let index = 0;
  while (index < messages.length) {
    if (gen !== state.chatRenderGen) return;
    const message = messages[index];
    if (message.role === "user") {
      const row = buildMessageRow("user");
      row.append(buildBubble(message.content));
      root.append(row);
      index += 1;
      continue;
    }
    if (message.role === "assistant") {
      // Tool-Schritte gehoeren ueber die tool_call_id zu dieser Antwort.
      const calls = toolCallInfos(message);
      const callsById = new Map(calls.map((call) => [call.id, call]));
      const steps: ChatMessageRecord[] = [];
      let next = index + 1;
      while (
        next < messages.length &&
        messages[next].role === "tool" &&
        (calls.length === 0 || callsById.has(messages[next].toolCallId ?? ""))
      ) {
        steps.push(messages[next]);
        next += 1;
      }
      const hasText = !!message.content.trim();
      if (hasText) {
        // Text-Blase: die Schritte dieses Turns folgen als neue Gruppe darunter.
        const { row, bubble } = buildAssistantMessage(
          message.provider ?? null,
          message.model ?? null,
        );
        root.append(row);
        await renderMarkdownInto(bubble, message.content, {
          mermaid: true,
          gen,
          stripFence: false,
          target: "chat",
        });
        if (steps.length) root.append(buildToolSteps(steps, callsById));
      } else if (steps.length) {
        // Ohne sichtbare Blase: an die laufende Gruppe anhaengen.
        appendStepsToGroup(root, steps, callsById);
      }
      index = next;
      continue;
    }
    // Verwaiste Tool-Nachrichten (ohne zugehoerige Antwort) sammeln.
    const orphans: ChatMessageRecord[] = [];
    while (index < messages.length && messages[index].role === "tool") {
      orphans.push(messages[index]);
      index += 1;
    }
    if (orphans.length) appendStepsToGroup(root, orphans, new Map());
  }
  if (gen === state.chatRenderGen && wasAtBottom) scrollChatToBottom(true);
}
