import test from "node:test";
import assert from "node:assert/strict";
import { withApp } from "./harness.mjs";
import { defaultFixtures } from "./tauri-mock.mjs";

const AGENT = defaultFixtures.agent;

async function selectVideo(page, videoId = 2) {
  await page.waitForSelector(`.video-item[data-id="${videoId}"]`);
  await page.locator(`.video-item[data-id="${videoId}"]`).click();
  await page.waitForSelector("#agentHandoffBtn");
  await page.waitForFunction(() => !document.querySelector("#detailContent").hidden);
}

// Mock der Zwischenablage: `fail` laesst `writeText` ablehnen, damit der
// Rueckfallweg ueber ein unsichtbares Textfeld geprueft werden kann.
async function installClipboard(page, { fail = false, execCommand = true } = {}) {
  await page.evaluate(
    ({ fail, execCommand }) => {
      window.__copied = [];
      window.__fallbackText = null;
      window.__execCommandCalls = 0;
      Object.defineProperty(navigator, "clipboard", {
        configurable: true,
        value: {
          writeText: async (text) => {
            if (fail) throw new Error("clipboard verweigert");
            window.__copied.push(text);
          },
        },
      });
      const originalSelect = HTMLTextAreaElement.prototype.select;
      HTMLTextAreaElement.prototype.select = function select() {
        if (this.id !== "agentCommand") {
          window.__fallbackText = this.value;
        }
        return originalSelect.call(this);
      };
      document.execCommand = () => {
        window.__execCommandCalls += 1;
        return execCommand;
      };
    },
    { fail, execCommand },
  );
}

async function openAgentDialog(page, mock, videoId = 2) {
  await selectVideo(page, videoId);
  await mock.waitForPending();
  await page.locator("#agentHandoffBtn").click();
  await mock.waitForPending();
}

async function openAgentSettings(page, mock) {
  await page.locator("#settingsBtn").click();
  await page.locator("#settings-tab-agent").click();
  await mock.waitForPending();
}

function callsOf(calls, cmd) {
  return calls.filter((call) => call.cmd === cmd);
}

test("G1: Klick ruft agent_prepare, kopiert und zeigt Dialog samt Erfolgsstatus", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);

    const calls = await mock.getCalls();
    const prepare = callsOf(calls, "agent_prepare");
    assert.strictEqual(prepare.length, 1, "genau ein agent_prepare");
    assert.strictEqual(prepare[0].args.videoId, 2);
    assert.strictEqual(prepare[0].args.templateId, null);

    assert.deepEqual(
      await page.evaluate(() => window.__copied),
      [AGENT.prepare.command],
      "das Kommando muss kopiert werden",
    );

    assert.strictEqual(await page.locator("#agentModal").isVisible(), true);
    assert.strictEqual(
      await page.locator("#agentCommand").inputValue(),
      AGENT.prepare.command,
      "das Kommandofeld zeigt das Kommando",
    );
    assert.strictEqual(await page.locator("#agentCommand").getAttribute("readonly"), "");
    assert.strictEqual(
      await page.locator("#agentCopyHint").textContent(),
      "Das Kommando wurde in die Zwischenablage kopiert; die App startet nichts selbst.",
      "Erfolgshinweis im Dialog",
    );

    const status = await page.locator("#statusText").textContent();
    assert.strictEqual(
      status,
      `Kommando kopiert – Kontext liegt in ${AGENT.prepare.workdir}`,
      `Status war: "${status}"`,
    );
  });
});

test("G2: abgelehntes writeText faellt auf das Textfeld zurueck", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page, { fail: true });
    await openAgentDialog(page, mock);

    assert.deepEqual(await page.evaluate(() => window.__copied), [], "writeText scheiterte");
    assert.strictEqual(await page.evaluate(() => window.__execCommandCalls), 1);
    assert.strictEqual(
      await page.evaluate(() => window.__fallbackText),
      AGENT.prepare.command,
      "der Rueckfall kopiert dasselbe Kommando",
    );

    assert.strictEqual(
      await page.locator("#statusText").textContent(),
      `Kommando kopiert – Kontext liegt in ${AGENT.prepare.workdir}`,
    );
    assert.strictEqual(await page.locator("#agentModal").isVisible(), true);
  });
});

test("G2b: beide Kopierwege scheitern – Dialog bleibt, Status ohne „kopiert“", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page, { fail: true, execCommand: false });
    await openAgentDialog(page, mock);

    assert.strictEqual(await page.locator("#agentModal").isVisible(), true);
    assert.strictEqual(
      await page.locator("#agentCommand").inputValue(),
      AGENT.prepare.command,
      "das Kommando ist im Dialog markierbar",
    );
    const status = await page.locator("#statusText").textContent();
    assert.ok(!status.includes("kopiert"), `Status darf kein Erfolg sein: "${status}"`);
    assert.strictEqual(
      status,
      `Kontext liegt in ${AGENT.prepare.workdir} – Kommando im Dialog markieren und kopieren`,
    );
    assert.strictEqual(
      await page.locator("#agentCopyHint").textContent(),
      "Kommando bitte markieren und kopieren; die App startet nichts selbst.",
      "Hinweiszeile darf keinen Kopiererfolg behaupten (H3)",
    );

    // Der Button „Kopieren“ behauptet weiterhin keinen Erfolg: beide Wege
    // scheitern auch dort.
    await page.locator("#agentCopy").click();
    await mock.waitForPending();
    assert.strictEqual(
      await page.locator("#agentCopyHint").textContent(),
      "Kommando bitte markieren und kopieren; die App startet nichts selbst.",
    );
  });
});

test("G3: Vorlagenwechsel im Dialog loest neu auf und kopiert erneut", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);

    await page.locator("#agentTemplate").selectOption("codex");
    await mock.waitForPending();

    const prepare = callsOf(await mock.getCalls(), "agent_prepare");
    assert.strictEqual(prepare.length, 2, "zwei Aufloesungen");
    assert.strictEqual(prepare[1].args.templateId, "codex");

    assert.strictEqual(
      await page.locator("#agentCommand").inputValue(),
      AGENT.prepareByTemplate.codex.command,
    );
    assert.deepEqual(await page.evaluate(() => window.__copied), [
      AGENT.prepare.command,
      AGENT.prepareByTemplate.codex.command,
    ]);
  });
});

test("G4: ungueltige eigene Vorlage zeigt den Backend-Fehlertext und speichert nichts", async () => {
  await withApp(async (page, mock) => {
    await openAgentSettings(page, mock);

    await page.locator("#agentTemplateNew").click();
    await page.locator("#agentTemplateId").fill("mein-agent");
    await page.locator("#agentTemplateName").fill("Mein Agent");
    await page.locator("#agentTemplateCommand").fill("echo hallo");
    await mock.waitForPending();
    await page.locator("#agentTemplateSave").click();
    await mock.waitForPending();

    const error = page.locator("#agentTemplateError");
    assert.strictEqual(await error.isVisible(), true, "Fehlertext muss sichtbar sein");
    assert.strictEqual(
      await error.textContent(),
      "Die Vorlage nutzt keinen Kontext-Platzhalter",
    );
    // Der Editor bleibt offen, die Liste unveraendert.
    assert.strictEqual(await page.locator("#agentTemplateEdit").isVisible(), true);
    assert.strictEqual(
      await page.locator("#agentTemplateList").textContent(),
      "Keine eigenen Vorlagen",
    );
  });
});

test("G5: Vorschau aktualisiert sich beim Tippen", async () => {
  await withApp(async (page, mock) => {
    await openAgentSettings(page, mock);
    await page.locator("#agentTemplateNew").click();

    await page.locator("#agentTemplateCommand").fill("cd {workdir}");
    await mock.waitForPending();
    assert.strictEqual(await page.locator("#agentPreview").textContent(), "VORSCHAU cd {workdir}");

    await page.locator("#agentTemplateCommand").fill("cd {workdir} && pi {prompt}");
    await mock.waitForPending();
    assert.strictEqual(
      await page.locator("#agentPreview").textContent(),
      "VORSCHAU cd {workdir} && pi {prompt}",
    );

    const preview = callsOf(await mock.getCalls(), "agent_preview");
    assert.strictEqual(preview[preview.length - 1].args.command, "cd {workdir} && pi {prompt}");
    assert.strictEqual(preview[preview.length - 1].args.shell, "auto");
  });
});

test("G6: HTML-Zeichen im Kommando erscheinen als Text", async () => {
  const command = "<b>fett</b> & \"quote\"";
  const agent = {
    ...AGENT,
    prepare: { ...AGENT.prepare, command },
  };
  await withApp(
    async (page, mock) => {
      await installClipboard(page);
      await mock.setFixtures({ agent });
      await openAgentDialog(page, mock);

      assert.strictEqual(await page.locator("#agentCommand").inputValue(), command);
      assert.strictEqual(await page.locator("#agentModal b").count(), 0, "kein HTML-Element");
      assert.strictEqual(
        await page.locator("#agentContextPath").textContent(),
        AGENT.prepare.contextFile,
      );
    },
    { fixtures: { agent } },
  );
});

test("G7: „Ordner oeffnen“ ruft reveal_item_in_dir mit der Kontextdatei", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);

    await page.locator("#agentReveal").click();
    await mock.waitForPending();

    const reveal = callsOf(await mock.getCalls(), "plugin:opener|reveal_item_in_dir");
    assert.strictEqual(reveal.length, 1);
    assert.deepEqual(reveal[0].args.paths, [AGENT.prepare.contextFile]);
  });
});

test("G8: Escape schliesst den Dialog", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);
    assert.strictEqual(await page.locator("#agentModal").isVisible(), true);

    await page.keyboard.press("Escape");
    assert.strictEqual(await page.locator("#agentModal").isVisible(), false);
  });
});

test("G9: Button ist waehrend setBusy deaktiviert", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await selectVideo(page, 2);
    await mock.waitForPending();
    await mock.setDelays({ agent_prepare: 300 });

    await page.locator("#agentHandoffBtn").click();
    await page.waitForFunction(() => document.querySelector("#agentHandoffBtn").disabled);
    assert.strictEqual(await page.locator("#agentHandoffBtn").isDisabled(), true);

    await mock.waitForPending();
    assert.strictEqual(await page.locator("#agentHandoffBtn").isDisabled(), false);
  });
});

test("G10: Videowechsel waehrend agent_prepare verwirft das Ergebnis", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await selectVideo(page, 2);
    await mock.waitForPending();
    await mock.setDelays({ agent_prepare: { 2: 300 } });

    await page.locator("#agentHandoffBtn").click();
    // Waehrend der Vorbereitung auf Video 3 wechseln.
    await page.locator('.video-item[data-id="3"]').click();
    await mock.waitForPending();

    assert.strictEqual(await page.locator("#agentModal").isVisible(), false, "kein Dialog");
    assert.deepEqual(
      await page.evaluate(() => window.__copied),
      [],
      "es darf nichts kopiert werden",
    );
    const status = await page.locator("#statusText").textContent();
    assert.ok(!status.includes("Kontext liegt in"), `kein Handoff-Status: "${status}"`);
  });
});

test("G11: nur die juengste Vorlagen-Antwort aktualisiert Feld und Zwischenablage", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);
    assert.strictEqual(await page.locator("#agentCommand").inputValue(), AGENT.prepare.command);

    // Vorlage A (codex) ist langsam, Vorlage B (claude) schnell.
    await mock.setFixtures({ agent: { ...AGENT, prepareDelays: { codex: 300 } } });
    await page.locator("#agentTemplate").selectOption("codex");
    await page.locator("#agentTemplate").selectOption("claude");
    await mock.waitForPending();

    const claude = AGENT.prepareByTemplate.claude.command;
    const codex = AGENT.prepareByTemplate.codex.command;
    assert.strictEqual(
      await page.locator("#agentCommand").inputValue(),
      claude,
      "das Feld zeigt die juengste Antwort",
    );
    const copied = await page.evaluate(() => window.__copied);
    assert.strictEqual(copied[copied.length - 1], claude, "zuletzt kopiert wird die juengste Antwort");
    assert.ok(!copied.includes(codex), `die ueberholte Antwort darf nichts kopieren: ${JSON.stringify(copied)}`);
  });
});

// ---------------------------------------- Revision 3: Kontextauswahl (G12-G20) --
// Die bereits vorhandenen Faelle G12-G17 (Standard-Prompt, Vorlagenformular,
// Fehlerstatus, Einstellungen, Dialog beim Videowechsel) heissen seit Revision 3
// G21-G26; ihre Erwartungen sind unveraendert.

function lastPrepare(calls) {
  const prepare = callsOf(calls, "agent_prepare");
  return prepare[prepare.length - 1];
}

function charsText(chars) {
  return ` · ≈ ${chars.toLocaleString("de-DE")} Zeichen`;
}

test("G12: Dialog öffnen zeigt die Vorbelegung im Kontextbereich", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);

    const prepare = callsOf(await mock.getCalls(), "agent_prepare");
    assert.strictEqual(prepare.length, 1);
    assert.strictEqual(prepare[0].args.selection, undefined, "erster Aufruf ohne selection");

    assert.strictEqual(await page.locator("#agentCtxTranscript").isChecked(), true);
    assert.strictEqual(await page.locator("#agentCtxTranscript").isDisabled(), false);
    assert.strictEqual(await page.locator("#agentCtxTranscriptLabel").textContent(), "Transkript");
    // „Neueste“ erscheint als angehakte neueste Version; Radios gibt es nicht.
    assert.strictEqual(prepare[0].result.selection.summaryIds, null);
    assert.strictEqual(await page.locator("#agentCtxSummary-302").isChecked(), true);
    assert.strictEqual(await page.locator("#agentCtxSummary-301").isChecked(), false);
    assert.strictEqual(await page.locator('#agentContext input[type="radio"]').count(), 0);
    assert.strictEqual(await page.locator("#agentCtxChat-401").isChecked(), false);
    assert.strictEqual(await page.locator("#agentCtxChat-402").isChecked(), false);
    // Beschriftung: 1 Nachricht im Singular, sonst Plural.
    const label401 = await page.locator("#agentCtxChat-401 + span").textContent();
    assert.ok(label401.endsWith("· 1 Nachricht"), `Label war: "${label401}"`);
    const label402 = await page.locator("#agentCtxChat-402 + span").textContent();
    assert.ok(label402.endsWith("· 4 Nachrichten"), `Label war: "${label402}"`);
    assert.strictEqual(
      await page.locator("#agentContextChars").textContent(),
      charsText(prepare[0].result.contextChars),
      "die Zeichenzahl der Antwort ist sichtbar",
    );
  });
});

test("G13: Transkript abwählen schreibt neu, kopiert aber nicht erneut", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);
    assert.strictEqual((await page.evaluate(() => window.__copied)).length, 1);

    await page.locator("#agentCtxTranscript").uncheck();
    await mock.waitForPending();

    const last = lastPrepare(await mock.getCalls());
    assert.deepEqual(last.args.selection, { transcript: false, summaryIds: null, chatIds: [] });
    assert.strictEqual(
      (await page.evaluate(() => window.__copied)).length,
      1,
      "eine Auswahländerung kopiert nicht erneut",
    );
    const status = await page.locator("#statusText").textContent();
    assert.ok(status.startsWith("Kontextdatei aktualisiert"), `Status war: "${status}"`);
    assert.ok(status.includes("Zeichen"), `Status war: "${status}"`);
    assert.strictEqual(await page.locator("#agentCtxTranscript").isChecked(), false);
    assert.strictEqual(
      await page.locator("#agentContextChars").textContent(),
      charsText(last.result.contextChars),
    );
  });
});

test("G14: Zweite Version anhaken, dann beide abwählen", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);

    await page.locator("#agentCtxSummary-301").check();
    await mock.waitForPending();
    await page.locator("#agentCtxSummary-301").uncheck();
    await mock.waitForPending();
    await page.locator("#agentCtxSummary-302").uncheck();
    await mock.waitForPending();

    const prepare = callsOf(await mock.getCalls(), "agent_prepare");
    const requested = prepare.slice(1).map((call) => call.args.selection.summaryIds);
    assert.deepEqual(requested, [[302, 301], [302], []], "angeforderte Auswahl");
    const last = prepare[prepare.length - 1];
    assert.deepEqual(last.result.selection.summaryIds, [], "wirksame Auswahl: keine");
    assert.strictEqual(await page.locator("#agentCtxSummary-301").isChecked(), false);
    assert.strictEqual(await page.locator("#agentCtxSummary-302").isChecked(), false);
  });
});

test("G15: Einen Chat anhaken sendet genau diese ID", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);

    await page.locator("#agentCtxChat-401").check();
    await mock.waitForPending();

    const last = lastPrepare(await mock.getCalls());
    assert.deepEqual(last.args.selection.chatIds, [401]);
    assert.strictEqual(await page.locator("#agentCtxChat-401").isChecked(), true);
    assert.strictEqual(await page.locator("#agentCtxChat-402").isChecked(), false);
  });
});

test("G16: Auswahl ändern, dann Vorlage wechseln kopiert mit der neuen Auswahl", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);

    await page.locator("#agentCtxTranscript").uncheck();
    await mock.waitForPending();
    const copiedBefore = (await page.evaluate(() => window.__copied)).length;

    await page.locator("#agentTemplate").selectOption("codex");
    await mock.waitForPending();

    const last = lastPrepare(await mock.getCalls());
    assert.strictEqual(last.args.templateId, "codex");
    assert.strictEqual(last.args.selection.transcript, false, "die geänderte Auswahl geht mit");
    assert.strictEqual(
      await page.locator("#agentCommand").inputValue(),
      AGENT.prepareByTemplate.codex.command,
    );
    const copied = await page.evaluate(() => window.__copied);
    assert.strictEqual(copied.length, copiedBefore + 1, "der Vorlagenwechsel kopiert erneut");
    assert.strictEqual(copied[copied.length - 1], AGENT.prepareByTemplate.codex.command);
  });
});

test("G17: Die Auswahl wird je Video gemerkt", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock, 2);
    await page.locator("#agentCtxTranscript").uncheck();
    await mock.waitForPending();
    await page.locator("#agentModalClose").click();

    await page.locator("#agentHandoffBtn").click();
    await mock.waitForPending();
    const reopened = lastPrepare(await mock.getCalls());
    assert.strictEqual(reopened.args.videoId, 2);
    assert.deepEqual(reopened.args.selection, { transcript: false, summaryIds: null, chatIds: [] });
    assert.strictEqual(await page.locator("#agentCtxTranscript").isChecked(), false);

    // Für ein anderes Video gibt es keine gemerkte Auswahl.
    await page.evaluate(() => document.querySelector('.video-item[data-id="3"]').click());
    await mock.waitForPending();
    await page.locator("#agentHandoffBtn").click();
    await mock.waitForPending();
    const other = lastPrepare(await mock.getCalls());
    assert.strictEqual(other.args.videoId, 3);
    assert.strictEqual(other.args.selection, undefined, "Vorbelegung des Backends");
  });
});

test("G18: Video ohne Transkript und ohne Zusammenfassung", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock, 1);

    const transcript = page.locator("#agentCtxTranscript");
    assert.strictEqual(await transcript.isDisabled(), true);
    assert.strictEqual(await transcript.isChecked(), false);
    assert.strictEqual(
      await page.locator("#agentCtxTranscriptLabel").textContent(),
      "Transkript (nicht vorhanden)",
    );
    assert.strictEqual(await page.locator("#agentCtxSummaries input").count(), 0);
    assert.ok(
      (await page.locator("#agentCtxSummaries").textContent()).includes(
        "Keine Zusammenfassung vorhanden",
      ),
    );
    assert.strictEqual(await page.locator("#agentCtxChats input").count(), 0);
    assert.ok((await page.locator("#agentCtxChats").textContent()).includes("Keine Chats vorhanden"));
  });
});

test("G19: Die Anzeige folgt der zweiten Auswahlanfrage", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    // Aufruf 1 (Dialog öffnen) ohne Verzögerung, Aufruf 2 (Transkript abwählen)
    // verzögert, Aufruf 3 (Chat anhaken) sofort.
    await mock.setDelays({ agent_prepare: [0, 300, 0] });
    await openAgentDialog(page, mock);

    await page.locator("#agentCtxTranscript").uncheck();
    await page.locator("#agentCtxChat-401").check();
    await mock.waitForPending();

    const last = lastPrepare(await mock.getCalls());
    assert.deepEqual(last.args.selection, { transcript: false, summaryIds: null, chatIds: [401] });
    assert.strictEqual(
      await page.locator("#agentContextChars").textContent(),
      charsText(last.result.contextChars),
      "die veraltete Antwort darf die Zeichenzahl nicht setzen",
    );
    assert.strictEqual(
      await page.locator("#agentCtxChat-401").isChecked(),
      true,
      "die veraltete Antwort darf die Häkchen nicht zurücksetzen",
    );
    assert.strictEqual(await page.locator("#agentCtxTranscript").isChecked(), false);
  });
});

test("G20: Einstellungen speichern includeTranscript", async () => {
  await withApp(async (page, mock) => {
    await openAgentSettings(page, mock);
    const checkbox = page.locator("#agentIncludeTranscript");
    assert.strictEqual(await checkbox.isChecked(), true, "Vorbelegung aus der Konfiguration");

    await checkbox.uncheck();
    await mock.waitForPending();

    const set = callsOf(await mock.getCalls(), "agent_config_set");
    assert.strictEqual(set[set.length - 1].args.config.includeTranscript, false);
  });
});

test("G27: Ein Fehler bei der Auswahländerung lässt die letzte gültige Auswahl stehen", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);
    // Der erste Aufruf gelingt; danach scheitert das Backend.
    await mock.setFixtures({
      agent: {
        ...AGENT,
        prepareErrorFrom: 1,
        prepareError: "Kontextdatei konnte nicht geschrieben werden",
      },
    });

    // `click` statt `uncheck`: die Anzeige wird gleich darauf zurückgesetzt.
    await page.locator("#agentCtxTranscript").click();
    await page.waitForFunction(
      () =>
        document.querySelector("#statusText").textContent ===
        "Kontextdatei konnte nicht geschrieben werden",
    );
    await mock.waitForPending();
    assert.strictEqual(
      await page.locator("#agentCtxTranscript").isChecked(),
      true,
      "die Häkchen fallen auf die letzte wirksame Auswahl zurück",
    );
    assert.strictEqual(await page.locator("#agentContextChars").textContent(), charsText(400));
    assert.strictEqual(
      (await page.evaluate(() => window.__copied)).length,
      1,
      "es wird nichts kopiert",
    );
  });
});

test("G28: Zwei schnelle Klicks auf Versionen gehen beide ein", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    // Aufruf 1 (Dialog öffnen) sofort, Aufruf 2 (301 an) verzögert, Aufruf 3 (302 ab) sofort.
    await mock.setDelays({ agent_prepare: [0, 300, 0] });
    await openAgentDialog(page, mock);

    await page.locator("#agentCtxSummary-301").check();
    await page.locator("#agentCtxSummary-302").uncheck();
    await mock.waitForPending();

    const last = lastPrepare(await mock.getCalls());
    assert.deepEqual(last.args.selection.summaryIds, [301], "der zweite Klick baut auf dem ersten auf");
    assert.deepEqual(last.result.selection.summaryIds, [301], "wirksame Auswahl");
    assert.strictEqual(await page.locator("#agentCtxSummary-301").isChecked(), true);
    assert.strictEqual(await page.locator("#agentCtxSummary-302").isChecked(), false);
  });
});

test("G29: Letzte Version abwählen und sofort Transkript abwählen", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    // Aufruf 1 (öffnen) sofort, 2 (302 ab) verzögert, 3 (Transkript) sofort.
    await mock.setDelays({ agent_prepare: [0, 300, 0] });
    await openAgentDialog(page, mock);

    await page.locator("#agentCtxSummary-302").uncheck();
    await page.locator("#agentCtxTranscript").uncheck();
    await mock.waitForPending();

    const last = lastPrepare(await mock.getCalls());
    assert.deepEqual(last.args.selection, { transcript: false, summaryIds: [], chatIds: [] });
    assert.strictEqual(await page.locator("#agentCtxSummary-302").isChecked(), false);
    assert.strictEqual(await page.locator("#agentCtxTranscript").isChecked(), false);
  });
});

test("G30: Version anhaken und sofort wieder abwählen", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await mock.setDelays({ agent_prepare: [0, 300, 0] });
    await openAgentDialog(page, mock);

    await page.locator("#agentCtxSummary-301").check();
    await page.locator("#agentCtxSummary-301").uncheck();
    await mock.waitForPending();

    const last = lastPrepare(await mock.getCalls());
    assert.deepEqual(last.args.selection.summaryIds, [302], "zurück beim angezeigten Ausgangsstand");
    assert.strictEqual(await page.locator("#agentCtxSummary-301").isChecked(), false);
    assert.strictEqual(await page.locator("#agentCtxSummary-302").isChecked(), true);
  });
});

test("G31: Die Vorbelegung folgt der Konfiguration", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await mock.setFixtures({
      agent: {
        ...AGENT,
        view: {
          ...AGENT.view,
          config: {
            ...AGENT.view.config,
            includeTranscript: false,
            summaries: "all",
            includeChats: true,
          },
        },
      },
    });
    await openAgentDialog(page, mock);

    const prepare = callsOf(await mock.getCalls(), "agent_prepare");
    assert.strictEqual(prepare[0].args.selection, undefined, "Vorbelegung kommt vom Backend");
    assert.strictEqual(await page.locator("#agentCtxTranscript").isChecked(), false);
    assert.strictEqual(await page.locator("#agentCtxSummary-301").isChecked(), true);
    assert.strictEqual(await page.locator("#agentCtxSummary-302").isChecked(), true);
    assert.strictEqual(await page.locator("#agentCtxChat-401").isChecked(), true);
    assert.strictEqual(await page.locator("#agentCtxChat-402").isChecked(), true);
  });
});

test("G32: Der Tastaturfokus bleibt im Kontextwähler", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);

    await page.locator("#agentCtxSummary-301").focus();
    await page.keyboard.press("Space");
    await mock.waitForPending();
    assert.strictEqual(
      await page.evaluate(() => document.activeElement?.id ?? null),
      "agentCtxSummary-301",
      "der Fokus bleibt nach Klick und Antwort beim Regler",
    );
    assert.strictEqual(await page.locator("#agentCtxSummary-301").isChecked(), true);

    await page.keyboard.press("Space");
    await mock.waitForPending();
    assert.strictEqual(
      await page.evaluate(() => document.activeElement?.id ?? null),
      "agentCtxSummary-301",
      "auch nach dem Abwählen",
    );
    assert.strictEqual(await page.locator("#agentCtxSummary-301").isChecked(), false);
  });
});

test("G33: Altbestand ohne Versionsliste zeigt nur „Aktuelle Zusammenfassung“", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    const entry = AGENT.context[2];
    await mock.setFixtures({
      agent: {
        ...AGENT,
        context: {
          ...AGENT.context,
          2: { ...entry, available: { ...entry.available, summaries: [] } },
        },
      },
    });
    await openAgentDialog(page, mock);

    const latest = page.locator("#agentCtxSummaryLatest");
    assert.strictEqual(await latest.isChecked(), true);
    await latest.uncheck();
    await mock.waitForPending();
    assert.deepEqual(lastPrepare(await mock.getCalls()).args.selection.summaryIds, []);
    await latest.check();
    await mock.waitForPending();
    assert.strictEqual(lastPrepare(await mock.getCalls()).args.selection.summaryIds, null);
  });
});

test("G34: Haken und Beschriftung stehen in einer Zeile", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);

    for (const id of ["agentCtxTranscript", "agentCtxSummary-302", "agentCtxChat-401"]) {
      const input = await page.locator(`#${id}`).boundingBox();
      const text = await page.locator(`#${id} + span`).boundingBox();
      const inputMiddle = input.y + input.height / 2;
      assert.ok(
        inputMiddle > text.y && inputMiddle < text.y + text.height,
        `${id}: Haken (${JSON.stringify(input)}) und Text (${JSON.stringify(text)}) auf einer Höhe`,
      );
      assert.ok(input.x + input.width <= text.x, `${id}: Haken steht links vom Text`);
    }
    if (process.env.AGENT_DIALOG_SCREENSHOT) {
      await page.locator("#agentModal .modal-content").screenshot({
        path: process.env.AGENT_DIALOG_SCREENSHOT,
      });
    }
  });
});

test("G35: Lange Chat-Titel bleiben einzeilig, die ganze Frage steht im Tooltip", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);

    const long = await page.locator("label:has(#agentCtxChat-402)").boundingBox();
    const short = await page.locator("label:has(#agentCtxChat-401)").boundingBox();
    assert.ok(
      Math.abs(long.height - short.height) < 1,
      `kein Zeilenumbruch: ${long.height} gegen ${short.height}`,
    );
    const context = await page.locator("#agentContext").boundingBox();
    assert.ok(long.x + long.width <= context.x + context.width, "die Zeile ragt nicht hinaus");

    assert.strictEqual(
      await page.locator("label:has(#agentCtxChat-402)").getAttribute("title"),
      "Überprüfe mal kurz im Internet, ob diese Harness tatsächlich so gut ist, wie im Video behauptet wird, und nenne Quellen.",
    );
    // Ohne `firstQuestion` trägt der Tooltip den Titel.
    assert.strictEqual(
      await page.locator("label:has(#agentCtxChat-401)").getAttribute("title"),
      "Chat Eins",
    );
    // Kurzes Datum mit Uhrzeit; Datum und Zahl bleiben trotz Kürzung sichtbar.
    const meta = page.locator("#agentCtxChat-402 + span .agent-context-meta");
    assert.match(await meta.textContent(), /· 02\.05\.2026 \d{2}:\d{2} · 4 Nachrichten$/);
    const metaBox = await meta.boundingBox();
    assert.ok(metaBox.x + metaBox.width <= context.x + context.width, "Metadaten sichtbar");
    // Versionen zeigen dasselbe kurze Format.
    assert.match(
      await page.locator("#agentCtxSummary-302 + span").textContent(),
      /^01\.03\.2026 \d{2}:\d{2} – /,
    );
  });
});

test("G21: Standard-Prompt ist als Platzhalter sichtbar und uebernehmbar", async () => {
  await withApp(async (page, mock) => {
    await openAgentSettings(page, mock);
    const prompt = page.locator("#agentPrompt");
    assert.strictEqual(await prompt.inputValue(), "", "das Feld ist zunaechst leer");
    assert.strictEqual(
      await prompt.getAttribute("placeholder"),
      AGENT.view.defaultPrompt,
      "der Standard-Prompt steht vollstaendig als Platzhalter",
    );
    assert.strictEqual(await prompt.getAttribute("rows"), "8", "Feldhoehe fuer den langen Text");

    await page.locator("#agentPromptUseDefault").click();
    await mock.waitForPending();
    assert.strictEqual(await prompt.inputValue(), AGENT.view.defaultPrompt);
    const set = callsOf(await mock.getCalls(), "agent_config_set");
    assert.strictEqual(set[set.length - 1].args.config.prompt, AGENT.view.defaultPrompt);
  });
});

test("G22: Das Vorlagenformular erscheint nur nach Anlegen oder Bearbeiten", async () => {
  await withApp(async (page, mock) => {
    await openAgentSettings(page, mock);
    assert.strictEqual(
      await page.locator("#agentTemplateEdit").isVisible(),
      false,
      "initial muss das Formular verborgen sein",
    );

    await page.locator("#agentTemplateNew").click();
    assert.strictEqual(await page.locator("#agentTemplateEdit").isVisible(), true);

    await page.locator("#agentTemplateCancel").click();
    assert.strictEqual(await page.locator("#agentTemplateEdit").isVisible(), false);
  });
});

test("G23: Fehler aus agent_prepare landet im Status", async () => {
  const agent = { ...AGENT, prepareError: "Vorlage nicht gefunden" };
  await withApp(
    async (page, mock) => {
      await installClipboard(page);
      await selectVideo(page, 2);
      await mock.waitForPending();
      await page.locator("#agentHandoffBtn").click();
      await mock.waitForPending();

      assert.strictEqual(await page.locator("#statusText").textContent(), "Vorlage nicht gefunden");
      assert.strictEqual(await page.locator("#agentModal").isVisible(), false);
      assert.deepEqual(await page.evaluate(() => window.__copied), []);
    },
    { fixtures: { agent } },
  );
});

test("G24: Einstellungen speichern Arbeitsverzeichnis und Shell", async () => {
  await withApp(async (page, mock) => {
    await openAgentSettings(page, mock);

    assert.strictEqual(await page.locator("#agentDefaultWorkdir").textContent(), "/home/user/yt-agent");
    assert.strictEqual(await page.locator("#agentWorkdirBase").inputValue(), "");

    await page.locator("#agentWorkdirBase").fill("/srv/yt agent");
    await page.locator("#agentWorkdirBase").blur();
    await mock.waitForPending();

    const set = callsOf(await mock.getCalls(), "agent_config_set");
    assert.strictEqual(set.length, 1);
    assert.strictEqual(set[0].args.config.workdirBase, "/srv/yt agent");
    assert.strictEqual(await page.locator("#statusText").textContent(), "Agent-Einstellungen gespeichert");
  });
});

test("G25: Ein Videowechsel schließt den offenen Übergabe-Dialog und verwirft die Übergabe", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);
    assert.strictEqual(await page.locator("#agentModal").isVisible(), true);

    // Das Modal überdeckt die Liste; der Wechsel kommt hier wie über die Tastatur zustande.
    await page.evaluate(() => document.querySelector('.video-item[data-id="3"]').click());
    await mock.waitForPending();

    assert.strictEqual(
      await page.locator("#agentModal").isVisible(),
      false,
      "der Dialog des alten Videos muss geschlossen sein",
    );
    assert.strictEqual(
      await page.locator("#agentCommand").inputValue(),
      "",
      "das Kommando des alten Videos darf nicht stehen bleiben",
    );
  });
});

test("G26: Erneutes Anzeigen desselben Videos lässt den Dialog offen", async () => {
  await withApp(async (page, mock) => {
    await installClipboard(page);
    await openAgentDialog(page, mock);
    await page.evaluate(() => document.querySelector('.video-item[data-id="2"]').click());
    await mock.waitForPending();
    assert.strictEqual(await page.locator("#agentModal").isVisible(), true);
    assert.strictEqual(await page.locator("#agentCommand").inputValue(), AGENT.prepare.command);
  });
});
