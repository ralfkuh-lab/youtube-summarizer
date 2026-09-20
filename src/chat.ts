import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ensureAiData, fillModelPicker, modelSupportsToolCall } from "./ai-config";
import {
  deleteChatDraft,
  forgetChatSelection,
  forgetChatState,
  getChatList,
  prependChatDraft,
  readChatDraft,
  rememberChatSelection,
  resolveChatSelection,
  setChatList,
  stashChatDraft,
} from "./chat-state";
import { seekVideo } from "./detail";
import { $, confirmDialog, errorMessage } from "./dom-utils";
import { formatShortDateTime } from "./utils";
import { getActiveVideo, setStatus, state, type ChatRun } from "./state";
import { applyStreamEvent, applyToolEvent, createChatRun } from "./chat-live";
import {
  appendProvisionalMessages,
  clearChatMessages,
  removeProvisionalMessages,
  renderChatMessages,
  renderLiveRun,
  scrollChatToBottom,
} from "./chat-render";
import type { Chat, ChatMessageRecord, ChatTurnResult, SummaryRecord, Video } from "./types";
import {
  applyChatContextFor,
  bindChatContextEvents,
  closeChatContextMenu,
  getChatContextOptions,
  isChatContextValid,
  refreshChatContextButton,
  setChatContextAvailability,
  setChatContextVersions,
} from "./chat-context";
import {
  isWebSearchConfigured,
  loadWebSearchConfig,
  onWebSearchConfigChanged,
} from "./websearch-settings";

export { forgetChatState };

const CHAT_MODEL_KEY = "chatModel";
const CHAT_WEB_SEARCH_KEY = "chatWebSearch";
const STREAM_EVENT = "ai:chat_stream";
const TOOL_EVENT = "ai:chat_tool";
const INPUT_MAX_LINES = 6;

let lastChatVideoId: number | null = null;

function chatList(): Chat[] {
  return getChatList();
}

/// Der Schalter ist nur bedienbar, wenn die Websuche konfiguriert ist und das
/// gewaehlte Modell Tool-Calling unterstuetzt.
export function updateWebSearchToggle() {
  const checkbox = $<HTMLInputElement>("#chatWebSearch");
  const label = $<HTMLLabelElement>("#chatWebSearchLabel");
  const configured = isWebSearchConfigured();
  const modelSupports = modelSupportsToolCall($<HTMLSelectElement>("#chatModel").value);
  const available = configured && modelSupports;
  checkbox.disabled = !available;
  checkbox.checked = available && localStorage.getItem(CHAT_WEB_SEARCH_KEY) === "1";
  label.title = available
    ? "Websuche fuer diese Frage nutzen"
    : configured
      ? "Modell unterstützt kein Tool-Calling"
      : "Websuche ist nicht konfiguriert";
}

function webSearchRequested(): boolean {
  const checkbox = $<HTMLInputElement>("#chatWebSearch");
  return !checkbox.disabled && checkbox.checked;
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
  const hasSummary = !!video?.has_summary;
  const run = activeRun(video);
  // Etappe 3: Eingabe ist auch ohne Transkript nutzbar, wenn eine
  // Zusammenfassung existiert (dann nur mit Zusammenfassung im Kontext).
  const usable = hasTranscript || hasSummary;
  const sendBtn = $<HTMLButtonElement>("#chatSend");
  sendBtn.textContent = run ? "Stopp" : "Senden";
  sendBtn.disabled = !usable;
  const input = $<HTMLTextAreaElement>("#chatInput");
  // Waehrend einer laufenden Anfrage bleibt nur der Stopp-Button bedienbar.
  input.disabled = !usable || !!run;
  input.placeholder = usable
    ? "Frage zum Video…"
    : "Für den Chat wird ein Transkript oder eine Zusammenfassung benötigt";
  $("#chatHint").hidden = usable;
  $<HTMLButtonElement>("#chatNew").disabled = !!run || !usable;
  $<HTMLButtonElement>("#chatDelete").disabled =
    !!run || state.activeChatId === null || !usable;
  $<HTMLSelectElement>("#chatModel").disabled = !!run;
  setChatContextAvailability(hasTranscript, hasSummary);
  refreshChatContextButton();
}

function fillChatSelect() {
  const select = $<HTMLSelectElement>("#chatSelect");
  select.textContent = "";
  const empty = document.createElement("option");
  empty.value = "";
  empty.textContent = chatList().length ? "Neuer Chat" : "Noch kein Chat";
  select.append(empty);
  for (const chat of chatList()) {
    const option = document.createElement("option");
    option.value = String(chat.id);
    option.textContent = `${chat.title} – ${formatShortDateTime(chat.updatedAt)}`;
    select.append(option);
  }
  select.value = state.activeChatId === null ? "" : String(state.activeChatId);
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
  setChatList(chats);
  fillChatSelect();
}

async function loadChatContext(video: Video, chatId: number | null) {
  let summaries: SummaryRecord[] = [];
  try {
    summaries = await invoke<SummaryRecord[]>("get_summaries", { videoId: video.id });
  } catch (error) {
    setStatus(errorMessage(error));
  }
  const stored =
    chatId === null ? null : (chatList().find((chat) => chat.id === chatId)?.contextOptions ?? null);
  setChatContextVersions(summaries);
  applyChatContextFor(chatId, stored);
}

async function loadActiveChatMessages(video: Video, gen: number) {
  const chatId = state.activeChatId;
  await loadChatContext(video, chatId);
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
    await renderLiveRun(run, gen);
  }
  updateChatControls(video);
}

/// Baut den Chat-Tab fuer ein Video neu auf (auch beim Aktivieren des Tabs).
export async function renderChatTab(video: Video) {
  closeChatContextMenu();
  const gen = ++state.chatRenderGen;
  if (lastChatVideoId !== video.id) {
    // Beim Videowechsel gehoert der getippte Text noch zum verlassenen Chat.
    stashInputDraft(lastChatVideoId, state.activeChatId);
    lastChatVideoId = video.id;
  }
  setChatList([]);
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
  closeChatContextMenu();
  state.activeChatId = null;
  setChatList([]);
  lastChatVideoId = null;
  state.chatRenderGen += 1;
  clearChatMessages();
  clearChatInput();
  fillChatSelect();
  updateChatControls(null);
}

function selectChat(chatId: number | null) {
  const video = getActiveVideo();
  // Jede explizite Wahl wird gemerkt, auch wenn sie schon aktiv ist.
  if (video) rememberChatSelection(video.id, chatId);
  closeChatContextMenu();
  if (state.activeChatId === chatId) return;
  stashInputDraft(video?.id ?? null, state.activeChatId);
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
  closeChatContextMenu();
  selectChat(null);
  setStatus("Neuer Chat");
}

async function deleteChat() {
  const video = getActiveVideo();
  const chatId = state.activeChatId;
  if (!video || chatId === null) return;
  const chat = chatList().find((item) => item.id === chatId);
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
  forgetChatSelection(video.id);
  deleteChatDraft(video.id, chatId);
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
  const draft = readChatDraft(videoId, chatId);
  if (draft === undefined) return;
  const input = $<HTMLTextAreaElement>("#chatInput");
  input.value = draft;
  resizeChatInput();
}

/// Merkt den Inhalt des Eingabefelds beim Verlassen eines Chats und leert es.
function stashInputDraft(videoId: number | null, chatId: number | null) {
  if (videoId === null) return;
  const input = $<HTMLTextAreaElement>("#chatInput");
  stashChatDraft(videoId, chatId, input.value);
  input.value = "";
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
  // Dieselbe Bedingung wie der Senden-Button: Enter ist keine Hintertuer.
  if (!isChatContextValid()) return;
  closeChatContextMenu();

  const requestId = crypto.randomUUID();
  const run: ChatRun = createChatRun({ requestId, chatId: state.activeChatId, question: text });
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
      webSearch: webSearchRequested(),
      contextOptions: getChatContextOptions(),
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
    deleteChatDraft(videoId, runChatId);
    if (isVisibleContext()) {
      // Frische Generation: der Sendezeitpunkt kann lange zurueckliegen.
      const gen = ++state.chatRenderGen;
      state.activeChatId = result.chat.id;
      await refreshChatList(videoId, gen);
      applyChatContextFor(result.chat.id, result.chat.contextOptions);
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
      prependChatDraft(videoId, runChatId, text);
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
  payload: {
    requestId: string;
    videoId: number;
    round: number;
    text: string;
    final: boolean;
    discarded?: boolean;
  };
}) {
  const run = state.chatRuns.get(event.payload.videoId);
  // Fremde requestIds (z. B. nach einem Neustart der Anfrage) duerfen die
  // angezeigte Blase nicht veraendern.
  if (!run || run.requestId !== event.payload.requestId) return;
  applyStreamEvent(run, event.payload);
  if (getActiveVideo()?.id !== event.payload.videoId || state.activeChatId !== run.chatId) return;
  void renderLiveRun(run, state.chatRenderGen);
  scrollChatToBottom();
}

function onChatTool(event: {
  payload: {
    requestId: string;
    videoId: number;
    round: number;
    kind: string;
    label: string;
    status: string;
  };
}) {
  const run = state.chatRuns.get(event.payload.videoId);
  // Fremde requestIds duerfen die angezeigte Blase nicht veraendern.
  if (!run || run.requestId !== event.payload.requestId) return;
  applyToolEvent(run, event.payload);
  if (getActiveVideo()?.id !== event.payload.videoId || state.activeChatId !== run.chatId) return;
  void renderLiveRun(run, state.chatRenderGen);
  scrollChatToBottom();
}

function bindChatMessagesEvents() {
  $<HTMLElement>("#chatMessages").addEventListener("click", (event) => {
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
  void listen<{
    requestId: string;
    videoId: number;
    round: number;
    text: string;
    final: boolean;
    discarded?: boolean;
  }>(STREAM_EVENT, onChatStream).catch(
    (error) => console.error("ai:chat_stream konnte nicht abonniert werden", error),
  );
  void listen<{
    requestId: string;
    videoId: number;
    round: number;
    kind: string;
    label: string;
    status: string;
  }>(TOOL_EVENT, onChatTool).catch((error) =>
    console.error("ai:chat_tool konnte nicht abonniert werden", error),
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
    updateWebSearchToggle();
  });
  $<HTMLInputElement>("#chatWebSearch").addEventListener("change", (event) => {
    const target = event.target;
    if (target instanceof HTMLInputElement) {
      localStorage.setItem(CHAT_WEB_SEARCH_KEY, target.checked ? "1" : "0");
    }
  });
  onWebSearchConfigChanged(() => updateWebSearchToggle());
  $('.tab[data-tab="chat"]').addEventListener("click", () => {
    const video = getActiveVideo();
    if (video) void renderChatTab(video);
  });
  bindChatMessagesEvents();
  bindChatContextEvents();
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
  await loadWebSearchConfig();
  updateWebSearchToggle();
}
