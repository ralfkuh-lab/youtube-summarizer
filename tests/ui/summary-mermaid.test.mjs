import test from "node:test";
import assert from "node:assert/strict";
import { withApp } from "./harness.mjs";
import { defaultFixtures } from "./tauri-mock.mjs";

const MERMAID_SUMMARY = [
  "## Ablauf",
  "",
  "```mermaid",
  "flowchart LR",
  "    A[Rohtext-Eingabe] --> B[Strukturierte Eingabe]",
  "    B --> C[Wahrscheinlichkeitsverteilung]",
  "```",
].join("\n");

test("Mermaid-Flowchart in der Zusammenfassung zeigt die Knotenbeschriftungen", async () => {
  const fixtures = JSON.parse(JSON.stringify(defaultFixtures));
  fixtures.videos[1].summary = MERMAID_SUMMARY;

  await withApp(
    async (page, mock) => {
      await page.waitForSelector('.video-item[data-id="2"]');
      await page.locator('.video-item[data-id="2"]').click();
      await mock.waitForPending();
      await page.locator('.tab[data-tab="summary"]').click();

      await page.waitForSelector("#summaryBody .summary-mermaid svg");

      // Die Labels müssen den Sanitizer überleben (kein foreignObject-HTML, sondern SVG-Text)
      const svgText = (await page.locator("#summaryBody .summary-mermaid svg text").allTextContents()).join(" ");
      for (const label of ["Rohtext-Eingabe", "Strukturierte Eingabe", "Wahrscheinlichkeitsverteilung"]) {
        assert.ok(svgText.includes(label), `Diagramm muss das Label "${label}" enthalten, war: "${svgText}"`);
      }
    },
    { fixtures },
  );
});
