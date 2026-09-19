// Chat-Zustand ohne DOM: Auswahl je Video, Entwuerfe, Laufregister-Zugriffe.
// Die Oberflaeche (chat.ts) liest und schreibt ausschliesslich ueber diese
// Funktionen, damit die Regeln testbar und an einer Stelle dokumentiert sind.

import { state } from "./state";
import type { Chat } from "./types";

let chatList: Chat[] = [];

export function chatContextKey(videoId: number, chatId: number | null): string {
  return `${videoId}:${chatId ?? "new"}`;
}

export function setChatList(chats: Chat[]) {
  chatList = chats;
}

export function getChatList(): Chat[] {
  return chatList;
}

/// Merkt die explizite Chat-Wahl eines Videos (`null` = „Neuer Chat“).
export function rememberChatSelection(videoId: number, chatId: number | null) {
  state.chatSelection.set(videoId, chatId);
}

export function forgetChatSelection(videoId: number) {
  state.chatSelection.delete(videoId);
}

/// Uebernimmt die gemerkte Chat-Wahl eines Videos, wenn sie noch gueltig ist.
/// Reihenfolge: laufende Anfrage, gemerkte Wahl, sonst der neueste Chat.
export function resolveChatSelection(videoId: number): number | null {
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

/// Merkt den Inhalt des Eingabefelds beim Verlassen eines Chats. Ein leerer
/// Inhalt entfernt den Entwurf dieses Chats.
export function stashChatDraft(videoId: number, chatId: number | null, value: string) {
  const key = chatContextKey(videoId, chatId);
  if (value.trim() === "") {
    state.chatDrafts.delete(key);
  } else {
    state.chatDrafts.set(key, value);
  }
}

export function readChatDraft(videoId: number, chatId: number | null): string | undefined {
  return state.chatDrafts.get(chatContextKey(videoId, chatId));
}

export function deleteChatDraft(videoId: number, chatId: number | null) {
  state.chatDrafts.delete(chatContextKey(videoId, chatId));
}

/// Eine fehlgeschlagene Frage in unsichtbarem Kontext wird vorangestellt, damit
/// ein vorhandener Entwurf des Chats nicht ueberschrieben wird.
export function prependChatDraft(videoId: number, chatId: number | null, question: string) {
  const key = chatContextKey(videoId, chatId);
  const existing = state.chatDrafts.get(key);
  state.chatDrafts.set(key, existing ? `${question}\n\n${existing}` : question);
}

/// Vergisst Auswahl und Entwuerfe eines geloeschten Videos.
export function forgetChatState(videoId: number) {
  forgetChatSelection(videoId);
  const prefix = `${videoId}:`;
  for (const key of [...state.chatDrafts.keys()]) {
    if (key.startsWith(prefix)) state.chatDrafts.delete(key);
  }
}
