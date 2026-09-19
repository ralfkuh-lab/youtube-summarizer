import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ensureAiData, fillModelPicker } from "./ai-config";
import { seekVideo } from "./detail";
import { $, confirmDialog, errorMessage } from "./dom-utils";
import { getActiveVideo, setStatus, state, type ChatRun } from "./state";
import { renderMarkdownInto } from "./summary-view";
import type { Chat, ChatMessageRecord, ChatTurnResult, Video } from "./types";

const CHAT_MODEL_KEY = "chatModel";
const STREAM_EVENT = "ai:chat_stream";
const AUTOSCROLL_TOLERANCE_PX = 40;
const INPUT_MAX_LINES = 6;

let chatList: Chat[] = [];

function parseModelValue(value: string): [string | null, string | null] {
  if (!value) return [null, null];
  try {
    const [providerId, modelId] = JSON.parse(value) as [string, string];
    return [providerId ?? null, modelId ?? null];
  } catch {
    return [null, null];
  }
}

function formatChatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date
    .toLocaleString("de-DE", {
      day: "2-digit",
      month: "2-digit",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    })
    .replace(",", "");
}

export function hasChatTranscript(video: Video | null): boolean {
  return !!video && video.has_transcript && !!video.transcript?.trim();
}

function activeRun(video: Video | null): ChatRun | undefined {
  return video ? state.chatRuns.get(video.id) : undefined;
}

/// Der Chat kennt nur seinen eigenen In-flight-Zustand; setBusy bleibt aussen
/// vor, damit Videowechsel und andere Aktionen nicht blockiert werden.
function updateChatControls(video: Video | null) {
  const hasTranscript = hasChatTranscript(video);
  const run = activeRun(video);
  const sendBtn = $<HTMLButtonElement>("#chatSend");
  sendBtn.textContent = run ? "Stopp" : "Senden";
  sendBtn.disabled = !hasTranscript;
  const input = $<HTMLTextAreaElement>("#chatInput");
  input.disabled = !hasTranscript;
  input.placeholder = hasTranscript ? "Frage zum Video…" : "Für den Chat wird ein Transkript benötigt";
  $("#chatHint").hidden = hasTranscript;
  $<HTMLButtonElement>("#chatNew").disabled = !!run || !hasTranscript;
  $<HTMLButtonElement>("#chatDelete").disabled =
    !!run || state.activeChatId === null || !hasTranscript;
  $<HTMLSelectElement>("#chatModel").disabled = !!run;
}

function fillChatSelect() {
  const select = $<HTMLSelectElement>("#chatSelect");
  select.textContent = "";
  const empty = document.createElement("option");
  empty.value = "";
  empty.textContent = chatList.length ? "Neuer Chat" : "Noch kein Chat";
  select.append(empty);
  for (const chat of chatList) {
    const option = document.createElement("option");
    option.value = String(chat.id);
    option.textContent = `${chat.title} – ${formatChatDate(chat.updatedAt)}`;
    select.append(option);
  }
  select.value = state.activeChatId === null ? "" : String(state.activeChatId);
}

function chatMessagesEl(): HTMLElement {
  return $("#chatMessages");
}

function isChatAtBottom(root: HTMLElement): boolean {
  return root.scrollHeight - root.scrollTop - root.clientHeight < AUTOSCROLL_TOLERANCE_PX;
}

function scrollChatToBottom(force = false) {
  const root = chatMessagesEl();
  if (force || isChatAtBottom(root)) {
    root.scrollTop = root.scrollHeight;
  }
}

function buildMessageRow(role: "user" | "assistant"): HTMLDivElement {
  const row = document.createElement("div");
  row.className = `chat-message chat-message--${role}`;
  return row;
}

function buildBubble(text: string): HTMLDivElement {
  const bubble = document.createElement("div");
  bubble.className = "chat-bubble";
  // Benutzertext ausschliesslich als Text - niemals als HTML.
  bubble.textContent = text;
  return bubble;
}

function buildAssistantMessage(
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

function buildToolMessage(content: string, provider: string | null, model: string | null): HTMLDivElement {
  const row = buildMessageRow("assistant");
  const details = document.createElement("details");
  details.className = "chat-tool";
  const summary = document.createElement("summary");
  summary.textContent = [provider, model].filter(Boolean).join(" · ") || "Werkzeug";
  const body = document.createElement("div");
  body.className = "chat-tool-body";
  body.textContent = content;
  details.append(summary, body);
  row.append(details);
  return row;
}

function removeProvisionalMessages(requestId: string) {
  chatMessagesEl()
    .querySelectorAll<HTMLElement>(`.chat-message[data-request-id="${requestId}"]`)
    .forEach((node) => node.remove());
}

/// Zeigt Frage und (leere) Antwortblase sofort an, waehrend die Anfrage laeuft.
function appendProvisionalMessages(run: ChatRun) {
  const root = chatMessagesEl();
  root.querySelector(".chat-empty")?.remove();
  const questionRow = buildMessageRow("user");
  questionRow.dataset.requestId = run.requestId;
  questionRow.append(buildBubble(run.question));
  const { row: answerRow, bubble } = buildAssistantMessage(null, null);
  answerRow.dataset.requestId = run.requestId;
  bubble.dataset.streaming = run.requestId;
  root.append(questionRow, answerRow);
  scrollChatToBottom(true);
}

async function renderStreamingAnswer(run: ChatRun, mermaid: boolean, gen: number) {
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

async function renderChatMessages(messages: ChatMessageRecord[], gen: number) {
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
  for (const message of messages) {
    if (gen !== state.chatRenderGen) return;
    if (message.role === "user") {
      const row = buildMessageRow("user");
      row.append(buildBubble(message.content));
      root.append(row);
      continue;
    }
    if (message.role === "assistant") {
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
      continue;
    }
    root.append(
      buildToolMessage(message.content, message.provider ?? null, message.model ?? null),
    );
  }
  if (gen === state.chatRenderGen && wasAtBottom) scrollChatToBottom(true);
}

async function refreshChatList(videoId: number, gen: number) {
  let chats: Chat[];
  try {
    chats = await invoke<Chat[]>("chat_list", { videoId });
  } catch (error) {
    if (gen === state.chatRenderGen && getActiveVideo()?.id === videoId) setStatus(errorMessage(error));
    return;
  }
  if (gen !== state.chatRenderGen || getActiveVideo()?.id !== videoId) return;
  chatList = chats;
  fillChatSelect();
}

async function loadActiveChatMessages(video: Video, gen: number) {
  const chatId = state.activeChatId;
  let messages: ChatMessageRecord[] = [];
  if (chatId !== null) {
    try {
      messages = await invoke<ChatMessageRecord[]>("chat_messages", { chatId });
    } catch (error) {
      if (gen === state.chatRenderGen && getActiveVideo()?.id === video.id) {
        setStatus(errorMessage(error));
      }
      return;
    }
  }
  if (gen !== state.chatRenderGen || getActiveVideo()?.id !== video.id || state.activeChatId !== chatId) {
    return;
  }
  await renderChatMessages(messages, gen);
  const run = state.chatRuns.get(video.id);
  if (run && run.chatId === chatId) {
    appendProvisionalMessages(run);
    await renderStreamingAnswer(run, false, gen);
  }
  updateChatControls(video);
}

/// Baut den Chat-Tab fuer ein Video neu auf (auch beim Aktivieren des Tabs).
export async function renderChatTab(video: Video) {
  const gen = ++state.chatRenderGen;
  chatList = [];
  fillChatSelect();
  updateChatControls(video);
  await refreshChatList(video.id, gen);
  if (gen !== state.chatRenderGen || getActiveVideo()?.id !== video.id) return;
  const known = state.activeChatId !== null && chatList.some((chat) => chat.id === state.activeChatId);
  state.activeChatId = known ? state.activeChatId : (chatList[0]?.id ?? null);
  fillChatSelect();
  await loadActiveChatMessages(video, gen);
}

/// Leert den Chat-Tab beim Schliessen der Detailansicht.
export function resetChat() {
  state.activeChatId = null;
  chatList = [];
  state.chatRenderGen += 1;
  chatMessagesEl().textContent = "";
  fillChatSelect();
  updateChatControls(null);
}

function selectChat(chatId: number | null) {
  if (state.activeChatId === chatId) return;
  state.activeChatId = chatId;
  fillChatSelect();
  const video = getActiveVideo();
  if (!video) return;
  const gen = ++state.chatRenderGen;
  updateChatControls(video);
  void loadActiveChatMessages(video, gen);
}

function startNewChat() {
  if (activeRun(getActiveVideo())) return;
  selectChat(null);
  setStatus("Neuer Chat");
}

async function deleteChat() {
  const video = getActiveVideo();
  const chatId = state.activeChatId;
  if (!video || chatId === null) return;
  const chat = chatList.find((item) => item.id === chatId);
  if (
    !(await confirmDialog(
      chat ? `Chat "${chat.title}" wirklich löschen?` : "Diesen Chat wirklich löschen?",
      { title: "Chat löschen", okLabel: "Löschen" },
    ))
  ) {
    return;
  }
  try {
    await invoke<void>("chat_delete", { chatId });
  } catch (error) {
    setStatus(errorMessage(error));
    return;
  }
  if (getActiveVideo()?.id !== video.id) return;
  state.activeChatId = null;
  const gen = ++state.chatRenderGen;
  await refreshChatList(video.id, gen);
  if (gen !== state.chatRenderGen || getActiveVideo()?.id !== video.id) return;
  state.activeChatId = chatList[0]?.id ?? null;
  fillChatSelect();
  await loadActiveChatMessages(video, gen);
  setStatus("Chat gelöscht");
}

function resizeChatInput() {
  const input = $<HTMLTextAreaElement>("#chatInput");
  const styles = window.getComputedStyle(input);
  const lineHeight = Number.parseFloat(styles.lineHeight) || 18;
  const extra = input.offsetHeight - input.clientHeight;
  input.style.height = "auto";
  const max = lineHeight * INPUT_MAX_LINES + extra;
  input.style.height = `${Math.min(input.scrollHeight + extra, max)}px`;
}

function restoreQuestion(text: string) {
  const input = $<HTMLTextAreaElement>("#chatInput");
  input.value = text;
  resizeChatInput();
}

async function sendChatMessage() {
  const video = getActiveVideo();
  if (!video) return;
  const input = $<HTMLTextAreaElement>("#chatInput");
  const text = input.value.trim();
  if (!text) return;
  // Synchroner In-flight-Guard: ein zweites Enter darf nichts senden.
  if (state.chatRuns.has(video.id)) return;

  const requestId = crypto.randomUUID();
  const run: ChatRun = { requestId, chatId: state.activeChatId, question: text, answer: "" };
  state.chatRuns.set(video.id, run);
  input.value = "";
  resizeChatInput();
  updateChatControls(video);
  appendProvisionalMessages(run);
  const [providerId, modelId] = parseModelValue($<HTMLSelectElement>("#chatModel").value);
  setStatus("Chat-Antwort läuft…");

  const videoId = video.id;
  const matchContext = (chatId: number | null) =>
    getActiveVideo()?.id === videoId && state.activeChatId === chatId;
  try {
    const result = await invoke<ChatTurnResult>("chat_send", {
      videoId,
      chatId: run.chatId,
      text,
      providerId,
      modelId,
      requestId,
    });
    state.chatRuns.delete(videoId);
    const gen = ++state.chatRenderGen;
    // Der Chat-Verlauf des Videos wird auch dann nachgeladen, wenn der
    // Benutzer inzwischen einen anderen Chat ansieht - die Nachrichten selbst
    // uebernimmt aber nur der passende Kontext ins DOM.
    if (getActiveVideo()?.id === videoId) {
      await refreshChatList(videoId, gen);
    }
    if (gen === state.chatRenderGen && matchContext(run.chatId)) {
      state.activeChatId = result.chat.id;
      fillChatSelect();
      await renderChatMessages(result.messages, gen);
    }
    setStatus("Chat-Antwort fertig");
  } catch (error) {
    state.chatRuns.delete(videoId);
    if (getActiveVideo()?.id === videoId) {
      removeProvisionalMessages(requestId);
      restoreQuestion(text);
    }
    setStatus(errorMessage(error));
  } finally {
    updateChatControls(getActiveVideo());
  }
}

async function cancelChatRun() {
  const run = activeRun(getActiveVideo());
  if (!run) return;
  try {
    await invoke<void>("chat_cancel", { requestId: run.requestId });
  } catch (error) {
    setStatus(errorMessage(error));
  }
}

function onChatStream(event: {
  payload: { requestId: string; videoId: number; text: string };
}) {
  const { requestId, videoId, text } = event.payload;
  const run = state.chatRuns.get(videoId);
  // Fremde requestIds (z. B. nach einem Neustart der Anfrage) duerfen die
  // angezeigte Blase nicht veraendern.
  if (!run || run.requestId !== requestId) return;
  run.answer = text;
  if (getActiveVideo()?.id !== videoId || state.activeChatId !== run.chatId) return;
  void renderStreamingAnswer(run, false, state.chatRenderGen);
  scrollChatToBottom();
}

function bindChatMessagesEvents() {
  chatMessagesEl().addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const link = target.closest<HTMLElement>("[data-seek]");
    if (!link) return;
    event.preventDefault();
    const seconds = Number(link.dataset.seek);
    if (!Number.isNaN(seconds)) {
      seekVideo(seconds);
    }
  });
}

export function bindChatEvents() {
  listen<{ requestId: string; videoId: number; text: string }>(STREAM_EVENT, onChatStream);

  $("#chatSend").addEventListener("click", () => {
    if (activeRun(getActiveVideo())) {
      void cancelChatRun();
    } else {
      void sendChatMessage();
    }
  });
  $<HTMLTextAreaElement>("#chatInput").addEventListener("keydown", (event) => {
    if (!(event instanceof KeyboardEvent)) return;
    if (event.key !== "Enter" || event.shiftKey) return;
    event.preventDefault();
    void sendChatMessage();
  });
  $("#chatInput").addEventListener("input", resizeChatInput);
  $("#chatNew").addEventListener("click", () => startNewChat());
  $("#chatDelete").addEventListener("click", () => void deleteChat());
  $<HTMLSelectElement>("#chatSelect").addEventListener("change", (event) => {
    const target = event.target;
    if (!(target instanceof HTMLSelectElement)) return;
    const value = Number(target.value);
    selectChat(target.value === "" || Number.isNaN(value) ? null : value);
  });
  $<HTMLSelectElement>("#chatModel").addEventListener("change", (event) => {
    const target = event.target;
    if (target instanceof HTMLSelectElement) {
      localStorage.setItem(CHAT_MODEL_KEY, target.value);
    }
  });
  $('.tab[data-tab="chat"]').addEventListener("click", () => {
    const video = getActiveVideo();
    if (video) void renderChatTab(video);
  });
  bindChatMessagesEvents();
  void initChatModelPicker();
}

async function initChatModelPicker() {
  try {
    await ensureAiData();
  } catch (error) {
    setStatus(errorMessage(error));
    return;
  }
  fillModelPicker($<HTMLSelectElement>("#chatModel"), localStorage.getItem(CHAT_MODEL_KEY));
}
