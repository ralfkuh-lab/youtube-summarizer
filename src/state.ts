import { $ } from "./dom-utils";
import type { Collection, TabName, Video, VideoStatusFilter } from "./types";

export interface AppState {
  videos: Video[];
  collections: Collection[];
  activeVideoId: number | null;
  activeCollectionId: number | null;
  activeTab: TabName;
  busy: boolean;
  videoSearchQuery: string;
  videoStatusFilter: VideoStatusFilter;
  streamingVideoId: number | null;
  summaryRenderGen: number;
}

export const state: AppState = {
  videos: [],
  collections: [],
  activeVideoId: null,
  activeCollectionId: null,
  activeTab: "transcript",
  busy: false,
  videoSearchQuery: "",
  videoStatusFilter: "all",
  streamingVideoId: null,
  summaryRenderGen: 0,
};

let statusTextEl: HTMLElement | null = null;

export function initState(elements?: { statusEl?: HTMLElement | null }) {
  if (elements?.statusEl !== undefined) {
    statusTextEl = elements.statusEl;
  }
}

export function getActiveVideo(): Video | null {
  return state.videos.find((video) => video.id === state.activeVideoId) ?? null;
}

export function setStatus(message: string) {
  statusTextEl ??= $<HTMLSpanElement>("#statusText");
  statusTextEl.textContent = message;
}

export function setBusy(value: boolean, message?: string) {
  state.busy = value;
  $<HTMLButtonElement>("#addBtn").disabled = value;
  $<HTMLButtonElement>("#summarizeBtn").disabled = value;
  $<HTMLButtonElement>("#reloadTranscriptBtn").disabled = value;
  $<HTMLButtonElement>("#deleteBtn").disabled = value;
  $<HTMLButtonElement>("#summaryHistoryDelete").disabled = value;
  $<HTMLSelectElement>("#summaryHistorySelect").disabled = value;
  document.querySelectorAll<HTMLButtonElement>(".collection-action, #addCollectionBtn, #collectionSave").forEach((button) => {
    button.disabled = value;
  });
  document.querySelectorAll<HTMLInputElement>('#collectionAssignment input[type="checkbox"]').forEach((input) => {
    input.disabled = value;
  });
  if (message) {
    setStatus(message);
  }
}
