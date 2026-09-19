// Live-Segmente einer laufenden Chat-Anfrage: reine Zustandslogik ohne DOM.
// Die Anzeige (`chat-render.ts`) baut aus diesen Segmenten dasselbe Layout wie
// der gespeicherte Verlauf: Textblase einer Provider-Runde, darunter ihre
// Recherche-Schritte.

import type { ChatLiveSegment, ChatRun, ChatToolStep } from "./state";

/// Payload von `ai:chat_stream`. `round` ist die 0-basierte Provider-Runde
/// dieser Frage, `final` schliesst sie ab (auch mit leerem `text`),
/// `discarded` verwirft sie wieder (Markup-Antwort vor der Wiederholung).
export interface ChatStreamEvent {
  requestId: string;
  videoId: number;
  round: number;
  text: string;
  final: boolean;
  discarded?: boolean;
}

/// Payload von `ai:chat_tool`; `round` ist die Runde, deren Assistant-Turn den
/// Aufruf ausgeloest hat.
export interface ChatToolEvent {
  requestId: string;
  videoId: number;
  round: number;
  kind: string;
  label: string;
  status: string;
}

export function createChatRun(input: {
  requestId: string;
  chatId: number | null;
  question: string;
}): ChatRun {
  return { ...input, segments: [] };
}

/// Segment einer Runde; fehlende Runden werden in Reihenfolge eingefuegt.
function segmentFor(run: ChatRun, round: number): ChatLiveSegment {
  const existing = run.segments.find((segment) => segment.round === round);
  if (existing) return existing;
  const segment: ChatLiveSegment = { round, text: "", final: false, tools: [] };
  const index = run.segments.findIndex((entry) => entry.round > round);
  if (index < 0) {
    run.segments.push(segment);
  } else {
    run.segments.splice(index, 0, segment);
  }
  return segment;
}

/// Uebernimmt ein Stream-Event: der Text ersetzt den Stand seiner Runde.
export function applyStreamEvent(run: ChatRun, event: ChatStreamEvent) {
  if (event.discarded) {
    const index = run.segments.findIndex((segment) => segment.round === event.round);
    if (index >= 0) run.segments.splice(index, 1);
    return;
  }
  const segment = segmentFor(run, event.round);
  segment.text = event.text;
  segment.final = event.final;
}

/// Uebernimmt ein Werkzeug-Event: `start` haengt eine Zeile an seiner Runde an,
/// `ok`/`error` aktualisiert die zuletzt offene Zeile mit gleichem kind+label.
export function applyToolEvent(run: ChatRun, event: ChatToolEvent) {
  const step: ChatToolStep = {
    kind: event.kind === "fetch" ? "fetch" : event.kind === "other" ? "other" : "search",
    label: event.label,
    status: event.status === "ok" || event.status === "error" ? event.status : "start",
  };
  const segment = segmentFor(run, event.round);
  if (step.status === "start") {
    segment.tools.push(step);
    return;
  }
  for (let index = segment.tools.length - 1; index >= 0; index -= 1) {
    const entry = segment.tools[index];
    if (entry.status === "start" && entry.kind === step.kind && entry.label === step.label) {
      segment.tools[index] = step;
      return;
    }
  }
  segment.tools.push(step);
}

/// Ein sichtbarer Block der Live-Anzeige: eine Textblase einer Runde oder eine
/// zusammenhaengende Folge von Recherche-Schritten.
export interface ChatLiveBlock {
  kind: "bubble" | "group";
  text: string;
  steps: ChatToolStep[];
}

/// Die Bloecke in Anzeigereihenfolge. Runden ohne Text erzeugen keine leere
/// Blase; ihre Schritte haengen an der laufenden Gruppe - dieselbe Regel wie im
/// gespeicherten Verlauf (dort bricht nur eine Textblase die Gruppe).
export function liveBlocks(run: ChatRun): ChatLiveBlock[] {
  const blocks: ChatLiveBlock[] = [];
  let group: ChatLiveBlock | null = null;
  for (const segment of run.segments) {
    if (segment.text.trim()) {
      blocks.push({ kind: "bubble", text: segment.text, steps: [] });
      group = null;
    }
    if (segment.tools.length) {
      if (!group) {
        group = { kind: "group", text: "", steps: [] };
        blocks.push(group);
      }
      group.steps.push(...segment.tools);
    }
  }
  return blocks;
}
