import { invoke } from "@tauri-apps/api/core";
import { $, errorMessage, escapeHtml } from "./dom-utils";
import { loadCollections, openCollectionDialog, renderVideoList } from "./library";
import { getActiveVideo, setBusy, setStatus, state } from "./state";
import { renderSummaryTab } from "./summary-view";
import type { Chapter, TabName, TranscriptSnippet, Video } from "./types";
import { capitalize, formatDate } from "./utils";

let detailPlaceholder: HTMLDivElement;
let detailContent: HTMLDivElement;
let chaptersPanel: HTMLDivElement;
let chaptersList: HTMLDivElement;

export function initDetail() {
  detailPlaceholder = $<HTMLDivElement>("#detailPlaceholder");
  detailContent = $<HTMLDivElement>("#detailContent");
  chaptersPanel = $<HTMLDivElement>("#chaptersPanel");
  chaptersList = $<HTMLDivElement>("#chaptersList");
  updateVideoCodecNotice();
}

export function clearDetail() {
  detailContent.hidden = true;
  detailPlaceholder.hidden = false;
  chaptersPanel.hidden = true;
}

// YouTube liefert seine Streams als H.264/MP4 oder VP8/VP9/WebM ueber die
// MediaSource-API aus. WebKitGTK reicht die Dekodierung an das GStreamer des
// Systems weiter; fehlen dort die Plugins, meldet der eingebettete Player nur
// "Your browser can't play this video" und nennt den Grund nicht. Auf Windows
// und macOS stellt sich die Frage nicht - WebView2 und WKWebView bringen ihre
// Decoder mit.
export function hasPlayableVideoCodec(): boolean {
  // Bewusst lokal: die Pruefung laeuft schon beim Verdrahten der Events, eine
  // Modulkonstante waere zu diesem Zeitpunkt noch nicht initialisiert.
  const streamTypes = [
    'video/mp4; codecs="avc1.42E01E"',
    'video/webm; codecs="vp9"',
    'video/webm; codecs="vp8"',
  ];
  const probe = document.createElement("video");
  if (streamTypes.some((type) => probe.canPlayType(type) !== "")) return true;
  const mediaSource = window.MediaSource;
  if (!mediaSource) return false;
  return streamTypes.some((type) => mediaSource.isTypeSupported(type));
}

export function updateVideoCodecNotice() {
  const notice = $("#videoCodecNotice");
  // Nur warnen, wenn wirklich kein einziger Codec gemeldet wird - ein
  // Fehlalarm waere schlimmer als gar kein Hinweis.
  const missing = navigator.userAgent.includes("Linux") && !hasPlayableVideoCodec();
  $("#tabVideo").classList.toggle("has-codec-notice", missing);
  if (!missing) {
    notice.hidden = true;
    return;
  }
  notice.innerHTML = `
    <strong>Video kann hier nicht abgespielt werden</strong>
    <p>Diesem System fehlen die GStreamer-Codecs, mit denen Videos dekodiert
    werden. Über den Link unter dem Player lässt sich das Video weiterhin
    direkt auf YouTube ansehen.</p>
    <p>Abhilfe: Codecs installieren und die Anwendung neu starten.<br>
    <code>sudo pacman -S gst-libav gst-plugins-good gst-plugins-bad</code><br>
    <code>sudo apt install gstreamer1.0-libav gstreamer1.0-plugins-good</code></p>
  `;
  notice.hidden = false;
}

// Escapes the description and turns plain http(s) URLs into clickable links;
// timestamps outside of URLs become video seek links.
export function renderDescriptionHtml(raw: string): string {
  const urlPattern = /https?:\/\/[^\s<>"')\]]+/g;
  let html = "";
  let last = 0;
  for (const match of raw.matchAll(urlPattern)) {
    const url = match[0];
    html += renderDescriptionTextFragment(raw.slice(last, match.index));
    html += `<a href="${escapeHtml(url)}" target="_blank" rel="noreferrer">${escapeHtml(url)}</a>`;
    last = match.index + url.length;
  }
  html += renderDescriptionTextFragment(raw.slice(last));
  return html;
}

// Escapes a URL-free description fragment and links timestamps (1:23, 1:02:03)
// as data-seek jumps into the video tab.
export function renderDescriptionTextFragment(raw: string): string {
  const timestampPattern = /(^|[^\d:])((?:\d{1,2}:)?\d{1,2}:\d{2})(?![\d:])/gm;
  let html = "";
  let last = 0;
  for (const match of raw.matchAll(timestampPattern)) {
    const stamp = match[2];
    const stampIndex = match.index + match[1].length;
    const seconds = stamp.split(":").reduce((total, part) => total * 60 + Number(part), 0);
    html += escapeHtml(raw.slice(last, stampIndex));
    html += `<a href="#" data-seek="${seconds}">${escapeHtml(stamp)}</a>`;
    last = stampIndex + stamp.length;
  }
  html += escapeHtml(raw.slice(last));
  return html;
}

export function transcriptErrorHint(error: string): string | null {
  if (error.includes("LOGIN_REQUIRED")) {
    return 'YouTube verlangt hier eine Anmeldung (Bot-Check). Das passiert typischerweise über bekannte VPN-Ausgangs-IPs, z. B. Mullvad. Abhilfe: die App außerhalb des VPN-Tunnels starten (unter Linux etwa mit mullvad-exclude) und dann „Transkript laden“ klicken.';
  }
  return null;
}

export function renderTranscript(
  raw?: string | null,
  chapters?: Chapter[] | null,
  error?: string | null,
): string {
  if (!raw) {
    if (error) {
      const hint = transcriptErrorHint(error);
      const hintHtml = hint ? `\n    <p class="transcript-error-hint">${escapeHtml(hint)}</p>` : "";
      return `<div class="transcript-error">
    <p class="transcript-error-title">Transkript konnte nicht geladen werden</p>
    <p class="transcript-error-message">${escapeHtml(error)}</p>${hintHtml}
    <p class="transcript-error-retry">Erneut versuchen über „Transkript laden“.</p>
  </div>`;
    }
    return '<p class="empty">Kein Transkript verfügbar</p>';
  }
  let snippets: TranscriptSnippet[];
  try {
    snippets = JSON.parse(raw) as TranscriptSnippet[];
  } catch {
    return `<p>${escapeHtml(raw).replace(/\n/g, "<br>")}</p>`;
  }

  let chapterIndex = 0;
  let html = "";
  for (const snippet of snippets) {
    while (chapters && chapterIndex < chapters.length && chapters[chapterIndex].start <= snippet.start) {
      const chapter = chapters[chapterIndex];
      html += `<button class="ts-chapter" data-start="${chapter.start}">${escapeHtml(chapter.title)}</button>`;
      chapterIndex += 1;
    }
    html += `<div class="ts-line"><span class="ts-time">${escapeHtml(snippet.time)}</span>${escapeHtml(snippet.text)}</div>`;
  }
  return html || '<p class="empty">Transkript ist leer</p>';
}

export function renderChapters(chapters?: Chapter[] | null) {
  if (!chapters || chapters.length === 0) {
    chaptersPanel.hidden = true;
    return;
  }

  chaptersPanel.hidden = false;
  chaptersList.innerHTML = chapters
    .map(
      (chapter) => `
      <button class="chapter-item" data-start="${chapter.start}">
        <span class="ts-time">${escapeHtml(chapter.time)}</span>
        ${escapeHtml(chapter.title)}
      </button>
    `,
    )
    .join("");

  chaptersList.querySelectorAll<HTMLButtonElement>(".chapter-item").forEach((item) => {
    item.addEventListener("click", () => {
      const start = Number(item.dataset.start);
      if (!Number.isNaN(start)) {
        seekVideo(start);
      }
    });
  });
}

export function updateDetailSummaryMeta(provider?: string | null, model?: string | null) {
  const summaryMeta = $("#detailSummaryMeta");
  const parts = [provider, model].filter((part): part is string => !!part && part.trim().length > 0);
  if (parts.length) {
    summaryMeta.textContent = `Zusammengefasst mit: ${parts.join(" / ")}`;
    summaryMeta.hidden = false;
  } else {
    summaryMeta.textContent = "";
    summaryMeta.hidden = true;
  }
}

export function buildYouTubeEmbedUrl(videoId: string, startSeconds?: number): string {
  const url = new URL(`https://www.youtube.com/embed/${encodeURIComponent(videoId)}`);
  url.searchParams.set("rel", "0");

  if (window.location.origin.startsWith("http")) {
    url.searchParams.set("origin", window.location.origin);
    url.searchParams.set("widget_referrer", window.location.origin);
  }

  if (startSeconds !== undefined) {
    url.searchParams.set("start", Math.floor(startSeconds).toString());
    url.searchParams.set("autoplay", "1");
  }

  return url.toString();
}

export function switchTab(tab: TabName) {
  state.activeTab = tab;
  document.querySelectorAll<HTMLButtonElement>(".tab").forEach((button) => {
    button.classList.toggle("active", button.dataset.tab === tab);
  });
  document.querySelectorAll<HTMLDivElement>(".tabPanel").forEach((panel) => {
    panel.classList.toggle("active", panel.id === `tab${capitalize(tab)}`);
  });
}

export function seekVideo(seconds: number) {
  const video = getActiveVideo();
  if (!video) return;
  $<HTMLIFrameElement>("#videoPlayer").src = buildYouTubeEmbedUrl(video.video_id, seconds);
  switchTab("video");
}

export function renderVideoCollections(video: Video) {
  const container = $("#collectionAssignment");
  if (!state.collections.length) {
    container.innerHTML = `
      <span class="collection-assignment-label">Sammlungen</span>
      <button class="inline-action" id="detailCreateCollection">Erste Sammlung erstellen</button>
    `;
    $("#detailCreateCollection").addEventListener("click", () => openCollectionDialog());
    return;
  }

  const selected = new Set(video.collection_ids);
  container.innerHTML = `
    <span class="collection-assignment-label">Sammlungen</span>
    <div class="collection-checkboxes">
      ${state.collections
        .map(
          (collection) => `
            <label class="collection-checkbox">
              <input type="checkbox" value="${collection.id}" ${selected.has(collection.id) ? "checked" : ""} />
              <span>${escapeHtml(collection.name)}</span>
            </label>
          `,
        )
        .join("")}
    </div>
  `;

  container.querySelectorAll<HTMLInputElement>('input[type="checkbox"]').forEach((input) => {
    input.addEventListener("change", () => void updateActiveVideoCollections());
  });
}

export async function updateActiveVideoCollections() {
  const video = getActiveVideo();
  if (!video || state.busy) return;
  const ids = Array.from(
    document.querySelectorAll<HTMLInputElement>('#collectionAssignment input[type="checkbox"]:checked'),
  )
    .map((input) => Number(input.value))
    .filter((id) => !Number.isNaN(id));

  setBusy(true, "Sammlungen werden gespeichert...");
  try {
    const updated = await invoke<Video>("set_video_collections", {
      videoId: video.id,
      collectionIds: ids,
    });
    state.videos = state.videos.map((item) => (item.id === updated.id ? updated : item));
    await loadCollections();
    renderVideoList();
    if (state.activeVideoId === video.id) {
      renderVideoCollections(updated);
      setStatus("Sammlungen gespeichert");
    }
  } catch (error) {
    if (state.activeVideoId === video.id) {
      setStatus(errorMessage(error));
      renderVideoCollections(video);
    }
  } finally {
    setBusy(false);
  }
}

export async function refreshActiveTranscript() {
  const video = getActiveVideo();
  if (!video || state.busy) return;

  setBusy(true, "Transkript wird geladen...");
  try {
    const updated = await invoke<Video>("refresh_transcript", { id: video.id });
    state.videos = state.videos.map((item) => (item.id === updated.id ? updated : item));
    renderVideoList();
    if (state.activeVideoId === video.id) {
      showDetail(updated);
      switchTab("transcript");
    }
    setStatus("Transkript geladen");
  } catch (error) {
    const message = errorMessage(error);
    try {
      const reloaded = await invoke<Video>("get_video_detail", { id: video.id });
      state.videos = state.videos.map((item) => (item.id === reloaded.id ? reloaded : item));
      renderVideoList();
      if (state.activeVideoId === video.id) {
        showDetail(reloaded);
      }
    } catch {
      // Schlägt das Nachladen fehl, wird es ignoriert
    }
    setStatus(message);
  } finally {
    setBusy(false);
  }
}

export function showDetail(video: Video) {
  detailPlaceholder.hidden = true;
  detailContent.hidden = false;
  $<HTMLImageElement>("#detailThumb").src = video.thumbnail || video.thumbnail_url;
  $("#detailTitle").textContent = video.title;
  const detailUrl = $<HTMLAnchorElement>("#detailUrl");
  detailUrl.href = video.url;
  detailUrl.textContent = video.url;
  const publishedMeta = $("#detailPublishedMeta");
  if (video.published_at) {
    publishedMeta.textContent = `Veröffentlicht: ${formatDate(video.published_at)}`;
    publishedMeta.hidden = false;
  } else {
    publishedMeta.textContent = "";
    publishedMeta.hidden = true;
  }
  updateDetailSummaryMeta(
    video.has_summary ? video.summary_provider : null,
    video.has_summary ? video.summary_model : null,
  );
  const descriptionDetails = $<HTMLDetailsElement>("#detailDescription");
  const descriptionText = $("#detailDescriptionText");
  descriptionDetails.open = false;
  if (video.description?.trim()) {
    descriptionText.innerHTML = renderDescriptionHtml(video.description);
    descriptionDetails.hidden = false;
  } else {
    descriptionText.textContent = "";
    descriptionDetails.hidden = true;
  }
  const videoFallbackLink = $<HTMLAnchorElement>("#videoFallbackLink");
  videoFallbackLink.href = video.url;
  $("#tabTranscript").innerHTML = renderTranscript(video.transcript, video.chapters, video.transcript_error);
  void renderSummaryTab(video);
  $<HTMLIFrameElement>("#videoPlayer").src = buildYouTubeEmbedUrl(video.video_id);
  const reloadBtn = $<HTMLButtonElement>("#reloadTranscriptBtn");
  reloadBtn.textContent = video.has_transcript ? "Neu laden" : "Transkript laden";
  reloadBtn.title = video.has_transcript
    ? "Transkript, Kapitel und Beschreibung neu von YouTube laden"
    : "";
  renderVideoCollections(video);
  renderChapters(video.chapters);
  switchTab(state.activeTab);
}

export function bindDetailEvents() {
  $("#reloadTranscriptBtn").addEventListener("click", () => void refreshActiveTranscript());

  document.querySelectorAll<HTMLButtonElement>(".tab").forEach((tab) => {
    tab.addEventListener("click", () => switchTab(tab.dataset.tab as TabName));
  });

  $("#tabTranscript").addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const chapter = target.closest<HTMLElement>(".ts-chapter");
    if (!chapter) return;
    const start = Number(chapter.dataset.start);
    if (!Number.isNaN(start)) {
      seekVideo(start);
    }
  });

  $("#detailDescriptionText").addEventListener("click", (event) => {
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
