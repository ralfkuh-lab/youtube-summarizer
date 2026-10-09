// UI-Tests fuer den Settings-Tab "Websuche" (src/websearch-settings.ts,
// Panel in src/template.ts, Commands in src-tauri/src/websearch.rs).

import test from "node:test";
import assert from "node:assert/strict";
import { withApp } from "./harness.mjs";

const EMPTY_URL_HINT = "Bitte zuerst eine SearXNG-URL eintragen";
const ENABLED_CONFIG = { enabled: true, searxngUrl: "http://127.0.0.1:8080" };

async function openWebSearch(page, mock) {
  await page.locator("#settingsBtn").click();
  await page.locator("#settings-tab-websuche").click();
  await page.waitForFunction(() =>
    window.__tauriMock.calls.some((call) => call.cmd === "web_search_config_get"),
  );
  await mock.waitForPending();
  await page.locator("#settings-panel-websuche").waitFor({ state: "visible" });
}

async function invokeArgs(page, cmd) {
  return await page.evaluate(
    (name) => window.__tauriMock.calls.filter((call) => call.cmd === name).map((call) => call.args),
    cmd,
  );
}

async function waitForCall(page, cmd, count = 1) {
  await page.waitForFunction(
    ({ name, expected }) =>
      window.__tauriMock.calls.filter((call) => call.cmd === name).length >= expected,
    { name: cmd, expected: count },
  );
}

test("U-WS1: Der Tab laedt die gespeicherten Werte", async () => {
  await withApp(
    async (page, mock) => {
      await openWebSearch(page, mock);

      assert.equal(await page.locator("#webSearchEnabled").isChecked(), true);
      assert.equal(await page.locator("#webSearchUrl").inputValue(), "http://127.0.0.1:8080");
      assert.equal(await page.locator("#webSearchError").isVisible(), false);
      assert.equal(await page.locator("#webSearchTestResult").textContent(), "");
    },
    { fixtures: { webSearchConfig: { ...ENABLED_CONFIG } } },
  );
});

test("U-WS2: Aktivieren ohne URL zeigt den Hinweis und speichert trotzdem", async () => {
  await withApp(
    async (page, mock) => {
      await openWebSearch(page, mock);
      assert.equal(await page.locator("#webSearchEnabled").isChecked(), false);

      await page.locator("#webSearchEnabled").check();
      await waitForCall(page, "web_search_config_set");
      await mock.waitForPending();
      await page.locator("#webSearchError").waitFor({ state: "visible" });

      assert.equal(await page.locator("#webSearchError").textContent(), EMPTY_URL_HINT);
      assert.equal(await page.locator("#webSearchEnabled").isChecked(), true);
      assert.deepEqual((await invokeArgs(page, "web_search_config_set"))[0], {
        config: { enabled: true, searxngUrl: "" },
      });
    },
  );
});

test("U-WS3: URL und Schalter werden mit den Feldwerten gespeichert", async () => {
  await withApp(
    async (page, mock) => {
      await openWebSearch(page, mock);

      await page.locator("#webSearchEnabled").check();
      await waitForCall(page, "web_search_config_set");
      await mock.waitForPending();

      await page.locator("#webSearchUrl").fill("http://127.0.0.1:8080");
      await page.locator("#webSearchUrl").press("Tab");
      await waitForCall(page, "web_search_config_set", 2);
      await mock.waitForPending();
      await page.locator("#webSearchError").waitFor({ state: "hidden" });

      const saves = await invokeArgs(page, "web_search_config_set");
      assert.deepEqual(saves[1], { config: { enabled: true, searxngUrl: "http://127.0.0.1:8080" } });
      assert.equal(await page.locator("#webSearchEnabled").isChecked(), true);
      assert.equal(await page.locator("#webSearchUrl").inputValue(), "http://127.0.0.1:8080");
      assert.equal(await page.locator("#statusText").textContent(), "Websuche gespeichert");
    },
    { fixtures: { webSearchConfig: { enabled: false, searxngUrl: "" } } },
  );
});

test("U-WS4: Verbindung testen braucht eine URL und zeigt die Trefferzahl", async () => {
  await withApp(
    async (page, mock) => {
      await openWebSearch(page, mock);

      // Leeres Feld: kein Backend-Aufruf, Hinweis im Ergebnisfeld.
      await page.locator("#webSearchTest").click();
      await mock.waitForPending();
      assert.equal(await page.locator("#webSearchTestResult").textContent(), EMPTY_URL_HINT);
      assert.equal((await invokeArgs(page, "web_search_test")).length, 0);
      assert.equal(await page.evaluate(() => document.activeElement?.id), "webSearchUrl");

      await page.locator("#webSearchUrl").fill("http://127.0.0.1:8080");
      await page.locator("#webSearchTest").click();
      await waitForCall(page, "web_search_test");
      await mock.waitForPending();
      await page.waitForFunction(
        () => document.querySelector("#webSearchTestResult").textContent === "3 Treffer",
      );

      assert.deepEqual((await invokeArgs(page, "web_search_test"))[0], {
        url: "http://127.0.0.1:8080",
      });
      assert.equal(
        await page.locator("#webSearchTestResult").evaluate((el) =>
          el.classList.contains("settings-ai-error"),
        ),
        false,
        "Erfolg ist kein Fehler",
      );
    },
    { fixtures: { webSearchTest: { count: 3, error: null } } },
  );
});

test("U-WS5: Ungueltige URL und Verbindungsfehler landen in den Fehlerfeldern", async () => {
  await withApp(
    async (page, mock) => {
      await openWebSearch(page, mock);

      await page.locator("#webSearchUrl").fill("kein-url");
      await page.locator("#webSearchUrl").press("Tab");
      await waitForCall(page, "web_search_config_set");
      await mock.waitForPending();
      await page.locator("#webSearchError").waitFor({ state: "visible" });
      assert.equal(await page.locator("#webSearchError").textContent(), "Ungültige SearXNG-URL");

      await mock.setFixtures({ webSearchTest: { error: "SearXNG nicht erreichbar" } });
      await page.locator("#webSearchUrl").fill("http://127.0.0.1:8080");
      await page.locator("#webSearchTest").click();
      await waitForCall(page, "web_search_test");
      await mock.waitForPending();
      await page.waitForFunction(
        () => document.querySelector("#webSearchTestResult").textContent === "SearXNG nicht erreichbar",
      );

      assert.equal(
        await page.locator("#webSearchTestResult").evaluate((el) =>
          el.classList.contains("settings-ai-error"),
        ),
        true,
      );
    },
  );
});
