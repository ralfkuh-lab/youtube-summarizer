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

/// Eingeklappte Werkzeug-Schritte des gespeicherten Verlaufs.
export function buildToolSteps(messages: ChatMessageRecord[]): HTMLDetailsElement {
  const details = document.createElement("details");
  details.className = "chat-tool";
  const summary = document.createElement("summary");
  summary.textContent = `Websuche: ${messages.length} Schritte`;
  details.append(summary);
  for (const message of messages) {
    const body = document.createElement("div");
    body.className = "chat-tool-body";
    // Nur Text: Werkzeug-Ausgaben sind untrusted.
    body.textContent = message.content;
    details.append(body);
  }
  return details;
}

function hasToolCalls(message: ChatMessageRecord): boolean {
  const calls = message.toolCalls;
  return Array.isArray(calls) ? calls.length > 0 : !!calls;
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
    line.textContent = `${step.kind === "search" ? "Sucht" : "Liest"}: ${step.label}`;
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
  let pendingTools: ChatMessageRecord[] = [];
  for (const message of messages) {
    if (gen !== state.chatRenderGen) return;
    if (message.role === "tool") {
      pendingTools.push(message);
      continue;
    }
    if (message.role === "user") {
      const row = buildMessageRow("user");
      row.append(buildBubble(message.content));
      root.append(row);
      continue;
    }
    if (message.role === "assistant") {
      // Assistant-Nachrichten mit Tool-Aufrufen und ohne Text sind kein Inhalt.
      if (!message.content.trim() && hasToolCalls(message)) continue;
      const { row, bubble } = buildAssistantMessage(
        message.provider ?? null,
        message.model ?? null,
      );
      if (pendingTools.length) {
        row.prepend(buildToolSteps(pendingTools));
        pendingTools = [];
      }
      root.append(row);
      await renderMarkdownInto(bubble, message.content, {
        mermaid: true,
        gen,
        stripFence: false,
        target: "chat",
      });
      continue;
    }
  }
  if (pendingTools.length) root.append(buildToolSteps(pendingTools));
  if (gen === state.chatRenderGen && wasAtBottom) scrollChatToBottom(true);
}
