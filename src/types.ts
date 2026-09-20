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

export type TabName = "transcript" | "summary" | "chat" | "video";

export type ChatContextOptions = {
  transcript: boolean;
  /// `null` = neueste Zusammenfassung, `[]` = keine.
  summaryIds: number[] | null;
};

export type Chat = {
  id: number;
  videoId: number;
  title: string;
  createdAt: string;
  updatedAt: string;
  contextOptions: ChatContextOptions;
};

export type ChatMessageRecord = {
  id: number;
  chatId: number;
  role: string;
  content: string;
  toolCalls?: unknown | null;
  toolCallId?: string | null;
  provider?: string | null;
  model?: string | null;
  createdAt: string;
};

export type ChatTurnResult = {
  chat: Chat;
  messages: ChatMessageRecord[];
};

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

export type AgentShell = "auto" | "posix" | "fish" | "powershell";

export type AgentSummaries = "latest" | "all" | "none";

export type AgentTemplate = {
  id: string;
  name: string;
  command: string;
};

export type AgentConfig = {
  workdirBase: string;
  shell: AgentShell;
  summaries: AgentSummaries;
  /// Vorbelegung: Transkript in die Kontextdatei aufnehmen.
  includeTranscript: boolean;
  includeChats: boolean;
  prompt: string;
  activeTemplate: string;
  customTemplates: AgentTemplate[];
};

export type AgentConfigView = {
  config: AgentConfig;
  builtinTemplates: AgentTemplate[];
  defaultWorkdirBase: string;
  /// Standard-Prompt des Backends: wird bei leerem Feld als Platzhalter gezeigt.
  defaultPrompt: string;
  effectiveShell: string;
};

export type AgentHandoff = {
  command: string;
  workdir: string;
  contextFile: string;
  /// Unicode-Skalare der geschriebenen Kontextdatei.
  contextChars: number;
  /// Wirksame Auswahl dieser Übergabe.
  selection: AgentHandoffSelection;
  /// Was der Dialog auswählen kann.
  available: AgentAvailable;
};

/// Auswahl des Kontexts für eine Übergabe (Revision 3).
export type AgentHandoffSelection = {
  transcript: boolean;
  /// `null` = neueste Zusammenfassung, `[]` = keine.
  summaryIds: number[] | null;
  chatIds: number[];
};

export type AgentSummaryOption = {
  id: number;
  createdAt: string;
  provider?: string | null;
  model?: string | null;
  options?: string | null;
};

export type AgentChatOption = {
  id: number;
  title: string;
  createdAt: string;
  messageCount: number;
  firstQuestion?: string | null;
};

export type AgentAvailable = {
  hasTranscript: boolean;
  hasLatestSummary: boolean;
  /// Neueste zuerst.
  summaries: AgentSummaryOption[];
  /// Neueste zuerst.
  chats: AgentChatOption[];
};

export type SummarySettings = {
  detail: string;
  lang: string;
  useChapters: string;
  // JSON-kodiertes [providerId, modelId] wie im Modell-Auswahlfeld
  model?: string;
  presetId: string;
  modules: SummaryModules;
};
