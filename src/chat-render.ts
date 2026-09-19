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

function truncate(value: string, max: number): string {
  return value.length > max ? `${value.slice(0, max)}…` : value;
}

/// Kopfzeile eines Schritts: gleiche Regel wie das Event-Label.
function stepHeadline(call: ToolCallInfo | undefined): string {
  if (!call) return "Werkzeug";
  const args = parseArguments(call.arguments);
  if (call.name === "web_search" && typeof args.query === "string") {
    return truncate(`Sucht: ${args.query}`, 120);
  }
  if (call.name === "fetch_page" && typeof args.url === "string") {
    return truncate(`Liest: ${hostAndPath(args.url)}`, 120);
  }
  return call.name || "Werkzeug";
}

/// Inhalt ohne die Delimiter-Zeilen, auf 1 500 Zeichen gekuerzt.
function cleanToolContent(content: string): string {
  const lines = content.split("\n");
  if (lines.length && /^===.*===$/.test(lines[0].trim())) lines.shift();
  if (lines.length && /^===.*===$/.test(lines[lines.length - 1].trim())) lines.pop();
  return truncate(lines.join("\n").trim(), 1500);
}

/// Eingeklappte Werkzeug-Schritte des gespeicherten Verlaufs.
export function buildToolSteps(
  messages: ChatMessageRecord[],
  calls: Map<string, ToolCallInfo>,
): HTMLDetailsElement {
  const details = document.createElement("details");
  details.className = "chat-tool";
  const summary = document.createElement("summary");
  summary.textContent = `Websuche: ${messages.length} Schritte`;
  details.append(summary);
  for (const message of messages) {
    const step = document.createElement("div");
    step.className = "chat-tool-step-detail";
    const head = document.createElement("div");
    head.className = "chat-tool-step-head";
    head.textContent = stepHeadline(calls.get(message.toolCallId ?? ""));
    const body = document.createElement("div");
    body.className = "chat-tool-body";
    // Nur Text: Werkzeug-Ausgaben sind untrusted.
    body.textContent = cleanToolContent(message.content);
    step.append(head, body);
    details.append(step);
  }
  return details;
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
  list.className = "chat-tool-activity";
  list.dataset.toolActivity = run.requestId;
  for (const step of run.tools) {
    const line = document.createElement("div");
    line.className = `chat-tool-step chat-tool-step--${step.status}`;
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
      if (hasText || steps.length) {
        const { row, bubble } = buildAssistantMessage(
          message.provider ?? null,
          message.model ?? null,
        );
        if (!hasText) {
          // Assistant mit Tool-Aufrufen und leerem Text ist keine Blase.
          bubble.remove();
        }
        if (steps.length) row.append(buildToolSteps(steps, callsById));
        root.append(row);
        if (hasText) {
          await renderMarkdownInto(bubble, message.content, {
            mermaid: true,
            gen,
            stripFence: false,
            target: "chat",
          });
        }
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
    if (orphans.length) root.append(buildToolSteps(orphans, new Map()));
  }
  if (gen === state.chatRenderGen && wasAtBottom) scrollChatToBottom(true);
}
