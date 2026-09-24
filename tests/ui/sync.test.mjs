import test from "node:test";
import assert from "node:assert/strict";
import { withApp } from "./harness.mjs";
import { defaultFixtures } from "./tauri-mock.mjs";

const TOKEN_PLACEHOLDER = "gespeichert – leer lassen, um es zu behalten";
const ENABLED_CONFIG = {
  enabled: true,
  serverUrl: "https://sync.example.org",
  hasToken: true,
  newVideosLocal: false,
};

function syncStatus(overrides = {}) {
  return {
    enabled: true,
    running: false,
    lastSuccessAt: null,
    lastError: null,
    pending: 0,
    unsendable: [],
    stopped: null,
    ...overrides,
  };
}

function videosWithLocalOnly(ids) {
  const local = new Set(ids);
  return defaultFixtures.videos.map((video) => ({ ...video, local_only: local.has(video.id) }));
}

async function emit(page, event, payload) {
  await page.evaluate(
    ({ event, payload }) => window.__tauriMock.emit(event, payload),
    { event, payload },
  );
}

async function waitForListener(page, event) {
  await page.waitForFunction(
    (name) =>
      window.__tauriMock.calls.some(
        (call) => call.cmd === "plugin:event|listen" && call.args?.event === name,
      ),
    event,
  );
}

async function callArgs(page, cmd) {
  return await page.evaluate(
    (name) => window.__tauriMock.calls.filter((call) => call.cmd === name).map((call) => call.args),
    cmd,
  );
}

async function openSyncTab(page) {
  await page.locator("#settingsBtn").click();
  await page.locator("#settings-tab-sync").click();
  await page.waitForFunction(() =>
    window.__tauriMock.calls.some((call) => call.cmd === "sync_config_get"),
  );
  await page.locator("#settings-panel-sync").waitFor({ state: "visible" });
}

test("U-SY1: Sync-Tab laedt und speichert; das Token wird nie angezeigt", async () => {
  await withApp(
    async (page, mock) => {
      await openSyncTab(page);

      assert.equal(await page.locator("#syncEnabled").isChecked(), true);
      assert.equal(await page.locator("#syncServerUrl").inputValue(), "https://sync.example.org");
      assert.equal(await page.locator("#syncNewVideosLocal").isChecked(), false);
      const token = page.locator("#syncToken");
      assert.equal(await token.inputValue(), "", "Token darf nicht im Feld stehen");
      assert.equal(await token.getAttribute("placeholder"), TOKEN_PLACEHOLDER);

      await page.locator("#syncServerUrl").fill("https://neu.example.org");
      await page.locator("#syncServerUrl").press("Tab");
      await page.waitForFunction(
        () => window.__tauriMock.calls.filter((call) => call.cmd === "sync_config_set").length === 1,
      );
      await mock.waitForPending();
      let setCalls = await callArgs(page, "sync_config_set");
      assert.equal(setCalls[0].config.serverUrl, "https://neu.example.org");
      assert.equal("token" in setCalls[0].config, false, "Leeres Tokenfeld wird nicht gesendet");

      await token.fill("geheim-123");
      await token.press("Tab");
      await page.waitForFunction(
        () => window.__tauriMock.calls.filter((call) => call.cmd === "sync_config_set").length === 2,
      );
      await mock.waitForPending();
      setCalls = await callArgs(page, "sync_config_set");
      assert.equal(setCalls[1].config.token, "geheim-123");
      assert.equal(await token.inputValue(), "", "Token wird nach dem Speichern nicht angezeigt");
      assert.equal(await token.getAttribute("placeholder"), TOKEN_PLACEHOLDER);
      assert.equal(await page.locator("#syncServerUrl").inputValue(), "https://neu.example.org");
    },
    { fixtures: { syncConfig: { ...ENABLED_CONFIG } } },
  );
});

test("U-SY2: Verbindung testen zeigt Erfolg und Fehler", async () => {
  await withApp(
    async (page, mock) => {
      await openSyncTab(page);

      await page.locator("#syncTest").click();
      await page.waitForFunction(() =>
        window.__tauriMock.calls.some((call) => call.cmd === "sync_test"),
      );
      await mock.waitForPending();
      let result = await page.locator("#syncTestResult").textContent();
      assert.match(result, /Verbindung erfolgreich/);
      assert.match(result, /01234567/);
      assert.equal(
        await page.locator("#syncTestResult").evaluate((el) => el.classList.contains("settings-ai-error")),
        false,
      );

      await mock.setFixtures({ syncTest: { error: "Server nicht erreichbar" } });
      await page.locator("#syncTest").click();
      await page.waitForFunction(
        () => window.__tauriMock.calls.filter((call) => call.cmd === "sync_test").length === 2,
      );
      await mock.waitForPending();
      result = await page.locator("#syncTestResult").textContent();
      assert.equal(result, "Server nicht erreichbar");
      assert.equal(
        await page.locator("#syncTestResult").evaluate((el) => el.classList.contains("settings-ai-error")),
        true,
      );
    },
    { fixtures: { syncConfig: { ...ENABLED_CONFIG }, syncTest: { datasetId: "0123456789abcdef", sameDataset: true } } },
  );
});

test("U-SY3: Status mit Fehler, unsendable und stopped dataset; Neu abgleichen erst nach Bestaetigung", async () => {
  await withApp(
    async (page, mock) => {
      await waitForListener(page, "sync://status");
      await openSyncTab(page);

      await emit(
        page,
        "sync://status",
        syncStatus({
          lastSuccessAt: "2026-09-24T12:00:00.000Z",
          lastError: "Netzwerk nicht erreichbar",
          pending: 2,
          unsendable: [{ entity: "video", reason: "zu gross" }],
        }),
      );
      await page.waitForFunction(() => !document.querySelector("#syncUnsendableList").hidden);

      const line = await page.locator("#syncStatusLine").textContent();
      assert.match(line, /Ausstehende Änderungen: 2/);
      assert.match(line, /Letzter Erfolg:/);
      assert.equal(await page.locator("#syncError").textContent(), "Netzwerk nicht erreichbar");
      assert.equal(await page.locator("#syncError").isVisible(), true);
      assert.deepEqual(await page.locator("#syncUnsendableList li").allTextContents(), [
        "video: zu gross",
      ]);

      await emit(page, "sync://status", syncStatus({ stopped: "dataset", pending: 1 }));
      await page.waitForFunction(() => !document.querySelector("#syncStoppedBlock").hidden);
      assert.equal(await page.locator("#syncStoppedBlock").isVisible(), true);

      await page.locator("#syncRebaseline").click();
      await page.locator("#confirmOk").waitFor({ state: "visible" });
      await page.locator("#confirmCancel").click();
      await mock.waitForPending();
      assert.equal((await callArgs(page, "sync_rebaseline")).length, 0, "Abbrechen darf nicht neu abgleichen");
      assert.equal(await page.locator("#syncStoppedBlock").isVisible(), true);

      await page.locator("#syncRebaseline").click();
      await page.locator("#confirmOk").waitFor({ state: "visible" });
      await page.locator("#confirmOk").click();
      await page.waitForFunction(() =>
        window.__tauriMock.calls.some((call) => call.cmd === "sync_rebaseline"),
      );
      await mock.waitForPending();
      assert.equal((await callArgs(page, "sync_rebaseline")).length, 1);
      assert.equal(await page.locator("#syncStoppedBlock").isVisible(), false);
    },
    { fixtures: { syncConfig: { ...ENABLED_CONFIG } } },
  );
});

test("U-SY4: Nur-lokal-Schalter bestaetigt und laesst sich abbrechen", async () => {
  await withApp(async (page, mock) => {
    await page.waitForSelector('.video-item[data-id="2"]');
    await page.locator('.video-item[data-id="2"]').click();
    await mock.waitForPending();

    const toggle = page.locator("#detailLocalOnly");
    assert.equal(await toggle.isChecked(), false);

    await toggle.check();
    await page.locator("#confirmOk").waitFor({ state: "visible" });
    assert.equal(
      await page.locator("#confirmMessage").textContent(),
      "Das Video wird vom Server entfernt. Andere Geräte behalten ihre Kopie als lokales Video.",
    );
    await page.locator("#confirmCancel").click();
    await mock.waitForPending();
    assert.equal(await toggle.isChecked(), false, "Abbrechen darf nichts umschalten");
    assert.equal((await callArgs(page, "video_set_local_only")).length, 0);

    await toggle.check();
    await page.locator("#confirmOk").waitFor({ state: "visible" });
    await page.locator("#confirmOk").click();
    await page.waitForFunction(() =>
      window.__tauriMock.calls.some((call) => call.cmd === "video_set_local_only"),
    );
    await mock.waitForPending();
    assert.deepEqual((await callArgs(page, "video_set_local_only"))[0], {
      videoId: 2,
      localOnly: true,
    });
    assert.equal(await toggle.isChecked(), true);
  });
});

test("U-SY5: Nur-lokal-Symbol und Filter in der Liste", async () => {
  await withApp(
    async (page) => {
      await page.waitForSelector('.video-item[data-id="1"]');
      const chip = page.locator('.video-item[data-id="1"] .local-chip');
      assert.equal(await chip.count(), 1);
      assert.equal(await chip.getAttribute("title"), "Nur lokal");
      assert.equal(await chip.getAttribute("aria-label"), "Nur lokal");
      assert.equal(await page.locator('.video-item[data-id="2"] .local-chip').count(), 0);

      await page.locator('.filter-chip[data-video-filter="local"]').click();
      assert.equal(await page.locator('.video-item[data-id="1"]').count(), 1);
      assert.equal(await page.locator('.video-item[data-id="2"]').count(), 0);
      assert.equal(await page.locator('.video-item[data-id="3"]').count(), 0);
    },
    { fixtures: { videos: videosWithLocalOnly([1]) } },
  );
});

test("U-SY6: Kopfleistenknopf zeigt ok, laeuft, Fehler und angehalten", async () => {
  await withApp(
    async (page) => {
      await waitForListener(page, "sync://status");
      const button = page.locator("#syncBtn");
      assert.equal(await button.textContent(), "✓");
      assert.match(await button.getAttribute("class"), /sync-btn--ok/);
      assert.match(await button.getAttribute("aria-label"), /Synchronisation aktiv/);

      await emit(page, "sync://status", syncStatus({ running: true }));
      await page.waitForFunction(() => document.querySelector("#syncBtn").textContent === "⟳");
      assert.match(await button.getAttribute("class"), /sync-btn--running/);
      assert.match(await button.getAttribute("aria-label"), /läuft/);

      await emit(page, "sync://status", syncStatus({ lastError: "401 Unauthorized" }));
      await page.waitForFunction(() => document.querySelector("#syncBtn").textContent === "⚠");
      assert.match(await button.getAttribute("class"), /sync-btn--error/);
      assert.match(await button.getAttribute("title"), /401 Unauthorized/);

      await emit(page, "sync://status", syncStatus({ stopped: "auth" }));
      await page.waitForFunction(() => document.querySelector("#syncBtn").textContent === "⏸");
      assert.match(await button.getAttribute("class"), /sync-btn--stopped/);
      assert.match(await button.getAttribute("aria-label"), /Anmeldung/);

      await button.click();
      await page.waitForFunction(() =>
        window.__tauriMock.calls.some((call) => call.cmd === "sync_now"),
      );
    },
    { fixtures: { syncConfig: { ...ENABLED_CONFIG } } },
  );
});

test("U-SY7: Kopfleistenknopf bei aus oeffnet den Sync-Tab", async () => {
  await withApp(async (page) => {
    const button = page.locator("#syncBtn");
    assert.equal(await button.textContent(), "⇅");
    assert.match(await button.getAttribute("class"), /sync-btn--off/);
    assert.match(await button.getAttribute("aria-label"), /Synchronisation aus/);

    await button.click();
    await page.locator("#settings-panel-sync").waitFor({ state: "visible" });
    assert.equal(await page.locator("#settings-tab-sync").getAttribute("aria-selected"), "true");
  });
});

test("U-SY8: sync://applied laedt die Liste neu und laesst die Detailansicht waehrend einer Anfrage in Ruhe", async () => {
  await withApp(
    async (page, mock) => {
      await waitForListener(page, "sync://applied");
      await page.waitForSelector('.video-item[data-id="2"]');
      await page.locator('.video-item[data-id="2"]').click();
      await mock.waitForPending();
      assert.equal((await page.locator("#detailTitle").textContent()).trim(), "Video Zwei (mit Transkript)");

      await mock.setDelays({ chat_send: 900 });
      await page.locator('.tab[data-tab="chat"]').click();
      await mock.waitForPending();
      await page.locator("#chatInput").fill("Was steht drin?");
      await page.locator("#chatSend").click();
      await page.waitForFunction(() =>
        window.__tauriMock.calls.some((call) => call.cmd === "chat_send"),
      );

      const renamed = videosWithLocalOnly([]).map((video) =>
        video.id === 2 ? { ...video, title: "Video Zwei (umbenannt)" } : video,
      );
      await mock.setFixtures({ videos: renamed });
      await emit(page, "sync://applied", { videoIds: [2], collections: false });

      await page.waitForFunction(
        () => window.__tauriMock.calls.filter((call) => call.cmd === "get_videos").length >= 2,
      );
      assert.equal(
        (await page.locator('.video-item[data-id="2"] .title').textContent()).trim(),
        "Video Zwei (umbenannt)",
        "Die Liste wird neu geladen",
      );
      assert.equal(
        (await page.locator("#detailTitle").textContent()).trim(),
        "Video Zwei (mit Transkript)",
        "Waehrend der Anfrage bleibt die Detailansicht stehen",
      );

      await mock.waitForPending();
      await page.waitForFunction(
        () => document.querySelector("#detailTitle").textContent.trim() === "Video Zwei (umbenannt)",
      );
      assert.equal(
        await page.locator('.video-item[data-id="2"]').evaluate((el) => el.classList.contains("active")),
        true,
        "Die Auswahl bleibt beim selben lokalen Video",
      );
    },
    { fixtures: { syncConfig: { ...ENABLED_CONFIG } } },
  );
});

test("U-SY9: sync://applied entfernt ein geloeschtes aktives Video wie lokal", async () => {
  await withApp(
    async (page, mock) => {
      await waitForListener(page, "sync://applied");
      await page.waitForSelector('.video-item[data-id="1"]');
      await page.locator('.video-item[data-id="1"]').click();
      await mock.waitForPending();
      assert.equal(await page.locator("#detailContent").isVisible(), true);

      await mock.setFixtures({
        videos: defaultFixtures.videos.filter((video) => video.id !== 1).map((video) => ({ ...video })),
      });
      await emit(page, "sync://applied", { videoIds: [1], collections: false });
      await page.waitForFunction(
        () => window.__tauriMock.calls.filter((call) => call.cmd === "get_videos").length >= 2,
      );
      await mock.waitForPending();

      await page.waitForFunction(() => document.querySelector("#detailContent").hidden);
      assert.equal(await page.locator("#detailPlaceholder").isVisible(), true);
      assert.equal(await page.locator(".video-item.active").count(), 0);
    },
    { fixtures: { syncConfig: { ...ENABLED_CONFIG } } },
  );
});

test("U-SY10: Layout von Sync-Tab und Kopfleiste", async () => {
  await withApp(async (page) => {
    await page.setViewportSize({ width: 900, height: 800 });
    await openSyncTab(page);

    const boxes = await page.evaluate(() => {
      const rect = (el) => {
        const r = el.getBoundingClientRect();
        return { x: r.x, y: r.y, width: r.width, height: r.height, right: r.right, bottom: r.bottom };
      };
      const row = (id) => {
        const input = document.querySelector(id);
        const label = input.closest("label");
        const text = input.nextElementSibling;
        return { input: rect(input), label: rect(label), text: rect(text) };
      };
      const panel = document.querySelector("#settings-panel-sync");
      return {
        enabled: row("#syncEnabled"),
        newVideos: row("#syncNewVideosLocal"),
        panelScrollWidth: panel.scrollWidth,
        panelClientWidth: panel.clientWidth,
        syncBtn: rect(document.querySelector("#syncBtn")),
        gearBtn: rect(document.querySelector("#settingsBtn")),
        bodyScrollWidth: document.body.scrollWidth,
      };
    });

    for (const [name, row] of Object.entries({ enabled: boxes.enabled, newVideos: boxes.newVideos })) {
      assert.ok(
        row.input.y >= row.label.y - 1 && row.input.bottom <= row.label.bottom + 1,
        `${name}: Checkbox liegt in der Labelzeile`,
      );
      assert.ok(
        row.input.bottom > row.text.y && row.input.y < row.text.bottom,
        `${name}: Checkbox und Beschriftung stehen in derselben Zeile`,
      );
      assert.ok(row.label.height <= 30, `${name}: Label ist einzeilig`);
    }
    assert.ok(boxes.panelScrollWidth <= boxes.panelClientWidth, "Sync-Panel laeuft nicht horizontal ueber");
    assert.ok(boxes.bodyScrollWidth <= 900, "Die Seite laeuft nicht horizontal ueber");
    assert.ok(boxes.syncBtn.right <= boxes.gearBtn.x + 1, "Sync-Knopf sitzt links neben dem Zahnrad");
    assert.ok(
      boxes.syncBtn.y < boxes.gearBtn.bottom && boxes.syncBtn.bottom > boxes.gearBtn.y,
      "Sync-Knopf und Zahnrad liegen auf einer Hoehe",
    );
  });
});

test("U-SY11: Pfeiltasten wechseln vom Tab Agent zu Sync und zurueck", async () => {
  await withApp(async (page) => {
    await page.locator("#settingsBtn").click();
    await page.locator("#settings-tab-agent").click();
    await page.locator("#settings-panel-agent").waitFor({ state: "visible" });

    await page.locator("#settings-tab-agent").press("ArrowRight");
    await page.locator("#settings-panel-sync").waitFor({ state: "visible" });
    assert.equal(await page.locator("#settings-tab-sync").getAttribute("aria-selected"), "true");
    assert.equal(await page.evaluate(() => document.activeElement?.id), "settings-tab-sync");

    await page.locator("#settings-tab-sync").press("ArrowLeft");
    await page.locator("#settings-panel-agent").waitFor({ state: "visible" });
    assert.equal(await page.locator("#settings-tab-agent").getAttribute("aria-selected"), "true");
    assert.equal(await page.evaluate(() => document.activeElement?.id), "settings-tab-agent");
  });
});
