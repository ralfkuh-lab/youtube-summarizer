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

test("G12: Standard-Prompt ist als Platzhalter sichtbar und uebernehmbar", async () => {
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

test("G13: Das Vorlagenformular erscheint nur nach Anlegen oder Bearbeiten", async () => {
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

test("G14: Fehler aus agent_prepare landet im Status", async () => {
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

test("G15: Einstellungen speichern Arbeitsverzeichnis und Shell", async () => {
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
