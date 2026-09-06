import { invoke } from "@tauri-apps/api/core";
import DOMPurify from "dompurify";
import { marked } from "marked";
import { $, confirmDialog, errorMessage, escapeHtml } from "./dom-utils";
import { renderVideoList } from "./library";
import { showDetail, updateDetailSummaryMeta, seekVideo } from "./detail";
import { getActiveVideo, setBusy, setStatus, state } from "./state";
import type { SummaryRecord, Video } from "./types";

marked.setOptions({ gfm: true, breaks: false });

let summaryHistory: SummaryRecord[] = [];
let activeSummaryId: number | null = null;
let mermaidLoader: Promise<typeof import("mermaid").default> | null = null;
let mermaidIdSeq = 0;

const EMPTY_SUMMARY_HTML =
  '<p class="empty">Noch keine Zusammenfassung - klicke auf "Zusammenfassen lassen"</p>';

const TIMESTAMP_RE = /\[(?:(\d+):)?(\d{1,2}):(\d{2})\]/g;

// Some models wrap their entire Markdown reply in a single ```markdown ... ```
// code fence. marked would then render the whole summary as one <pre><code>
// block, showing the raw Markdown source. Strip such a wrapping fence before
// parsing. Only strips when the fence wraps the complete text and the content
// itself contains no further fences, so genuine code blocks stay intact.
export function stripWrappingCodeFence(markdown: string): string {
  const trimmed = markdown.trim();
  const match = /^```([^\n]*)\n([\s\S]*?)\n?```$/.exec(trimmed);
  if (!match) return markdown;
  const info = match[1].trim().toLowerCase();
  if (info !== "" && info !== "markdown" && info !== "md") return markdown;
  const inner = match[2];
  if (/^[ \t]*```/m.test(inner)) return markdown;
  return inner;
}

export function markdownToHtml(markdown: string): string {
  const rendered = marked.parse(stripWrappingCodeFence(markdown), { async: false }) as string;
  return DOMPurify.sanitize(rendered, {
    ADD_ATTR: ["target", "rel"],
  });
}

export function timestampSeconds(match: RegExpMatchArray): number | null {
  const hoursPart = match[1];
  const mid = Number(match[2]);
  const seconds = Number(match[3]);
  if (!Number.isFinite(mid) || !Number.isFinite(seconds) || seconds > 59) return null;
  if (hoursPart !== undefined) {
    const hours = Number(hoursPart);
    if (!Number.isFinite(hours) || mid > 59) return null;
    return hours * 3600 + mid * 60 + seconds;
  }
  if (mid > 59) return null;
  return mid * 60 + seconds;
}

export function linkifySummaryTimestamps(root: HTMLElement) {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      const parent = node.parentElement;
      if (!parent || parent.closest("a, pre, code, svg, button")) {
        return NodeFilter.FILTER_REJECT;
      }
      TIMESTAMP_RE.lastIndex = 0;
      return TIMESTAMP_RE.test(node.textContent ?? "")
        ? NodeFilter.FILTER_ACCEPT
        : NodeFilter.FILTER_REJECT;
    },
  });
  const nodes: Text[] = [];
  for (let current = walker.nextNode(); current; current = walker.nextNode()) {
    nodes.push(current as Text);
  }
  for (const textNode of nodes) {
    replaceTimestampsInTextNode(textNode);
  }
}

export function replaceTimestampsInTextNode(textNode: Text) {
  const text = textNode.textContent ?? "";
  TIMESTAMP_RE.lastIndex = 0;
  const frag = document.createDocumentFragment();
  let last = 0;
  let found = false;
  let match: RegExpExecArray | null;
  while ((match = TIMESTAMP_RE.exec(text))) {
    const seconds = timestampSeconds(match);
    if (seconds === null) continue;
    found = true;
    if (match.index > last) {
      frag.append(text.slice(last, match.index));
    }
    const link = document.createElement("a");
    link.href = "#";
    link.dataset.seek = String(seconds);
    link.className = "summary-seek";
    link.textContent = match[0];
    frag.append(link);
    last = match.index + match[0].length;
  }
  if (!found) return;
  if (last < text.length) {
    frag.append(text.slice(last));
  }
  textNode.replaceWith(frag);
}

export function getMermaid() {
  if (!mermaidLoader) {
    mermaidLoader = import("mermaid")
      .then((mod) => {
        const mermaid = mod.default;
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: "strict",
          theme: "dark",
          darkMode: true,
        });
        return mermaid;
      })
      .catch((error) => {
        mermaidLoader = null;
        throw error;
      });
  }
  return mermaidLoader;
}

export function removeMermaidArtifacts(id: string) {
  document.getElementById(id)?.remove();
  document.getElementById(`d${id}`)?.remove();
}

export async function renderMermaidBlocks(root: HTMLElement, gen: number) {
  const blocks = [...root.querySelectorAll("pre > code.language-mermaid")];
  if (!blocks.length) return;
  let mermaid;
  try {
    mermaid = await getMermaid();
  } catch {
    return;
  }
  if (gen !== state.summaryRenderGen) return;
  for (const code of blocks) {
    if (gen !== state.summaryRenderGen) return;
    const pre = code.parentElement;
    if (!(pre instanceof HTMLElement) || !pre.isConnected) continue;
    const source = (code.textContent ?? "").trim();
    if (!source) continue;
    const id = `summaryMermaid${(mermaidIdSeq += 1)}`;
    try {
      const { svg } = await mermaid.render(id, source);
      if (gen !== state.summaryRenderGen || !pre.isConnected) {
        removeMermaidArtifacts(id);
        return;
      }
      const wrapper = document.createElement("div");
      wrapper.className = "summary-mermaid";
      wrapper.innerHTML = DOMPurify.sanitize(svg, {
        USE_PROFILES: { svg: true, svgFilters: true },
      });
      if (!wrapper.querySelector("svg")) {
        removeMermaidArtifacts(id);
        continue;
      }
      pre.replaceWith(wrapper);
    } catch {
      removeMermaidArtifacts(id);
    }
  }
}

export async function renderSummaryMarkdown(markdown: string, gen = ++state.summaryRenderGen) {
  const body = $("#summaryBody");
  body.innerHTML = markdownToHtml(markdown);
  linkifySummaryTimestamps(body);
  await renderMermaidBlocks(body, gen);
}

export function hideSummaryHistoryBar() {
  $<HTMLDivElement>("#summaryHistoryBar").hidden = true;
  activeSummaryId = null;
}

export function formatSummaryHistoryTime(value: string): string {
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

export function summaryHistoryLabel(row: SummaryRecord): string {
  const model = row.model?.trim() || row.provider?.trim() || "unbekannt";
  return `${formatSummaryHistoryTime(row.created_at)} – ${model}`;
}

export function fillSummaryHistorySelect(selectedId: number) {
  const select = $<HTMLSelectElement>("#summaryHistorySelect");
  select.innerHTML = summaryHistory
    .map(
      (row) =>
        `<option value="${row.id}">${escapeHtml(summaryHistoryLabel(row))}</option>`,
    )
    .join("");
  select.value = String(selectedId);
}

export async function renderSummaryTab(video: Video) {
  const gen = ++state.summaryRenderGen;
  const body = $("#summaryBody");
  if (state.streamingVideoId === video.id) {
    hideSummaryHistoryBar();
    body.innerHTML = '<p class="empty">Zusammenfassung wird erstellt…</p>';
    return;
  }

  hideSummaryHistoryBar();
  if (video.summary) {
    body.innerHTML = markdownToHtml(video.summary);
    linkifySummaryTimestamps(body);
  } else {
    body.innerHTML = EMPTY_SUMMARY_HTML;
  }

  try {
    summaryHistory = await invoke<SummaryRecord[]>("get_summaries", { videoId: video.id });
  } catch (error) {
    summaryHistory = [];
    if (gen === state.summaryRenderGen) setStatus(errorMessage(error));
  }
  if (gen !== state.summaryRenderGen || getActiveVideo()?.id !== video.id) return;

  if (summaryHistory.length) {
    const newest = summaryHistory[0];
    activeSummaryId = newest.id;
    fillSummaryHistorySelect(newest.id);
    $<HTMLDivElement>("#summaryHistoryBar").hidden = false;
    await renderSummaryMarkdown(newest.summary, gen);
    return;
  }

  if (video.summary) {
    await renderMermaidBlocks(body, gen);
  }
}

export async function showSummaryVersion(id: number) {
  const row = summaryHistory.find((item) => item.id === id);
  if (!row) return;
  activeSummaryId = row.id;
  updateDetailSummaryMeta(row.provider, row.model);
  await renderSummaryMarkdown(row.summary);
}

export async function deleteDisplayedSummary() {
  if (state.busy || activeSummaryId === null) return;
  const video = getActiveVideo();
  if (!video) return;
  if (
    !(await confirmDialog("Diese Zusammenfassungs-Version wirklich löschen?", {
      title: "Zusammenfassung löschen",
      okLabel: "Löschen",
    }))
  ) {
    return;
  }
  const id = activeSummaryId;
  setBusy(true, "Zusammenfassung wird gelöscht...");
  try {
    await invoke<void>("delete_summary", { id });
    const updated = await invoke<Video>("get_video_detail", { id: video.id });
    state.videos = state.videos.map((item) => (item.id === updated.id ? updated : item));
    renderVideoList();
    if (getActiveVideo()?.id === updated.id) {
      showDetail(updated);
    }
    setStatus("Zusammenfassung gelöscht");
  } catch (error) {
    setStatus(errorMessage(error));
  } finally {
    setBusy(false);
  }
}

export function bindSummaryViewEvents() {
  $("#tabSummary").addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const link = target.closest<HTMLElement>("[data-seek]");
    if (!link || !$("#tabSummary").contains(link)) return;
    event.preventDefault();
    const seconds = Number(link.dataset.seek);
    if (!Number.isNaN(seconds)) {
      seekVideo(seconds);
    }
  });

  $("#summaryHistorySelect").addEventListener("change", () => {
    const id = Number($<HTMLSelectElement>("#summaryHistorySelect").value);
    if (!Number.isNaN(id)) void showSummaryVersion(id);
  });
  $("#summaryHistoryDelete").addEventListener("click", () => void deleteDisplayedSummary());
}
