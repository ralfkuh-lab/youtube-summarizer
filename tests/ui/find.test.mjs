import test from "node:test";
import assert from "node:assert/strict";
import { withApp } from "./harness.mjs";
import { defaultFixtures } from "./tauri-mock.mjs";

const SUMMARY = "## Kernpunkte\n\nDas **Modell** lernt. Ein Modell ist gut.\n\nNoch ein modell am Ende.";

async function highlightSizes(page) {
  return page.evaluate(() => ({
    match: CSS.highlights.get("find-match")?.size ?? 0,
    current: CSS.highlights.get("find-current")?.size ?? 0,
  }));
}

async function openVideo(page, mock, id) {
  await page.waitForSelector(`.video-item[data-id="${id}"]`);
  await page.locator(`.video-item[data-id="${id}"]`).click();
  await mock.waitForPending();
}

test("F1: Strg+F sucht in der Zusammenfassung, F3/Umschalt+F3 springen, Esc schließt", async () => {
  const fixtures = JSON.parse(JSON.stringify(defaultFixtures));
  fixtures.videos[1].summary = SUMMARY;

  await withApp(
    async (page, mock) => {
      await openVideo(page, mock, 2);
      await page.locator('.tab[data-tab="summary"]').click();
      await page.waitForSelector("#summaryBody h2");

      await page.keyboard.press("Control+f");
      assert.equal(await page.locator("#findBar").isVisible(), true, "Suchleiste muss offen sein");
      assert.equal(await page.evaluate(() => document.activeElement?.id), "findInput");

      await page.keyboard.type("modell");
      assert.equal(await page.locator("#findCount").textContent(), "1/3", "drei Treffer, ohne Groß-/Kleinschreibung");
      assert.deepEqual(await highlightSizes(page), { match: 3, current: 1 });

      await page.keyboard.press("F3");
      assert.equal(await page.locator("#findCount").textContent(), "2/3");
      await page.keyboard.press("Shift+F3");
      await page.keyboard.press("Shift+F3");
      assert.equal(await page.locator("#findCount").textContent(), "3/3", "Umschalt+F3 läuft rückwärts um");
      await page.keyboard.press("Enter");
      assert.equal(await page.locator("#findCount").textContent(), "1/3", "Enter springt vorwärts um");

      await page.keyboard.press("Escape");
      assert.equal(await page.locator("#findBar").isVisible(), false);
      assert.deepEqual(await highlightSizes(page), { match: 0, current: 0 });
    },
    { fixtures },
  );
});

test("F2: Lupe öffnet die Suche, Tabwechsel sucht im Transkript weiter, Video-Tab hat keine Suche", async () => {
  await withApp(async (page, mock) => {
    await openVideo(page, mock, 2);
    await page.locator('.tab[data-tab="summary"]').click();

    await page.locator("#findToggle").click();
    await page.locator("#findInput").fill("transkript");
    assert.equal(await page.locator("#findCount").textContent(), "Keine Treffer");

    await page.locator('.tab[data-tab="transcript"]').click();
    assert.equal(await page.locator("#findCount").textContent(), "1/1", "Suchbegriff bleibt beim Tabwechsel");
    assert.equal(await page.inputValue("#findInput"), "transkript");

    await page.locator('.tab[data-tab="video"]').click();
    assert.equal(await page.locator("#findToggle").isDisabled(), true);
    assert.equal(await page.locator("#findInput").isDisabled(), true);
    assert.equal(await page.locator("#findCount").textContent(), "");
    assert.deepEqual(await highlightSizes(page), { match: 0, current: 0 });
  });
});

test("F3: Suche im Chat findet neue Nachrichten, ohne neu suchen zu müssen", async () => {
  await withApp(async (page, mock) => {
    await openVideo(page, mock, 2);
    await page.locator('.tab[data-tab="chat"]').click();
    await page.keyboard.press("Control+f");
    await page.locator("#findInput").fill("Mock-Modell");
    assert.equal(await page.locator("#findCount").textContent(), "Keine Treffer");

    await page.locator("#chatInput").fill("Frage");
    await page.locator("#chatInput").press("Enter");
    await mock.waitForPending();
    await page.waitForFunction(() => document.querySelector("#findCount")?.textContent === "1/1");
  });
});

test("F4: Strg+F tut nichts, solange ein Dialog offen ist", async () => {
  await withApp(async (page, mock) => {
    await openVideo(page, mock, 2);
    await page.locator("#summarizeBtn").click();
    await page.waitForSelector("#summaryModal:not([hidden])");
    await page.keyboard.press("Control+f");
    assert.equal(await page.locator("#findBar").isVisible(), false);
  });
});

test("F5: Treffer im Transkript liegen in einem eigenen span, nicht direkt in der Flex-Zeile", async () => {
  const fixtures = JSON.parse(JSON.stringify(defaultFixtures));
  fixtures.videos[1].transcript = JSON.stringify([
    { start: 0, time: "00:00", text: "Erste Zeile mit Suchwort" },
    { start: 5, time: "00:05", text: "Zweite Zeile, wieder Suchwort" },
  ]);

  await withApp(
    async (page, mock) => {
      await openVideo(page, mock, 2);
      await page.locator('.tab[data-tab="transcript"]').click();
      await page.waitForSelector("#tabTranscript .ts-line");
      await page.keyboard.press("Control+f");
      await page.keyboard.type("suchwort");
      assert.equal(await page.locator("#findCount").textContent(), "1/2");

      // WebKitGTK malt ::highlight() nicht in nackten Textknoten eines Flex-Containers
      const parents = await page.evaluate(() =>
        [...CSS.highlights.get("find-match")].map((range) => {
          const parent = range.startContainer.parentElement;
          return `${parent.tagName}/${getComputedStyle(parent).display}`;
        }),
      );
      assert.deepEqual(parents, ["SPAN/block", "SPAN/block"]);
    },
    { fixtures },
  );
});
