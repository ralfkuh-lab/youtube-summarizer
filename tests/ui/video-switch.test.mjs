import test from "node:test";
import assert from "node:assert/strict";
import { withApp } from "./harness.mjs";

test("Race Condition 1: Vertauschte Antwortreihenfolge beim Videowechsel", async () => {
  await withApp(async (page, mock) => {
    // Warten bis die initiale Liste gerendert ist
    await page.waitForSelector('.video-item[data-id="1"]');
    await page.waitForSelector('.video-item[data-id="2"]');

    // get_video_detail für 1 antwortet nach 300 ms, für 2 nach 20 ms
    await mock.setDelays({
      get_video_detail: { 1: 300, 2: 20 },
    });

    // Video 1 anklicken, sofort Video 2 anklicken
    await page.locator('.video-item[data-id="1"]').click();
    await page.locator('.video-item[data-id="2"]').click();

    // Warten bis alle Aufrufe beantwortet sind
    await mock.waitForPending();

    // Erwartung nach Abschluss aller Aufrufe:
    // #detailTitle zeigt den Titel von Video 2
    const detailTitle = (await page.locator("#detailTitle").textContent())?.trim();
    assert.strictEqual(
      detailTitle,
      "Video Zwei (mit Transkript)",
      "Detailansicht muss Video 2 zeigen",
    );

    // Listeneintrag von Video 2 trägt die Klasse active, der von Video 1 nicht
    const item1Class = await page.locator('.video-item[data-id="1"]').getAttribute("class");
    const item2Class = await page.locator('.video-item[data-id="2"]').getAttribute("class");
    assert.ok(item2Class?.includes("active"), "Listeneintrag Video 2 muss 'active' sein");
    assert.ok(!item1Class?.includes("active"), "Listeneintrag Video 1 darf nicht 'active' sein");
  });
});

test("Race Condition 2: Videowechsel während Transkript-Neuladen", async () => {
  await withApp(async (page, mock) => {
    await page.waitForSelector('.video-item[data-id="1"]');

    // Video 1 auswählen (schnell)
    await page.locator('.video-item[data-id="1"]').click();
    await mock.waitForPending();

    const initialTitle = (await page.locator("#detailTitle").textContent())?.trim();
    assert.strictEqual(initialTitle, "Video Eins (ohne Transkript)");

    // Auf Zusammenfassungs-Tab wechseln, um spätere ungewollte Aktivierung des Transkript-Tabs zu prüfen
    await page.locator('.tab[data-tab="summary"]').click();
    const isSummaryActiveBefore = await page
      .locator('.tab[data-tab="summary"]')
      .evaluate((el) => el.classList.contains("active"));
    assert.strictEqual(isSummaryActiveBefore, true);

    // refresh_transcript antwortet nach 300 ms mit Video 1 inklusive Transkript,
    // get_video_detail für Video 2 nach 20 ms
    await mock.setDelays({
      refresh_transcript: { 1: 300 },
      get_video_detail: { 2: 20 },
    });

    // „Transkript laden" klicken
    await page.locator("#reloadTranscriptBtn").click();

    // Währenddessen Video 2 anklicken
    await page.locator('.video-item[data-id="2"]').click();

    // Warten bis alle Aufrufe beantwortet sind
    await mock.waitForPending();

    // Erwartung nach Abschluss:
    // 1. #detailTitle zeigt Video 2
    const detailTitle = (await page.locator("#detailTitle").textContent())?.trim();
    assert.strictEqual(
      detailTitle,
      "Video Zwei (mit Transkript)",
      "Detailansicht muss Video 2 zeigen",
    );

    // 2. Transkript-Tab wurde nicht aktiviert (aktiver Tab bleibt Zusammenfassung)
    const isTranscriptTabActive = await page
      .locator('.tab[data-tab="transcript"]')
      .evaluate((el) => el.classList.contains("active"));
    assert.strictEqual(
      isTranscriptTabActive,
      false,
      "Transkript-Tab darf bei inaktivem Video nicht aktiviert werden",
    );

    const isSummaryTabActive = await page
      .locator('.tab[data-tab="summary"]')
      .evaluate((el) => el.classList.contains("active"));
    assert.strictEqual(
      isSummaryTabActive,
      true,
      "Zusammenfassungs-Tab muss aktiv geblieben sein",
    );

    // 3. Listeneintrag von Video 1 zeigt den Transkript-Chip als vorhanden (videos wurde aktualisiert)
    const chipHasAvailable = await page
      .locator('.video-item[data-id="1"] .status-chip')
      .first()
      .evaluate((el) => el.classList.contains("available"));
    assert.strictEqual(
      chipHasAvailable,
      true,
      "Video 1 Listeneintrag muss Transkript-Chip als vorhanden anzeigen",
    );
  });
});

test("Race Condition 3: Löschen während Videowechsel", async () => {
  await withApp(async (page, mock) => {
    await page.waitForSelector('.video-item[data-id="1"]');
    await page.waitForSelector('.video-item[data-id="2"]');

    // Video 1 auswählen
    await page.locator('.video-item[data-id="1"]').click();
    await mock.waitForPending();
    assert.strictEqual(
      (await page.locator("#detailTitle").textContent())?.trim(),
      "Video Eins (ohne Transkript)",
    );

    // delete_video für 1 mit 300 ms Verzögerung, get_video_detail für 2 mit 20 ms
    await mock.setDelays({
      delete_video: { 1: 300 },
      get_video_detail: { 2: 20 },
    });

    // Löschen auslösen (Klick auf #deleteBtn öffnet #confirmModal, Klick auf #confirmOk bestätigt)
    await page.locator("#deleteBtn").click();
    await page.waitForSelector("#confirmModal:not([hidden])");
    await page.locator("#confirmOk").click();

    // Sofort Video 2 anklicken
    await page.locator('.video-item[data-id="2"]').click();

    // Warten bis alle Aufrufe abgeschlossen sind
    await mock.waitForPending();

    // Erwartung: Detailansicht zeigt Video 2 und ist sichtbar
    const isDetailVisible = await page.locator("#detailContent").isVisible();
    assert.strictEqual(isDetailVisible, true, "Detailansicht muss sichtbar sein");

    const isPlaceholderVisible = await page.locator("#detailPlaceholder").isVisible();
    assert.strictEqual(isPlaceholderVisible, false, "Placeholder darf nicht sichtbar sein");

    const detailTitle = (await page.locator("#detailTitle").textContent())?.trim();
    assert.strictEqual(
      detailTitle,
      "Video Zwei (mit Transkript)",
      "Detailansicht muss Video 2 zeigen",
    );

    // Listeneintrag 1 ist weg
    const count1 = await page.locator('.video-item[data-id="1"]').count();
    assert.strictEqual(count1, 0, "Listeneintrag 1 muss gelöscht sein");

    // Eintrag 2 ist active
    const item2Class = await page.locator('.video-item[data-id="2"]').getAttribute("class");
    assert.ok(item2Class?.includes("active"), "Listeneintrag 2 muss 'active' sein");
  });
});

