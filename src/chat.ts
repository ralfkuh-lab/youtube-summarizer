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
let lastChatVideoId: number | null = null;

function chatContextKey(videoId: number, chatId: number | null): string {
  return `${videoId}:${chatId ?? "new"}`;
}

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
  // Waehrend einer laufenden Anfrage bleibt nur der Stopp-Button bedienbar.
  input.disabled = !hasTranscript || !!run;
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
  // Eigener Klassenname: `.chat-message` gehoert dem Modell-Testchat in den
  // Einstellungen und stylt dort jeden Absatz als eigenen Kasten.
  row.className = `chat-row chat-row--${role}`;
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
    .querySelectorAll<HTMLElement>(`.chat-row[data-request-id="${requestId}"]`)
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

/// Uebernimmt die gemerkte Chat-Wahl eines Videos, wenn sie noch gueltig ist.
/// Reihenfolge: laufende Anfrage, gemerkte Wahl, sonst der neueste Chat.
function resolveChatSelection(videoId: number): number | null {
  const run = state.chatRuns.get(videoId);
  if (run) return run.chatId;
  if (state.chatSelection.has(videoId)) {
    const remembered = state.chatSelection.get(videoId) ?? null;
    if (remembered === null || chatList.some((chat) => chat.id === remembered)) {
      return remembered;
    }
  }
  return chatList[0]?.id ?? null;
}

/// Baut den Chat-Tab fuer ein Video neu auf (auch beim Aktivieren des Tabs).
export async function renderChatTab(video: Video) {
  const gen = ++state.chatRenderGen;
  if (lastChatVideoId !== video.id) {
    // Beim Videowechsel gehoert der getippte Text noch zum verlassenen Chat.
    stashChatDraft(lastChatVideoId, state.activeChatId);
    lastChatVideoId = video.id;
  }
  chatList = [];
  fillChatSelect();
  updateChatControls(video);
  await refreshChatList(video.id, gen);
  if (gen !== state.chatRenderGen || getActiveVideo()?.id !== video.id) return;
  state.activeChatId = resolveChatSelection(video.id);
  fillChatSelect();
  restoreChatDraft(video.id, state.activeChatId);
  await loadActiveChatMessages(video, gen);
}

/// Leert den Chat-Tab beim Schliessen der Detailansicht.
export function resetChat() {
  state.activeChatId = null;
  chatList = [];
  lastChatVideoId = null;
  state.chatRenderGen += 1;
  chatMessagesEl().textContent = "";
  clearChatInput();
  fillChatSelect();
  updateChatControls(null);
}

function selectChat(chatId: number | null) {
  const video = getActiveVideo();
  // Jede explizite Wahl wird gemerkt, auch wenn sie schon aktiv ist.
  if (video) state.chatSelection.set(video.id, chatId);
  if (state.activeChatId === chatId) return;
  stashChatDraft(video?.id ?? null, state.activeChatId);
  state.activeChatId = chatId;
  fillChatSelect();
  if (!video) return;
  restoreChatDraft(video.id, chatId);
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
  state.chatSelection.delete(video.id);
  state.chatDrafts.delete(chatContextKey(video.id, chatId));
  const gen = ++state.chatRenderGen;
  await refreshChatList(video.id, gen);
  if (gen !== state.chatRenderGen || getActiveVideo()?.id !== video.id) return;
  state.activeChatId = resolveChatSelection(video.id);
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

function clearChatInput() {
  const input = $<HTMLTextAreaElement>("#chatInput");
  input.value = "";
  resizeChatInput();
}

/// Ein Entwurf wird beim Anzeigen eines Chats eingesetzt und bleibt erhalten;
/// geloescht wird er beim erfolgreichen Senden aus diesem Chat oder wenn der
/// Benutzer das Feld leert und den Chat verlaesst.
function restoreChatDraft(videoId: number, chatId: number | null) {
  const draft = state.chatDrafts.get(chatContextKey(videoId, chatId));
  if (draft === undefined) return;
  const input = $<HTMLTextAreaElement>("#chatInput");
  input.value = draft;
  resizeChatInput();
}

/// Merkt den nichtleeren Inhalt des Eingabefelds beim Verlassen eines Chats
/// und leert das Feld; ein leeres Feld entfernt den Entwurf dieses Chats.
function stashChatDraft(videoId: number | null, chatId: number | null) {
  if (videoId === null) return;
  const input = $<HTMLTextAreaElement>("#chatInput");
  const value = input.value;
  const key = chatContextKey(videoId, chatId);
  if (value.trim() === "") {
    state.chatDrafts.delete(key);
  } else {
    state.chatDrafts.set(key, value);
  }
  input.value = "";
  resizeChatInput();
}

/// Vergisst Auswahl und Entwuerfe eines geloeschten Videos.
export function forgetChatState(videoId: number) {
  state.chatSelection.delete(videoId);
  const prefix = `${videoId}:`;
  for (const key of [...state.chatDrafts.keys()]) {
    if (key.startsWith(prefix)) state.chatDrafts.delete(key);
  }
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
  const runChatId = run.chatId;
  const isActiveVideo = () => getActiveVideo()?.id === videoId;
  const isVisibleContext = () => isActiveVideo() && state.activeChatId === runChatId;
  let focusInput = false;
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
    // N3: Die Auswahl folgt dem fertig gewordenen Chat, solange der Benutzer
    // nicht inzwischen explizit einen anderen Chat gewaehlt hat.
    const remembered = state.chatSelection.has(videoId)
      ? (state.chatSelection.get(videoId) ?? null)
      : runChatId;
    if (remembered === runChatId) {
      state.chatSelection.set(videoId, result.chat.id);
    }
    // Der Text ist abgeschickt; fuer diesen Chat gibt es keinen Entwurf mehr.
    state.chatDrafts.delete(chatContextKey(videoId, runChatId));
    if (isVisibleContext()) {
      // Frische Generation: der Sendezeitpunkt kann lange zurueckliegen.
      const gen = ++state.chatRenderGen;
      state.activeChatId = result.chat.id;
      await refreshChatList(videoId, gen);
      if (gen === state.chatRenderGen) {
        fillChatSelect();
        await renderChatMessages(result.messages, gen);
      }
      focusInput = true;
    } else if (isActiveVideo()) {
      // Gleiches Video, aber ein anderer Chat sichtbar: nur die Liste
      // nachziehen, den sichtbaren Verlauf nicht anfassen.
      await refreshChatList(videoId, state.chatRenderGen);
    }
    if (isActiveVideo()) setStatus("Chat-Antwort fertig");
  } catch (error) {
    state.chatRuns.delete(videoId);
    removeProvisionalMessages(requestId);
    if (isVisibleContext()) {
      restoreQuestion(text);
      focusInput = true;
    } else {
      // Die Frage gehoert in ihren Chat; ein vorhandener Entwurf bleibt erhalten.
      const key = chatContextKey(videoId, runChatId);
      const existing = state.chatDrafts.get(key);
      state.chatDrafts.set(key, existing ? `${text}\n\n${existing}` : text);
    }
    const message = errorMessage(error);
    setStatus(isActiveVideo() ? message : `Chat zu „${video.title}“: ${message}`);
  } finally {
    updateChatControls(getActiveVideo());
    if (focusInput) $<HTMLTextAreaElement>("#chatInput").focus();
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
  void listen<{ requestId: string; videoId: number; text: string }>(STREAM_EVENT, onChatStream).catch(
    (error) => console.error("ai:chat_stream konnte nicht abonniert werden", error),
  );

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
