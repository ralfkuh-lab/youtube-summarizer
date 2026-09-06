export type Chapter = {
  time: string;
  start: number;
  title: string;
};

export type TranscriptSnippet = {
  text: string;
  start: number;
  time: string;
};

export type Video = {
  id: number;
  video_id: string;
  url: string;
  title: string;
  thumbnail_url: string;
  thumbnail?: string | null;
  transcript?: string | null;
  chapters?: Chapter[] | null;
  summary?: string | null;
  summary_provider?: string | null;
  summary_model?: string | null;
  published_at?: string | null;
  description?: string | null;
  collection_ids: number[];
  created_at: string;
  updated_at: string;
  transcript_error?: string | null;
  has_transcript: boolean;
  has_summary: boolean;
};

export type Collection = {
  id: number;
  name: string;
  video_count: number;
  created_at: string;
  updated_at: string;
};

export type TabName = "transcript" | "summary" | "video";

export type SummaryRecord = {
  id: number;
  video_id: number;
  created_at: string;
  summary: string;
  provider?: string | null;
  model?: string | null;
  options?: string | null;
};

export type SummaryPreset = {
  id: string;
  name: string;
  prompt: string;
  builtin: boolean;
};

export type SummaryModules = {
  tables: boolean;
  mermaid: boolean;
  assessment: boolean;
  verify: boolean;
  timestamps: boolean;
  links: boolean;
};

export type VideoStatusFilter = "all" | "transcript" | "missing-transcript" | "summary" | "missing-summary";

export type SummarySettings = {
  detail: string;
  lang: string;
  useChapters: string;
  // JSON-kodiertes [providerId, modelId] wie im Modell-Auswahlfeld
  model?: string;
  presetId: string;
  modules: SummaryModules;
};
