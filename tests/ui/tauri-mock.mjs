export const defaultFixtures = {
  videos: [
    {
      id: 1,
      video_id: "vid1",
      url: "https://www.youtube.com/watch?v=vid1",
      title: "Video Eins (ohne Transkript)",
      thumbnail_url: "https://example.com/thumb1.jpg",
      thumbnail: null,
      transcript: null,
      chapters: [],
      summary: null,
      summary_provider: null,
      summary_model: null,
      published_at: "2026-01-01T00:00:00Z",
      description: "Beschreibung 1",
      collection_ids: [],
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
      transcript_error: null,
      has_transcript: false,
      has_summary: false,
    },
    {
      id: 2,
      video_id: "vid2",
      url: "https://www.youtube.com/watch?v=vid2",
      title: "Video Zwei (mit Transkript)",
      thumbnail_url: "https://example.com/thumb2.jpg",
      thumbnail: null,
      transcript: "Dies ist das Transkript von Video 2.",
      chapters: [],
      summary: "Zusammenfassung Video 2",
      summary_provider: "openai",
      summary_model: "gpt-4o",
      published_at: "2026-01-02T00:00:00Z",
      description: "Beschreibung 2",
      collection_ids: [],
      created_at: "2026-01-02T00:00:00Z",
      updated_at: "2026-01-02T00:00:00Z",
      transcript_error: null,
      has_transcript: true,
      has_summary: true,
    },
    {
      id: 3,
      video_id: "vid3",
      url: "https://www.youtube.com/watch?v=vid3",
      title: "Video Drei (mit Transkript)",
      thumbnail_url: "https://example.com/thumb3.jpg",
      thumbnail: null,
      transcript: "Dies ist das Transkript von Video 3.",
      chapters: [],
      summary: null,
      summary_provider: null,
      summary_model: null,
      published_at: "2026-01-03T00:00:00Z",
      description: "Beschreibung 3",
      collection_ids: [],
      created_at: "2026-01-03T00:00:00Z",
      updated_at: "2026-01-03T00:00:00Z",
      transcript_error: null,
      has_transcript: true,
      has_summary: false,
    },
  ],
  collections: [],
  aiConfig: {
    provider: {
      openai: {
        enabled: true,
        name: "OpenAI",
        whitelist: ["gpt-4o"],
      },
    },
    defaultModel: { provider: "openai", model: "gpt-4o" },
  },
  catalog: {
    catalog: {
      openai: {
        id: "openai",
        name: "OpenAI",
        models: {
          "gpt-4o": { id: "gpt-4o", name: "GPT-4o" },
        },
      },
    },
    source: "cache",
    updatedAt: "2026-09-01T00:00:00Z",
  },
  summaryPresets: [
    {
      id: "standard",
      name: "Standard",
      prompt: "Standard Prompt",
      builtin: true,
    },
  ],
  chat: {
    answer: "Antwort vom Mock-Modell",
    error: null,
  },
  summaries: [],
  webSearchConfig: {
    enabled: false,
    searxngUrl: "",
  },
  webSearchTest: {
    count: 3,
    error: null,
  },
  agent: {
    view: {
      config: {
        workdirBase: "",
        shell: "auto",
        summaries: "latest",
        includeChats: false,
        prompt: "",
        activeTemplate: "claude",
        customTemplates: [],
      },
      builtinTemplates: [
        { id: "claude", name: "Claude Code", command: "cd {workdir} && claude {prompt}" },
        { id: "codex", name: "Codex", command: "cd {workdir} && codex {prompt}" },
      ],
      defaultWorkdirBase: "/home/user/yt-agent",
      effectiveShell: "posix",
    },
    prepare: {
      command: "cd '/home/user/yt-agent/video-zwei-vid2' && claude 'Hallo'",
      workdir: "/home/user/yt-agent/video-zwei-vid2",
      contextFile: "/home/user/yt-agent/video-zwei-vid2/context.md",
    },
    prepareByTemplate: {
      codex: {
        command: "cd '/home/user/yt-agent/video-zwei-vid2' && codex 'Hallo'",
        workdir: "/home/user/yt-agent/video-zwei-vid2",
        contextFile: "/home/user/yt-agent/video-zwei-vid2/context.md",
      },
    },
    preview: { text: "", error: null },
  },
};

export function createMockScript(fixtures = {}, delays = {}) {
  const mergedFixtures = { ...defaultFixtures, ...fixtures };
  const fixturesJson = JSON.stringify(mergedFixtures);
  const delaysJson = JSON.stringify(delays);

  return `
(() => {
  const initialFixtures = ${fixturesJson};
  const initialDelays = ${delaysJson};
  const callbacks = new Map();
  let nextCallbackId = 1;
  const chatStore = { chats: [], messages: {}, nextChatId: 1 };
  const cancelledRequests = new Set();

  function seedChatStore(seed) {
    for (const [videoId, entries] of Object.entries(seed || {})) {
      for (const entry of entries) {
        const id = chatStore.nextChatId++;
        const createdAt = entry.createdAt || '2026-01-01T00:00:00Z';
        const updatedAt = entry.updatedAt || createdAt;
        chatStore.chats.push({
          id,
          videoId: Number(videoId),
          title: entry.title,
          createdAt,
          updatedAt,
          contextOptions: entry.contextOptions || { transcript: true, summaryIds: null },
        });
        chatStore.messages[id] = (entry.messages || []).map((message, index) => ({
          id: index + 1,
          chatId: id,
          role: message.role,
          content: message.content,
          toolCalls: message.toolCalls !== undefined ? message.toolCalls : null,
          toolCallId: message.toolCallId !== undefined ? message.toolCallId : null,
          provider: message.provider !== undefined ? message.provider : null,
          model: message.model !== undefined ? message.model : null,
          createdAt,
        }));
      }
    }
  }

  seedChatStore(initialFixtures.chatSeed);

  function chatTitle(text) {
    const normalized = String(text ?? '').trim().split(/\\s+/).join(' ');
    const chars = [...normalized];
    return chars.length > 60 ? chars.slice(0, 60).join('') + '…' : normalized;
  }

  function chatMessagesOf(chatId) {
    return chatStore.messages[chatId] || [];
  }

  function chatsOfVideo(videoId) {
    return chatStore.chats
      .filter((chat) => chat.videoId === videoId)
      .sort((a, b) => {
        if (a.updatedAt !== b.updatedAt) return a.updatedAt < b.updatedAt ? 1 : -1;
        return b.id - a.id;
      });
  }

  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: () => {},
  };

  window.__tauriMock = {
    pending: 0,
    calls: [],
    delays: initialDelays,
    fixtures: initialFixtures,
    listeners: [],
    emit: function(event, payload) {
      for (const entry of this.listeners) {
        if (entry.event === event) {
          const fn = window['_' + entry.handlerId];
          if (typeof fn === 'function') {
            fn({ event, id: entry.eventId, payload });
          }
        }
      }
    },
    handleInvoke: async function(cmd, args, options) {
      this.pending++;
      const callRecord = { cmd, args, time: Date.now() };
      this.calls.push(callRecord);

      try {
        let delayMs = 0;
        if (this.delays && this.delays[cmd] !== undefined) {
          const d = this.delays[cmd];
          if (typeof d === 'number') {
            delayMs = d;
          } else if (typeof d === 'object' && d !== null) {
            const id = args?.id ?? args?.videoId ?? args?.chatId;
            if (id !== undefined && d[id] !== undefined) {
              delayMs = d[id];
            } else if (d.default !== undefined) {
              delayMs = d.default;
            }
          }
        }

        if (delayMs > 0) {
          await new Promise((resolve) => setTimeout(resolve, delayMs));
        }

        if (cmd === 'plugin:event|listen') {
          const eventId = 1000 + this.calls.length;
          this.listeners.push({
            eventId,
            event: args?.event,
            handlerId: args?.handler,
          });
          return eventId;
        }
        if (cmd === 'plugin:event|unlisten') {
          const eventId = args?.eventId;
          this.listeners = this.listeners.filter((l) => l.eventId !== eventId);
          return null;
        }
        if (cmd === 'plugin:opener|open_url') {
          return null;
        }
        if (cmd === 'plugin:opener|reveal_item_in_dir') {
          return null;
        }
        if (cmd === 'get_videos') {
          const res = JSON.parse(JSON.stringify(this.fixtures.videos.map((v) => ({
            ...v,
            transcript: null,
            chapters: null,
            summary: null,
            description: null,
          }))));
          callRecord.result = res;
          return res;
        }
        if (cmd === 'get_collections') {
          return JSON.parse(JSON.stringify(this.fixtures.collections || []));
        }
        if (cmd === 'ai_config_get') {
          return JSON.parse(JSON.stringify(this.fixtures.aiConfig));
        }
        if (cmd === 'ai_catalog_get') {
          return JSON.parse(JSON.stringify(this.fixtures.catalog || { catalog: {}, source: 'cache', updatedAt: '' }));
        }
        if (cmd === 'get_video_detail') {
          const id = args?.id;
          const v = (this.fixtures.videoDetails && this.fixtures.videoDetails[id]) ||
                    this.fixtures.videos.find((item) => item.id === id);
          if (!v) {
            throw new Error('Video not found for id ' + id);
          }
          return JSON.parse(JSON.stringify(v));
        }
        if (cmd === 'refresh_transcript') {
          const id = args?.id;
          if (this.fixtures.refreshTranscript && this.fixtures.refreshTranscript[id]) {
            return JSON.parse(JSON.stringify(this.fixtures.refreshTranscript[id]));
          }
          const v = this.fixtures.videos.find((item) => item.id === id);
          if (!v) {
            throw new Error('Video not found for refresh_transcript id ' + id);
          }
          const updated = {
            ...v,
            transcript: 'Nachgeladenes Transkript für Video ' + id,
            has_transcript: true,
            transcript_error: null,
          };
          return JSON.parse(JSON.stringify(updated));
        }
        if (cmd === 'delete_video') {
          return null;
        }
        if (cmd === 'get_summaries') {
          const rows = (this.fixtures.summaries || []).filter(
            (row) => row.video_id === args?.videoId,
          );
          callRecord.result = rows;
          return JSON.parse(JSON.stringify(rows));
        }
        if (cmd === 'chat_context_set') {
          const chat = chatStore.chats.find((item) => item.id === args?.chatId);
          if (!chat) {
            throw new Error('Chat wurde gelöscht');
          }
          chat.contextOptions = args?.options || { transcript: true, summaryIds: null };
          callRecord.result = chat;
          return JSON.parse(JSON.stringify(chat));
        }
        if (cmd === 'summary_presets_list') {
          return JSON.parse(JSON.stringify(this.fixtures.summaryPresets || []));
        }
        if (cmd === 'ai_auth_status') {
          return {};
        }
        if (cmd === 'chat_list') {
          const chats = chatsOfVideo(args?.videoId);
          callRecord.result = chats;
          return JSON.parse(JSON.stringify(chats));
        }
        if (cmd === 'chat_messages') {
          const messages = chatMessagesOf(args?.chatId);
          callRecord.result = messages;
          return JSON.parse(JSON.stringify(messages));
        }
        if (cmd === 'chat_send') {
          const chatFixture = this.fixtures.chat || {};
          if (cancelledRequests.has(args?.requestId)) {
            throw new Error('KI-Antwort abgebrochen');
          }
          if (chatFixture.error) {
            throw new Error(chatFixture.error);
          }
          const now = new Date().toISOString();
          let chat = chatStore.chats.find((item) => item.id === args?.chatId);
          if (args?.chatId !== null && args?.chatId !== undefined && !chat) {
            throw new Error('Chat wurde gelöscht');
          }
          if (!chat) {
            chat = {
              id: chatStore.nextChatId++,
              videoId: args.videoId,
              title: chatTitle(args.text),
              createdAt: now,
              updatedAt: now,
              contextOptions:
                args?.contextOptions || { transcript: true, summaryIds: null },
            };
            chatStore.chats.push(chat);
            chatStore.messages[chat.id] = [];
          } else {
            chat.updatedAt = now;
            if (args?.contextOptions) {
              chat.contextOptions = args.contextOptions;
            }
          }
          const answer = chatFixture.answer !== undefined ? chatFixture.answer : 'Antwort vom Mock-Modell';
          const messages = chatStore.messages[chat.id];
          messages.push({
            id: messages.length + 1,
            chatId: chat.id,
            role: 'user',
            content: String(args.text).trim(),
            toolCalls: null,
            toolCallId: null,
            provider: null,
            model: null,
            createdAt: now,
          });
          messages.push({
            id: messages.length + 1,
            chatId: chat.id,
            role: 'assistant',
            content: answer,
            toolCalls: null,
            toolCallId: null,
            provider: 'openai',
            model: 'gpt-4o',
            createdAt: now,
          });
          const result = { chat, messages };
          callRecord.result = result;
          return JSON.parse(JSON.stringify(result));
        }
        if (cmd === 'web_search_config_get') {
          const config = this.fixtures.webSearchConfig || { enabled: false, searxngUrl: '' };
          callRecord.result = config;
          return JSON.parse(JSON.stringify(config));
        }
        if (cmd === 'web_search_config_set') {
          const config = args?.config || { enabled: false, searxngUrl: '' };
          this.fixtures.webSearchConfig = config;
          callRecord.result = config;
          return JSON.parse(JSON.stringify(config));
        }
        if (cmd === 'web_search_test') {
          const fixture = this.fixtures.webSearchTest || {};
          if (fixture.error) {
            throw new Error(fixture.error);
          }
          return fixture.count !== undefined ? fixture.count : 3;
        }
        if (cmd === 'agent_config_get') {
          const agent = this.fixtures.agent || {};
          callRecord.result = agent.view;
          return JSON.parse(JSON.stringify(agent.view));
        }
        if (cmd === 'agent_config_set') {
          const agent = this.fixtures.agent || (this.fixtures.agent = {});
          if (agent.configSetError) {
            throw new Error(agent.configSetError);
          }
          const config = args?.config || {};
          const invalid = (config.customTemplates || []).find(
            (template) => !/\{(prompt|context_file|workdir)\}/.test(template.command || ''),
          );
          if (invalid) {
            // Wie das Backend: ohne Kontext-Platzhalter wird nichts gespeichert.
            throw new Error('Die Vorlage nutzt keinen Kontext-Platzhalter');
          }
          if (agent.view) {
            agent.view.config = config;
          }
          callRecord.result = agent.view;
          return JSON.parse(JSON.stringify(agent.view));
        }
        if (cmd === 'agent_prepare') {
          const agent = this.fixtures.agent || {};
          if (agent.prepareError) {
            throw new Error(agent.prepareError);
          }
          const byTemplate = agent.prepareByTemplate || {};
          const result = (args?.templateId && byTemplate[args.templateId]) || agent.prepare;
          callRecord.result = result;
          return JSON.parse(JSON.stringify(result));
        }
        if (cmd === 'agent_preview') {
          const agent = this.fixtures.agent || {};
          const preview = agent.preview || {};
          if (preview.error) {
            throw new Error(preview.error);
          }
          const text = preview.text || 'VORSCHAU ' + String(args?.command ?? '');
          callRecord.result = text;
          return text;
        }
        if (cmd === 'chat_cancel') {
          cancelledRequests.add(args?.requestId);
          return null;
        }
        if (cmd === 'chat_delete') {
          chatStore.chats = chatStore.chats.filter((chat) => chat.id !== args?.chatId);
          delete chatStore.messages[args?.chatId];
          return null;
        }

        throw new Error('Unhandled Tauri mock command: ' + cmd + ' with args: ' + JSON.stringify(args));
      } finally {
        this.pending--;
      }
    }
  };

  window.__TAURI_INTERNALS__ = {
    invoke: (cmd, args, options) => window.__tauriMock.handleInvoke(cmd, args, options),
    transformCallback: (callback, once = false) => {
      const id = nextCallbackId++;
      const prop = '_' + id;
      callbacks.set(id, callback);
      window[prop] = (result) => {
        if (once) {
          callbacks.delete(id);
          delete window[prop];
        }
        return callback && callback(result);
      };
      return id;
    },
    unregisterCallback: (id) => {
      callbacks.delete(id);
      delete window['_' + id];
    },
    metadata: {
      currentWindow: { label: 'main' },
      currentWebview: { windowLabel: 'main', label: 'main' },
    },
  };
})();
`;
}
