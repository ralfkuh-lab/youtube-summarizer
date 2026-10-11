// Suche im aktiven Tab (Transkript, Zusammenfassung, Chat). Treffer werden
// ueber die CSS Custom Highlight API markiert, das DOM bleibt dabei
// unveraendert - wichtig fuer gerendertes Markdown, Mermaid und den Chat, die
// ihr DOM selbst neu aufbauen. Aendert sich der Inhalt, laeuft die Suche per
// MutationObserver erneut.

import { $ } from "./dom-utils";
import { state } from "./state";
import type { TabName } from "./types";

// Durchsuchte Bereiche je Tab; Video hat keinen Text
const SEARCH_ROOTS: Partial<Record<TabName, string>> = {
  transcript: "#tabTranscript",
  summary: "#summaryBody",
  chat: "#chatMessages",
};

// Text in diesen Elementen ist kein Lesetext (Diagramme, Auswahllisten, Eingaben)
const SKIPPED = "svg, select, option, script, style, textarea, input";

let matches: Range[] = [];
let current = -1;
let observer: MutationObserver | null = null;
let rerunTimer: number | undefined;

const bar = () => $<HTMLDivElement>("#findBar");
const input = () => $<HTMLInputElement>("#findInput");

function isOpen(): boolean {
  return !bar().hidden;
}

function searchRoot(): HTMLElement | null {
  const selector = SEARCH_ROOTS[state.activeTab];
  return selector ? document.querySelector<HTMLElement>(selector) : null;
}

function collectMatches(root: HTMLElement, query: string): Range[] {
  const needle = query.toLowerCase();
  const found: Range[] = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.nodeValue?.toLowerCase() ?? "";
    let index = text.indexOf(needle);
    if (index < 0) continue;
    const parent = node.parentElement;
    // Unsichtbares (eingeklappt, hidden) ueberspringen - dorthin kann nicht gescrollt werden
    if (!parent || parent.closest(SKIPPED) || parent.getClientRects().length === 0) continue;
    while (index >= 0) {
      const range = document.createRange();
      range.setStart(node, index);
      range.setEnd(node, index + needle.length);
      found.push(range);
      index = text.indexOf(needle, index + needle.length);
    }
  }
  return found;
}

function paint() {
  if (!("highlights" in CSS)) return;
  CSS.highlights.set("find-match", new Highlight(...matches));
  const active = matches[current];
  if (active) CSS.highlights.set("find-current", new Highlight(active));
  else CSS.highlights.delete("find-current");
}

function updateCount() {
  const query = input().value;
  const count = $("#findCount");
  if (!query || !searchRoot()) count.textContent = "";
  else if (matches.length === 0) count.textContent = "Keine Treffer";
  else count.textContent = `${current + 1}/${matches.length}`;
  $<HTMLButtonElement>("#findPrev").disabled = matches.length === 0;
  $<HTMLButtonElement>("#findNext").disabled = matches.length === 0;
}

function scrollToCurrent() {
  const target = matches[current]?.startContainer.parentElement;
  target?.scrollIntoView({ block: "center" });
}

// keepPosition: bei Inhaltsaenderungen den aktuellen Treffer halten und nicht scrollen
function runSearch(keepPosition = false) {
  const root = searchRoot();
  const query = input().value;
  matches = root && query ? collectMatches(root, query) : [];
  current = matches.length === 0 ? -1 : keepPosition ? Math.min(Math.max(current, 0), matches.length - 1) : 0;
  paint();
  updateCount();
  if (!keepPosition) scrollToCurrent();
}

function observeActiveTab() {
  observer?.disconnect();
  const root = searchRoot();
  if (!root) return;
  observer ??= new MutationObserver(() => {
    window.clearTimeout(rerunTimer);
    rerunTimer = window.setTimeout(() => runSearch(true), 150);
  });
  observer.observe(root, { childList: true, subtree: true, characterData: true });
}

function step(delta: number) {
  if (matches.length === 0) return;
  current = (current + delta + matches.length) % matches.length;
  paint();
  updateCount();
  scrollToCurrent();
}

export function openFind() {
  if (!searchRoot()) return;
  bar().hidden = false;
  $("#findToggle").classList.add("active");
  const field = input();
  field.focus();
  field.select();
  observeActiveTab();
  runSearch();
}

export function closeFind() {
  bar().hidden = true;
  $("#findToggle").classList.remove("active");
  observer?.disconnect();
  window.clearTimeout(rerunTimer);
  matches = [];
  current = -1;
  if ("highlights" in CSS) {
    CSS.highlights.delete("find-match");
    CSS.highlights.delete("find-current");
  }
}

// Von switchTab aufgerufen: Button-Zustand anpassen, offene Suche im neuen Tab wiederholen
export function onFindTabChanged() {
  const searchable = !!SEARCH_ROOTS[state.activeTab];
  $<HTMLButtonElement>("#findToggle").disabled = !searchable;
  if (!isOpen()) return;
  input().disabled = !searchable;
  observeActiveTab();
  runSearch();
}

function anyModalOpen(): boolean {
  return !!document.querySelector(".modal:not([hidden]), .settings-ai-overlay:not([hidden])");
}

export function bindFindEvents() {
  $("#findToggle").addEventListener("click", () => (isOpen() ? closeFind() : openFind()));
  $("#findClose").addEventListener("click", closeFind);
  $("#findPrev").addEventListener("click", () => step(-1));
  $("#findNext").addEventListener("click", () => step(1));
  input().addEventListener("input", () => runSearch());
  input().addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      step(event.shiftKey ? -1 : 1);
    } else if (event.key === "Escape") {
      event.preventDefault();
      closeFind();
    }
  });

  document.addEventListener("keydown", (event) => {
    if ($("#detailContent").hidden || anyModalOpen()) return;
    const ctrlF = (event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === "f";
    if (ctrlF) {
      event.preventDefault();
      openFind();
    } else if (event.key === "F3") {
      event.preventDefault();
      if (isOpen()) step(event.shiftKey ? -1 : 1);
      else openFind();
    }
  });
}
