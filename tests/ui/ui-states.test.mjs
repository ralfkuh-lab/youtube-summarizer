import test from "node:test";
import assert from "node:assert/strict";
import { withApp } from "./harness.mjs";

const COLLECTIONS = [
  {
    id: 1,
    name: "Leere Sammlung",
    video_count: 0,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  },
];

const SUMMARIES = [
  {
    id: 11,
    video_id: 2,
    summary: "# Zusammenfassung\n\nInhalt",
    provider: "openai",
    model: "gpt-4o",
    created_at: "2026-01-02T10:00:00Z",
  },
];

async function emptyState(page, selector) {
  const block = page.locator(`${selector} .empty-state`);
  await block.waitFor({ state: "visible" });
  return {
    title: await block.locator(".empty-state-title").textContent(),
    hint: await block.locator(".empty-state-hint").textContent(),
  };
}

test("U-ST1: leere Bibliothek zeigt Hinweis mit Handlungsanweisung", async () => {
  await withApp(
    async (page) => {
      const state = await emptyState(page, "#videoList");
      assert.equal(state.title, "Noch keine Videos");
      assert.match(state.hint, /YouTube-URL/);
      assert.equal(await page.locator("#detailPlaceholder .empty-state-title").textContent(), "Kein Video ausgewählt");
    },
    { fixtures: { videos: [] } },
  );
});

test("U-ST2: Suche ohne Treffer und leere Sammlung zeigen eigene Hinweise", async () => {
  await withApp(
    async (page) => {
      await page.fill("#videoSearchInput", "gibtesnicht");
      let state = await emptyState(page, "#videoList");
      assert.equal(state.title, "Keine passenden Videos");
      assert.match(state.hint, /Suchbegriff oder Filter/);

      await page.fill("#videoSearchInput", "");
      await page.click('.collection-item[data-collection-id="1"]');
      state = await emptyState(page, "#videoList");
      assert.equal(state.title, "Diese Sammlung ist leer");
    },
    { fixtures: { collections: COLLECTIONS } },
  );
});

test("U-ST3: Video ohne Transkript, Zusammenfassung und Chats zeigt je einen Hinweis", async () => {
  await withApp(async (page, mock) => {
    await page.click('.video-item[data-id="1"]');
    await mock.waitForPending();
    let state = await emptyState(page, "#tabTranscript");
    assert.equal(state.title, "Kein Transkript vorhanden");
    assert.match(state.hint, /Transkript laden/);

    await page.click('.tab[data-tab="summary"]');
    state = await emptyState(page, "#summaryBody");
    assert.equal(state.title, "Noch keine Zusammenfassung");
    assert.match(state.hint, /Zusammenfassen lassen/);

    await page.click('.tab[data-tab="chat"]');
    state = await emptyState(page, "#chatMessages");
    assert.equal(state.title, "Noch keine Nachrichten");
  });
});

test("U-ST4: Tastaturfokus ist an jedem Element sichtbar", async () => {
  await withApp(async (page) => {
    await page.focus("#urlInput");
    const seen = [];
    for (let step = 0; step < 14; step += 1) {
      const info = await page.evaluate(() => {
        const el = document.activeElement;
        const style = getComputedStyle(el);
        return {
          name: `${el.tagName.toLowerCase()}#${el.id}.${el.className}`,
          outline: style.outlineStyle === "none" ? "none" : style.outlineWidth,
          shadow: style.boxShadow,
        };
      });
      seen.push(info.name);
      assert.ok(info.outline !== "none" || info.shadow !== "none", `kein Fokusring: ${info.name}`);
      await page.keyboard.press("Tab");
    }
    assert.ok(seen.some((name) => name.startsWith("button#settingsBtn")), seen.join(", "));
    assert.ok(seen.some((name) => name.includes("video-item")), seen.join(", "));
  });
});

test("U-ST5: gesperrte Knöpfe während setBusy sind gedimmt und zeigen den Fortschritts-Cursor", async () => {
  await withApp(
    async (page) => {
      await page.fill("#urlInput", "https://www.youtube.com/watch?v=neu");
      await page.click("#addBtn");
      await page.waitForFunction(() => document.querySelector("#addBtn").disabled);
      const style = await page.evaluate(() => {
        const button = getComputedStyle(document.querySelector("#addBtn"));
        return { opacity: Number(button.opacity), cursor: button.cursor, body: getComputedStyle(document.body).cursor };
      });
      assert.ok(style.opacity < 1);
      assert.equal(style.cursor, "progress");
      assert.equal(style.body, "progress");
    },
    { delays: { add_video: 1500 } },
  );
});

async function iconMetrics(page, selector) {
  return await page.evaluate((sel) => {
    return [...document.querySelectorAll(sel)].map((button) => {
      const b = button.getBoundingClientRect();
      const svg = button.querySelector("svg.icon");
      const i = svg.getBoundingClientRect();
      return {
        sel,
        label: button.getAttribute("aria-label"),
        hidden: svg.getAttribute("aria-hidden"),
        stroke: svg.getAttribute("stroke"),
        text: button.textContent.trim(),
        iconHeight: i.height,
        offsetY: Math.abs(i.top + i.height / 2 - (b.top + b.height / 2)),
        offsetX: Math.abs(i.left + i.width / 2 - (b.left + b.width / 2)),
      };
    });
  }, selector);
}

test("U-ST6: Icon-Knöpfe – Icon mittig, 14–20 px hoch, Label erhalten; Kopfleiste ohne Überlauf bei 900×600", async () => {
  await withApp(
    async (page, mock) => {
      await page.setViewportSize({ width: 900, height: 600 });
      await page.click('.video-item[data-id="2"]');
      await mock.waitForPending();
      await page.click('.tab[data-tab="summary"]');
      await page.locator("#summaryHistoryBar").waitFor({ state: "visible" });

      const selectors = [
        "#syncBtn",
        "#settingsBtn",
        "#addCollectionBtn",
        ".collection-action",
        "#deleteBtn",
        "#summaryHistoryDelete",
      ];
      for (const selector of selectors) {
        const list = await iconMetrics(page, selector);
        assert.ok(list.length > 0, `${selector} fehlt`);
        for (const m of list) {
          assert.ok(m.label, `${selector}: aria-label fehlt`);
          assert.equal(m.hidden, "true");
          assert.equal(m.stroke, "currentColor");
          assert.equal(m.text, "", `${selector}: Zeichen statt Icon`);
          assert.ok(m.iconHeight >= 14 && m.iconHeight <= 20, `${selector}: Höhe ${m.iconHeight}`);
          assert.ok(m.offsetY <= 2, `${selector}: vertikal ${m.offsetY}`);
          assert.ok(m.offsetX <= 2, `${selector}: horizontal ${m.offsetX}`);
        }
      }

      const header = await page.evaluate(() => {
        const el = document.querySelector("header");
        const settings = document.querySelector("#settingsBtn").getBoundingClientRect();
        return { scroll: el.scrollWidth, client: el.clientWidth, right: settings.right, width: window.innerWidth };
      });
      assert.ok(header.scroll <= header.client, `Kopfleiste läuft über: ${header.scroll} > ${header.client}`);
      assert.ok(header.right <= header.width, "Einstellungsknopf außerhalb des Fensters");
    },
    { fixtures: { collections: COLLECTIONS, summaries: SUMMARIES } },
  );
});

test("U-ST7: Live-Werkzeugschritte zeigen Status-Icons statt Zeichen, mit Klartext-Label", async () => {
  await withApp(
    async (page, mock) => {
      await page.click('.video-item[data-id="2"]');
      await mock.waitForPending();
      await page.click('.tab[data-tab="chat"]');
      await page.fill("#chatInput", "Frage");
      await page.click("#chatSend");
      await page.waitForFunction(() => window.__tauriMock.calls.some((call) => call.cmd === "chat_send"));
      const { requestId } = await page.evaluate(
        () => window.__tauriMock.calls.find((call) => call.cmd === "chat_send").args,
      );
      const tool = (payload) =>
        page.evaluate((data) => window.__tauriMock.emit("ai:chat_tool", data), {
          requestId,
          videoId: 2,
          round: 0,
          ...payload,
        });
      await tool({ kind: "search", label: "eins", status: "start" });
      await tool({ kind: "search", label: "eins", status: "ok" });
      await tool({ kind: "fetch", label: "example.com/a", status: "error" });
      await tool({ kind: "fetch", label: "example.com/b", status: "start" });
      await page.waitForFunction(() => document.querySelectorAll("#chatMessages .chat-tool-live-step").length === 3);

      const rows = await page.evaluate(() =>
        [...document.querySelectorAll("#chatMessages .chat-tool-step--live")].map((row) => {
          const marker = row.querySelector(".chat-tool-live-icon");
          const svg = marker.querySelector("svg.icon");
          const before = getComputedStyle(row.querySelector(".chat-tool-live-step"), "::before").content;
          return {
            role: marker.getAttribute("role"),
            label: marker.getAttribute("aria-label"),
            svgHidden: svg.getAttribute("aria-hidden"),
            stroke: svg.getAttribute("stroke"),
            icon: [...svg.classList].find((name) => name.startsWith("icon-")),
            before,
            text: row.querySelector(".chat-tool-live-step").textContent,
          };
        }),
      );
      assert.deepEqual(
        rows.map((row) => [row.icon, row.label, row.text]),
        [
          ["icon-check", "Erledigt", "Sucht: eins"],
          ["icon-warning", "Fehler", "Liest: example.com/a"],
          ["icon-dots", "Läuft", "Liest: example.com/b"],
        ],
      );
      for (const row of rows) {
        assert.equal(row.role, "img");
        assert.equal(row.svgHidden, "true");
        assert.equal(row.stroke, "currentColor");
        assert.equal(row.before, "none", "kein Zeichen mehr per ::before");
      }
    },
    { delays: { chat_send: { 2: 5000 } } },
  );
});
