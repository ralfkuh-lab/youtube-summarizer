import { invoke } from "@tauri-apps/api/core";
import { clearDetail, renderVideoCollections, showDetail } from "./detail";
import { $, confirmDialog, errorMessage, escapeHtml, hideModal, showModal } from "./dom-utils";
import { getActiveVideo, setBusy, setStatus, state } from "./state";
import type { Collection, Video } from "./types";
import { compareCollections, normalizeSearch } from "./utils";

let videoList: HTMLDivElement;
let collectionList: HTMLDivElement;
let editingCollectionId: number | null = null;

export function initLibrary() {
  videoList = $<HTMLDivElement>("#videoList");
  collectionList = $<HTMLDivElement>("#collectionList");
}

export function renderVideoFilters() {
  document.querySelectorAll<HTMLButtonElement>(".filter-chip").forEach((button) => {
    button.classList.toggle("active", button.dataset.videoFilter === state.videoStatusFilter);
  });
}

function matchesActiveCollection(video: Video): boolean {
  return state.activeCollectionId === null || video.collection_ids.includes(state.activeCollectionId);
}

function matchesVideoStatusFilter(video: Video): boolean {
  switch (state.videoStatusFilter) {
    case "transcript":
      return video.has_transcript;
    case "missing-transcript":
      return !video.has_transcript;
    case "summary":
      return video.has_summary;
    case "missing-summary":
      return !video.has_summary;
    case "all":
      return true;
  }
}

function matchesVideoSearch(video: Video, normalizedQuery: string): boolean {
  if (!normalizedQuery) return true;
  return [video.title, video.url, video.video_id, video.published_at]
    .filter((value): value is string => !!value)
    .some((value) => normalizeSearch(value).includes(normalizedQuery));
}

function getFilteredVideos(): Video[] {
  const normalizedQuery = normalizeSearch(state.videoSearchQuery);
  return state.videos.filter(
    (video) =>
      matchesActiveCollection(video) &&
      matchesVideoStatusFilter(video) &&
      matchesVideoSearch(video, normalizedQuery),
  );
}

export function renderVideoStatusChip(
  label: string,
  available: boolean,
  title: string,
  detail?: string | null,
): string {
  const stateClass = available ? " available" : "";
  const titleText =
    !available && detail ? `${title} fehlt: ${detail}` : `${title} ${available ? "vorhanden" : "fehlt"}`;
  return `<span class="status-chip${stateClass}" title="${escapeHtml(titleText)}">${label}</span>`;
}

export function renderVideoList() {
  if (!state.videos.length) {
    videoList.innerHTML = '<p class="empty-list">Noch keine Videos</p>';
    return;
  }

  const filteredVideos = getFilteredVideos();
  if (!filteredVideos.length) {
    videoList.innerHTML = '<p class="empty-list">Keine passenden Videos</p>';
    return;
  }

  videoList.innerHTML = filteredVideos
    .map((video) => {
      const activeClass = state.activeVideoId === video.id ? " active" : "";
      const thumb = video.thumbnail || video.thumbnail_url;
      return `
        <button class="video-item${activeClass}" data-id="${video.id}">
          <img src="${escapeHtml(thumb)}" alt="" loading="lazy" />
          <span class="info">
            <span class="title">${escapeHtml(video.title)}</span>
            <span class="meta">
              ${renderVideoStatusChip("T", video.has_transcript, "Transkript", video.transcript_error)}
              ${renderVideoStatusChip("Z", video.has_summary, "Zusammenfassung")}
            </span>
          </span>
        </button>
      `;
    })
    .join("");

  videoList.querySelectorAll<HTMLButtonElement>(".video-item").forEach((item) => {
    item.addEventListener("click", () => {
      const id = Number(item.dataset.id);
      if (!Number.isNaN(id)) {
        void selectVideo(id);
      }
    });
  });
}

export async function selectVideo(id: number) {
  state.activeVideoId = id;
  renderVideoList();
  try {
    const video = await invoke<Video>("get_video_detail", { id });
    state.videos = state.videos.map((item) => (item.id === id ? video : item));
    renderVideoList();
    if (state.activeVideoId === id) {
      showDetail(video);
    }
  } catch (error) {
    if (state.activeVideoId === id) {
      setStatus(errorMessage(error));
    }
  }
}

export async function addVideo() {
  if (state.busy) return;
  const input = $<HTMLInputElement>("#urlInput");
  const url = input.value.trim();
  if (!url) return;

  setBusy(true, "Video wird hinzugefügt...");
  try {
    const video = await invoke<Video>("add_video", { url });
    state.videos = [video, ...state.videos.filter((item) => item.id !== video.id)];
    input.value = "";
    renderVideoList();
    await selectVideo(video.id);
    if (video.has_transcript) {
      setStatus("Video hinzugefügt und Transkript geladen");
    } else if (video.transcript_error) {
      setStatus(`Video hinzugefügt, Transkript fehlgeschlagen: ${video.transcript_error}`);
    } else {
      setStatus("Video hinzugefügt, aber kein Transkript gefunden");
    }
  } catch (error) {
    setStatus(errorMessage(error));
  } finally {
    setBusy(false);
  }
}

export async function deleteActiveVideo() {
  if (state.activeVideoId === null || state.busy) return;
  if (
    !(await confirmDialog("Video wirklich löschen?", {
      title: "Video löschen",
      okLabel: "Löschen",
    }))
  ) {
    return;
  }
  const id = state.activeVideoId;
  setBusy(true, "Video wird gelöscht...");
  try {
    await invoke<void>("delete_video", { id });
    state.videos = state.videos.filter((video) => video.id !== id);
    await loadCollections();
    renderVideoList();
    if (state.activeVideoId === id) {
      state.activeVideoId = null;
      clearDetail();
    }
    setStatus("Video gelöscht");
  } catch (error) {
    if (state.activeVideoId === id) {
      setStatus(errorMessage(error));
    }
  } finally {
    setBusy(false);
  }
}

export function renderCollectionItem(collection: Collection): string {
  const activeClass = state.activeCollectionId === collection.id ? " active" : "";
  return `
    <div class="collection-row${activeClass}">
      <button class="collection-item" data-collection-id="${collection.id}">
        <span class="collection-name">${escapeHtml(collection.name)}</span>
        <span class="collection-count">${collection.video_count}</span>
      </button>
      <div class="collection-actions">
        <button class="collection-action" data-action="rename" data-collection-id="${collection.id}" title="Sammlung umbenennen" aria-label="Sammlung umbenennen">✎</button>
        <button class="collection-action danger" data-action="delete" data-collection-id="${collection.id}" title="Sammlung löschen" aria-label="Sammlung löschen">×</button>
      </div>
    </div>
  `;
}

export function renderCollectionList() {
  collectionList.innerHTML = `
    <button class="collection-item${state.activeCollectionId === null ? " active" : ""}" data-collection-id="all">
      <span class="collection-name">Alle Videos</span>
      <span class="collection-count">${state.videos.length}</span>
    </button>
    ${
      state.collections.length
        ? state.collections.map(renderCollectionItem).join("")
        : '<p class="empty-list compact">Noch keine Sammlungen</p>'
    }
  `;

  collectionList.querySelectorAll<HTMLButtonElement>(".collection-item").forEach((button) => {
    button.addEventListener("click", () => {
      const id = button.dataset.collectionId;
      state.activeCollectionId = id === "all" ? null : Number(id);
      if (Number.isNaN(state.activeCollectionId)) state.activeCollectionId = null;
      renderCollectionList();
      renderVideoList();
    });
  });

  collectionList.querySelectorAll<HTMLButtonElement>(".collection-action").forEach((button) => {
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      const id = Number(button.dataset.collectionId);
      const collection = state.collections.find((item) => item.id === id);
      if (!collection) return;
      if (button.dataset.action === "rename") {
        openCollectionDialog(collection);
      } else if (button.dataset.action === "delete") {
        void deleteCollection(collection);
      }
    });
  });
}

export function openCollectionDialog(collection?: Collection) {
  editingCollectionId = collection?.id ?? null;
  $("#collectionModalTitle").textContent = collection ? "Sammlung umbenennen" : "Sammlung erstellen";
  const input = $<HTMLInputElement>("#collectionNameInput");
  input.value = collection?.name ?? "";
  showModal("#collectionModal");
  queueMicrotask(() => {
    input.focus();
    input.select();
  });
}

export async function saveCollection() {
  if (state.busy) return;
  const input = $<HTMLInputElement>("#collectionNameInput");
  const name = input.value.trim();
  if (!name) {
    setStatus("Sammlungsname darf nicht leer sein");
    return;
  }

  setBusy(
    true,
    editingCollectionId === null ? "Sammlung wird erstellt..." : "Sammlung wird umbenannt...",
  );
  try {
    if (editingCollectionId === null) {
      const collection = await invoke<Collection>("create_collection", { name });
      state.collections = [...state.collections, collection].sort(compareCollections);
      state.activeCollectionId = collection.id;
    } else {
      const updated = await invoke<Collection>("update_collection", {
        id: editingCollectionId,
        name,
      });
      state.collections = state.collections
        .map((collection) => (collection.id === updated.id ? updated : collection))
        .sort(compareCollections);
    }
    hideModal("#collectionModal");
    renderCollectionList();
    renderVideoList();
    setStatus("Sammlung gespeichert");
  } catch (error) {
    setStatus(errorMessage(error));
  } finally {
    setBusy(false);
  }
}

export async function deleteCollection(collection: Collection) {
  if (
    !(await confirmDialog(`Sammlung "${collection.name}" löschen? Die Videos bleiben erhalten.`, {
      title: "Sammlung löschen",
      okLabel: "Löschen",
    }))
  ) {
    return;
  }

  setBusy(true, "Sammlung wird gelöscht...");
  try {
    await invoke<void>("delete_collection", { id: collection.id });
    state.collections = state.collections.filter((item) => item.id !== collection.id);
    state.videos = state.videos.map((video) => ({
      ...video,
      collection_ids: video.collection_ids.filter((id) => id !== collection.id),
    }));
    if (state.activeCollectionId === collection.id) state.activeCollectionId = null;
    renderCollectionList();
    renderVideoList();
    const active = getActiveVideo();
    if (active) renderVideoCollections(active);
    setStatus("Sammlung gelöscht");
  } catch (error) {
    setStatus(errorMessage(error));
  } finally {
    setBusy(false);
  }
}

export async function loadCollections() {
  state.collections = await invoke<Collection[]>("get_collections");
  renderCollectionList();
}

export function bindLibraryEvents() {
  $("#addCollectionBtn").addEventListener("click", () => openCollectionDialog());
  $("#collectionSave").addEventListener("click", () => void saveCollection());
  $("#collectionCancel").addEventListener("click", () => hideModal("#collectionModal"));
  $("#collectionNameInput").addEventListener("keydown", (event) => {
    if (!(event instanceof KeyboardEvent)) return;
    if (event.key === "Enter") void saveCollection();
  });
  $("#deleteBtn").addEventListener("click", () => void deleteActiveVideo());
}
