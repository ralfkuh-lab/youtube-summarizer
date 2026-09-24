import test from "node:test";
import assert from "node:assert/strict";
import { withApp } from "./harness.mjs";

function collections(count) {
  return Array.from({ length: count }, (_, index) => ({
    id: index + 1,
    name: `Sammlung ${String(index + 1).padStart(2, "0")}`,
    video_count: index % 5,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  }));
}

async function measure(page) {
  return await page.evaluate(() => {
    const panel = document.querySelector("#libraryPanel").getBoundingClientRect();
    const tools = document.querySelector(".collection-tools").getBoundingClientRect();
    const all = document.querySelector('#collectionList [data-collection-id="all"]').getBoundingClientRect();
    const scroll = document.querySelector(".collection-scroll");
    return {
      panelHeight: panel.height,
      toolsHeight: tools.height,
      allTop: all.top,
      allBottom: all.bottom,
      scrollTop: scroll.getBoundingClientRect().top,
      scrollClient: scroll.clientHeight,
      scrollHeight: scroll.scrollHeight,
    };
  });
}

test("U-CL1: viele Sammlungen belegen höchstens die halbe Seitenleiste und scrollen unter „Alle Videos“", async () => {
  await withApp(
    async (page) => {
      for (const height of [700, 1100]) {
        await page.setViewportSize({ width: 1280, height });
        await page.waitForTimeout(50);
        const m = await measure(page);
        assert.ok(m.toolsHeight <= m.panelHeight / 2 + 1, `Höhe ${height}: ${m.toolsHeight} > ${m.panelHeight / 2}`);
        assert.ok(m.scrollHeight > m.scrollClient, `Höhe ${height}: kein Scrollbereich`);
        assert.ok(m.allBottom <= m.scrollTop + 1, "„Alle Videos“ steht über dem Scrollbereich");
      }
      const small = await (async () => {
        await page.setViewportSize({ width: 1280, height: 700 });
        await page.waitForTimeout(50);
        return measure(page);
      })();
      const large = await (async () => {
        await page.setViewportSize({ width: 1280, height: 1100 });
        await page.waitForTimeout(50);
        return measure(page);
      })();
      assert.ok(large.scrollClient > small.scrollClient, "Bereich wächst mit der Fensterhöhe");
    },
    { fixtures: { collections: collections(20) } },
  );
});

test("U-CL2: wenige Sammlungen brauchen keinen Scrollbereich", async () => {
  await withApp(
    async (page) => {
      await page.setViewportSize({ width: 1280, height: 800 });
      const m = await measure(page);
      assert.equal(m.scrollHeight, m.scrollClient);
    },
    { fixtures: { collections: collections(3) } },
  );
});

test("U-CL3: Auswahl einer Sammlung weiter unten behält die Scrollposition", async () => {
  await withApp(
    async (page, mock) => {
      await page.setViewportSize({ width: 1280, height: 700 });
      await page.evaluate(() => {
        const scroll = document.querySelector(".collection-scroll");
        scroll.scrollTop = scroll.scrollHeight;
      });
      const before = await page.evaluate(() => document.querySelector(".collection-scroll").scrollTop);
      assert.ok(before > 0);
      await page.click('.collection-scroll [data-collection-id="20"].collection-item');
      await mock.waitForPending();
      const after = await page.evaluate(() => document.querySelector(".collection-scroll").scrollTop);
      assert.equal(after, before);
      assert.ok(await page.isVisible('.collection-row.active [data-collection-id="20"]'));
    },
    { fixtures: { collections: collections(20) } },
  );
});
