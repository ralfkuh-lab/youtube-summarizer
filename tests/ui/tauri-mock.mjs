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
            const id = args?.id ?? args?.videoId;
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
          return [];
        }
        if (cmd === 'summary_presets_list') {
          return JSON.parse(JSON.stringify(this.fixtures.summaryPresets || []));
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
