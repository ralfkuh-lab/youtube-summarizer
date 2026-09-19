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

test("G10: Fehler aus agent_prepare landet im Status", async () => {
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

test("G11: Einstellungen speichern Arbeitsverzeichnis und Shell", async () => {
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
