// UI-Tests fuer die Settings-Tabs "KI-Anbieter" und "KI-Modelle".
// Erwartungen stammen aus src/ai-config.ts, src/template.ts und den
// Backend-Commands in src-tauri/src/commands.rs bzw. src-tauri/src/ai/config.rs.

import test from "node:test";
import assert from "node:assert/strict";
import { withApp } from "./harness.mjs";

const CATALOG = {
  catalog: {
    openai: {
      id: "openai",
      name: "OpenAI",
      api: "https://api.openai.com/v1",
      doc: "https://platform.openai.com/docs",
      models: {
        "gpt-4o": {
          id: "gpt-4o",
          name: "GPT-4o",
          tool_call: true,
          limit: { context: 128000 },
          cost: { input: 2.5, output: 10 },
        },
        "o3-mini": {
          id: "o3-mini",
          name: "o3-mini",
          reasoning: true,
          tool_call: false,
          limit: { context: 200000 },
          cost: { input: 1.1, output: 4.4 },
        },
      },
    },
    anthropic: {
      id: "anthropic",
      name: "Anthropic",
      api: "https://api.anthropic.com",
      doc: "https://docs.anthropic.com",
      models: {
        "claude-sonnet-4": { id: "claude-sonnet-4", name: "Claude Sonnet 4", tool_call: true },
      },
    },
  },
  source: "snapshot",
  updatedAt: "2026-07-04",
};

const AI_CONFIG = {
  provider: {
    openai: { enabled: true, name: "OpenAI", whitelist: ["gpt-4o"] },
    anthropic: { enabled: false, whitelist: [] },
  },
  defaultModel: { provider: "openai", model: "gpt-4o" },
};

const CUSTOM_PROVIDER = {
  enabled: true,
  custom: true,
  name: "Lokales LLM",
  options: { baseURL: "http://127.0.0.1:11434/v1" },
  whitelist: [],
};

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function aiFixtures(overrides = {}) {
  return { catalog: clone(CATALOG), aiConfig: clone(AI_CONFIG), ...overrides };
}

function aiConfigWithCustom() {
  const config = clone(AI_CONFIG);
  config.provider["local-llm"] = clone(CUSTOM_PROVIDER);
  return config;
}

async function openSettings(page, mock, tab = "ki-anbieter") {
  await page.locator("#settingsBtn").click();
  if (tab !== "ki-anbieter") await page.locator(`#settings-tab-${tab}`).click();
  await page.waitForFunction(
    () =>
      window.__tauriMock.calls.some((call) => call.cmd === "ai_config_get") &&
      window.__tauriMock.calls.some((call) => call.cmd === "ai_catalog_get") &&
      window.__tauriMock.calls.some((call) => call.cmd === "ai_auth_status"),
  );
  await mock.waitForPending();
  await page.locator(`#settings-panel-${tab}`).waitFor({ state: "visible" });
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

async function queryIds(page, selector) {
  return await page.evaluate(
    (sel) => Array.from(document.querySelectorAll(sel)).map((el) => el.dataset.aiProviderId ?? el.dataset.aiModelProvider),
    selector,
  );
}

async function providerIds(page) {
  return await queryIds(page, "#ai-provider-list [data-ai-provider-id]");
}

async function modelProviderIds(page) {
  return await queryIds(page, "#ai-model-list [data-ai-model-provider]");
}

async function waitForIds(page, selector, expected) {
  await page.waitForFunction(
    ({ sel, want }) => {
      const got = Array.from(document.querySelectorAll(sel)).map(
        (el) => el.dataset.aiProviderId ?? el.dataset.aiModelProvider,
      );
      return got.length === want.length && got.every((value, index) => value === want[index]);
    },
    { sel: selector, want: expected },
  );
}

async function waitForText(page, selector, expected) {
  await page.waitForFunction(
    ({ sel, want }) => (document.querySelector(sel)?.textContent ?? "").includes(want),
    { sel: selector, want: expected },
  );
}

async function waitForChecked(page, selector, checked) {
  await page.waitForFunction(
    ({ sel, want }) => document.querySelector(sel)?.checked === want,
    { sel: selector, want: checked },
  );
}

test("U-AI1: Einstellungen oeffnen zeigt den Anbieter-Tab mit Katalogliste und Katalogstand", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);

      assert.equal(await page.locator("#settings-tab-ki-anbieter").getAttribute("aria-selected"), "true");
      assert.equal(await page.locator("#settings-panel-ki-anbieter").isVisible(), true);
      assert.equal(await page.locator("#settings-panel-ki-modelle").isVisible(), false);

      assert.deepEqual(await providerIds(page), ["openai", "anthropic"], "aktive zuerst, dann der Rest");
      assert.equal(await page.locator("#ai-provider-enabled-openai").isChecked(), true);
      assert.equal(await page.locator("#ai-provider-enabled-anthropic").isChecked(), false);
      assert.match(
        await page.locator('[data-ai-provider-id="openai"]').textContent(),
        /API: https:\/\/api\.openai\.com\/v1/,
      );
      assert.equal(
        await page.locator("#ai-catalog-updated").textContent(),
        "Katalogstand: 04.07.2026 (Snapshot)",
      );
      assert.equal(await page.locator("#ai-providers-error").isVisible(), false);
    },
    { fixtures: aiFixtures() },
  );
});

test("U-AI2: Anbietersuche filtert die Liste und meldet leere Treffer", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);

      await page.locator("#ai-provider-search").fill("anth");
      await waitForIds(page, "#ai-provider-list [data-ai-provider-id]", ["anthropic"]);

      await page.locator("#ai-provider-search").fill("OpenAI");
      await waitForIds(page, "#ai-provider-list [data-ai-provider-id]", ["openai"]);

      await page.locator("#ai-provider-search").fill("gibtsnicht");
      await waitForText(page, "#ai-provider-list", "Keine passenden Anbieter.");
      assert.deepEqual(await providerIds(page), []);

      await page.locator("#ai-provider-search").fill("");
      await waitForIds(page, "#ai-provider-list [data-ai-provider-id]", ["openai", "anthropic"]);
    },
    { fixtures: aiFixtures() },
  );
});

test("U-AI3: Anbieter aktivieren und deaktivieren ruft ai_provider_enable und wirkt auf die Modellliste", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);

      await page.locator("#ai-provider-enabled-anthropic").check({ force: true });
      await waitForCall(page, "ai_provider_enable");
      await mock.waitForPending();
      await waitForChecked(page, "#ai-provider-enabled-anthropic", true);
      assert.deepEqual((await invokeArgs(page, "ai_provider_enable"))[0], {
        providerId: "anthropic",
        enabled: true,
      });

      await page.locator("#settings-tab-ki-modelle").click();
      await waitForIds(page, "#ai-model-list [data-ai-model-provider]", ["anthropic", "openai"]);
      assert.deepEqual(await modelProviderIds(page), ["anthropic", "openai"], "beide Anbieter aktiv");

      await page.locator("#settings-tab-ki-anbieter").click();
      await page.locator("#ai-provider-enabled-openai").uncheck({ force: true });
      await waitForCall(page, "ai_provider_enable", 2);
      await mock.waitForPending();
      await waitForChecked(page, "#ai-provider-enabled-openai", false);
      assert.deepEqual((await invokeArgs(page, "ai_provider_enable"))[1], {
        providerId: "openai",
        enabled: false,
      });

      await page.locator("#settings-tab-ki-modelle").click();
      await waitForIds(page, "#ai-model-list [data-ai-model-provider]", ["anthropic"]);
    },
    { fixtures: aiFixtures() },
  );
});

test("U-AI4: Schluessel speichern ruft ai_auth_set und zeigt danach nur den Status", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);

      const authRow = '[data-ai-auth-provider="openai"]';
      assert.match(await page.locator(authRow).textContent(), /Schlüssel fehlt/);
      assert.equal(await page.locator("#ai-auth-remove-openai").isVisible(), false);
      assert.equal(await page.locator("#ai-auth-edit-openai").textContent(), "Schlüssel setzen");

      await page.locator("#ai-auth-edit-openai").click();
      await page.locator("#ai-auth-key-openai").waitFor({ state: "visible" });
      await page.locator("#ai-auth-key-openai").fill("geheim-123");
      await page.locator("#ai-auth-save-openai").click();
      await waitForCall(page, "ai_auth_set");
      await mock.waitForPending();
      await waitForText(page, authRow, "Schlüssel hinterlegt");

      assert.deepEqual((await invokeArgs(page, "ai_auth_set"))[0], {
        providerId: "openai",
        key: "geheim-123",
      });
      assert.equal(await page.locator("#ai-auth-edit-openai").textContent(), "Schlüssel ändern");
      assert.equal(await page.locator("#ai-auth-remove-openai").isVisible(), true);
      assert.equal(
        await page.locator("#ai-auth-key-openai").inputValue(),
        "",
        "Schluessel wird nach dem Speichern nicht angezeigt",
      );
      assert.equal(
        await page.evaluate(() => document.body.innerText.includes("geheim-123")),
        false,
        "der Schluessel steht nirgends im DOM",
      );
      assert.ok(
        (await invokeArgs(page, "ai_auth_status")).length >= 2,
        "Status wird nach dem Speichern erneut geladen",
      );
    },
    { fixtures: aiFixtures() },
  );
});

test("U-AI5: Leerer Schluessel wird ohne Command abgelehnt", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);

      await page.locator("#ai-auth-edit-openai").click();
      await page.locator("#ai-auth-save-openai").click();
      await page.locator("#ai-auth-error-openai").waitFor({ state: "visible" });
      await mock.waitForPending();

      assert.equal(
        await page.locator("#ai-auth-error-openai").textContent(),
        "Schlüssel darf nicht leer sein.",
      );
      assert.equal((await invokeArgs(page, "ai_auth_set")).length, 0, "kein ai_auth_set-Aufruf");
    },
    { fixtures: aiFixtures() },
  );
});

test("U-AI6: Schluessel entfernen ruft ai_auth_remove und setzt den Status zurueck", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);
      const authRow = '[data-ai-auth-provider="openai"]';
      assert.match(await page.locator(authRow).textContent(), /Schlüssel hinterlegt/);
      assert.equal(await page.locator("#ai-auth-remove-openai").isVisible(), true);

      await page.locator("#ai-auth-remove-openai").click();
      await waitForCall(page, "ai_auth_remove");
      await mock.waitForPending();
      await waitForText(page, authRow, "Schlüssel fehlt");

      assert.deepEqual((await invokeArgs(page, "ai_auth_remove"))[0], { providerId: "openai" });
      assert.equal(await page.locator("#ai-auth-remove-openai").isVisible(), false);
      assert.equal(await page.locator("#ai-auth-edit-openai").textContent(), "Schlüssel setzen");
    },
    { fixtures: aiFixtures({ aiAuthStatus: { openai: true } }) },
  );
});

test("U-AI7: Fehler von ai_provider_enable erscheint im Anbieter-Fehlerfeld und der Schalter bleibt aus", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);
      await mock.setFixtures({ commandErrors: { ai_provider_enable: "Anbieter nicht verfügbar" } });

      // click statt check: der Fehlerpfad rendert die Liste (und den Schalter) neu.
      await page.locator("#ai-provider-enabled-anthropic").click({ force: true });
      await waitForCall(page, "ai_provider_enable");
      await mock.waitForPending();
      await page.locator("#ai-providers-error").waitFor({ state: "visible" });

      assert.match(await page.locator("#ai-providers-error").textContent(), /Anbieter nicht verfügbar/);
      assert.equal(await page.locator("#ai-provider-enabled-anthropic").isChecked(), false);
      assert.deepEqual(await providerIds(page), ["openai", "anthropic"]);
    },
    { fixtures: aiFixtures() },
  );
});

test("U-AI8: Fehler von ai_auth_set erscheint im Fehlerfeld der Anbieterkarte", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);
      await mock.setFixtures({ commandErrors: { ai_auth_set: "Schlüssel abgelehnt" } });

      await page.locator("#ai-auth-edit-openai").click();
      await page.locator("#ai-auth-key-openai").fill("schlecht");
      await page.locator("#ai-auth-save-openai").click();
      await waitForCall(page, "ai_auth_set");
      await mock.waitForPending();
      await page.locator("#ai-auth-error-openai").waitFor({ state: "visible" });

      assert.match(await page.locator("#ai-auth-error-openai").textContent(), /Schlüssel abgelehnt/);
      assert.match(
        await page.locator('[data-ai-auth-provider="openai"]').textContent(),
        /Schlüssel fehlt/,
        "Status bleibt unveraendert",
      );
      assert.equal(await page.locator("#ai-auth-key-openai").inputValue(), "", "Feld wird geleert");
    },
    { fixtures: aiFixtures() },
  );
});

test("U-AI9: Custom-Provider-Dialog meldet fehlende Pflichtfelder und laesst sich abbrechen", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);

      await page.locator("#ai-custom-add").click();
      await page.locator("#ai-custom-dialog").waitFor({ state: "visible" });
      assert.equal(await page.locator("#ai-custom-title").textContent(), "Anbieter hinzufügen");
      assert.equal(
        await page.evaluate(() => document.activeElement?.id),
        "ai-custom-id",
        "Fokus steht im ersten Pflichtfeld",
      );

      await page.locator("#ai-custom-id").fill("local-llm");
      await page.locator("#ai-custom-base-url").fill("http://127.0.0.1:11434/v1");
      await page.locator("#ai-custom-save").click();
      await waitForCall(page, "ai_custom_upsert");
      await mock.waitForPending();
      await page.locator("#ai-custom-error").waitFor({ state: "visible" });

      assert.match(await page.locator("#ai-custom-error").textContent(), /darf nicht leer sein/);
      assert.equal(await page.locator("#ai-custom-dialog").isVisible(), true, "Dialog bleibt offen");
      assert.deepEqual(await providerIds(page), ["openai", "anthropic"], "kein neuer Anbieter");

      await page.locator("#ai-custom-cancel").click();
      await page.locator("#ai-custom-dialog").waitFor({ state: "hidden" });
      assert.equal(await page.locator("#ai-custom-error").isVisible(), false, "Fehler wird verworfen");
    },
    { fixtures: aiFixtures() },
  );
});

test("U-AI10: Gueltiger Custom-Provider ruft ai_custom_upsert und erscheint in der Liste", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);

      await page.locator("#ai-custom-add").click();
      await page.locator("#ai-custom-id").fill("local-llm");
      await page.locator("#ai-custom-name").fill("Lokales LLM");
      await page.locator("#ai-custom-base-url").fill("http://127.0.0.1:11434/v1");
      await page.locator("#ai-custom-save").click();
      await waitForCall(page, "ai_custom_upsert");
      await mock.waitForPending();
      await page.locator("#ai-custom-dialog").waitFor({ state: "hidden" });
      await waitForText(page, '[data-ai-provider-id="local-llm"]', "Lokales LLM");

      assert.deepEqual((await invokeArgs(page, "ai_custom_upsert"))[0], {
        definition: { id: "local-llm", name: "Lokales LLM", baseURL: "http://127.0.0.1:11434/v1" },
      });
      assert.equal((await invokeArgs(page, "ai_auth_set")).length, 0, "ohne Schluessel kein Auth-Aufruf");
      assert.ok((await providerIds(page)).includes("local-llm"));
      assert.match(
        await page.locator('[data-ai-provider-id="local-llm"]').textContent(),
        /API: http:\/\/127\.0\.0\.1:11434\/v1/,
      );
      assert.equal(await page.locator("#ai-provider-enabled-local-llm").isChecked(), true);
    },
    { fixtures: aiFixtures() },
  );
});

test("U-AI11: Bearbeiten eines Custom-Providers ist vorbefuellt und speichert Name sowie Schluessel", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);

      await page.locator("#ai-custom-edit-local-llm").click();
      await page.locator("#ai-custom-dialog").waitFor({ state: "visible" });
      assert.equal(await page.locator("#ai-custom-title").textContent(), "Anbieter bearbeiten");
      assert.equal(await page.locator("#ai-custom-name").inputValue(), "Lokales LLM");
      assert.equal(await page.locator("#ai-custom-base-url").inputValue(), "http://127.0.0.1:11434/v1");
      assert.equal(await page.locator("#ai-custom-id").isDisabled(), true, "ID bleibt gesperrt");

      await page.locator("#ai-custom-name").fill("Lokales LLM 2");
      await page.locator("#ai-custom-key").fill("lokal-key");
      await page.locator("#ai-custom-save").click();
      await waitForCall(page, "ai_auth_set");
      await mock.waitForPending();
      await page.locator("#ai-custom-dialog").waitFor({ state: "hidden" });
      await waitForText(page, '[data-ai-auth-provider="local-llm"]', "Schlüssel hinterlegt");

      assert.deepEqual((await invokeArgs(page, "ai_custom_upsert"))[0], {
        definition: {
          id: "local-llm",
          name: "Lokales LLM 2",
          baseURL: "http://127.0.0.1:11434/v1",
        },
      });
      assert.deepEqual((await invokeArgs(page, "ai_auth_set"))[0], {
        providerId: "local-llm",
        key: "lokal-key",
      });
      assert.match(
        await page.locator('[data-ai-provider-id="local-llm"]').textContent(),
        /Lokales LLM 2/,
      );
    },
    { fixtures: aiFixtures({ aiConfig: aiConfigWithCustom() }) },
  );
});

test("U-AI12: Custom-Provider loeschen ruft ai_custom_delete und entfernt die Karte", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);
      assert.ok((await providerIds(page)).includes("local-llm"));

      await page.locator("#ai-custom-delete-local-llm").click();
      await waitForCall(page, "ai_custom_delete");
      await mock.waitForPending();
      await waitForIds(page, "#ai-provider-list [data-ai-provider-id]", ["openai", "anthropic"]);

      assert.deepEqual((await invokeArgs(page, "ai_custom_delete"))[0], { id: "local-llm" });
    },
    { fixtures: aiFixtures({ aiConfig: aiConfigWithCustom() }) },
  );
});

test("U-AI13: Modell an- und abhaken ruft ai_model_toggle mit den erwarteten Argumenten", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock, "ki-modelle");

      assert.equal(await page.locator("#ai-model-toggle-openai-o3-mini").isChecked(), false);
      await page.locator("#ai-model-toggle-openai-o3-mini").check({ force: true });
      await waitForCall(page, "ai_model_toggle");
      await mock.waitForPending();
      await waitForChecked(page, "#ai-model-toggle-openai-o3-mini", true);
      assert.deepEqual((await invokeArgs(page, "ai_model_toggle"))[0], {
        providerId: "openai",
        modelId: "o3-mini",
        on: true,
      });

      await page.locator("#ai-model-toggle-openai-gpt-4o").uncheck({ force: true });
      await waitForCall(page, "ai_model_toggle", 2);
      await mock.waitForPending();
      await waitForChecked(page, "#ai-model-toggle-openai-gpt-4o", false);
      assert.deepEqual((await invokeArgs(page, "ai_model_toggle"))[1], {
        providerId: "openai",
        modelId: "gpt-4o",
        on: false,
      });
      await page.waitForFunction(
        () =>
          Array.from(document.querySelectorAll("#ai-default-model option")).map(
            (option) => option.textContent,
          ).join("|") === "(keins)|OpenAI — o3-mini",
      );
    },
    { fixtures: aiFixtures() },
  );
});

test("U-AI14: Modellzeilen zeigen Katalog-Badges", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock, "ki-modelle");

      const gpt = await page.locator('[data-ai-model-id="gpt-4o"]').textContent();
      assert.match(gpt, /GPT-4o/);
      assert.match(gpt, /Kontext 128k/);
      assert.match(gpt, /Tools/);
      assert.match(gpt, /\$2\.5\/\$10 je 1M/);
      assert.doesNotMatch(gpt, /Reasoning/);

      const o3 = await page.locator('[data-ai-model-id="o3-mini"]').textContent();
      assert.match(o3, /Reasoning/);
      assert.match(o3, /Kontext 200k/);
      assert.doesNotMatch(o3, /Tools/, "tool_call: false erzeugt kein Tools-Badge");

      assert.equal(
        await page.locator('[data-ai-model-provider="anthropic"]').count(),
        0,
        "inaktiver Anbieter fehlt",
      );
    },
    { fixtures: aiFixtures() },
  );
});

test("U-AI15: Standardmodell setzen ruft ai_default_model_set und laesst sich leeren", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock, "ki-modelle");

      const select = page.locator("#ai-default-model");
      assert.equal(await select.inputValue(), JSON.stringify(["openai", "gpt-4o"]));

      await select.selectOption({ label: "OpenAI — o3-mini" });
      await waitForCall(page, "ai_default_model_set");
      await mock.waitForPending();
      assert.deepEqual((await invokeArgs(page, "ai_default_model_set"))[0], {
        providerId: "openai",
        modelId: "o3-mini",
      });
      assert.equal(await select.inputValue(), JSON.stringify(["openai", "o3-mini"]));

      await select.selectOption("");
      await waitForCall(page, "ai_default_model_set", 2);
      await mock.waitForPending();
      assert.deepEqual((await invokeArgs(page, "ai_default_model_set"))[1], {
        providerId: null,
        modelId: null,
      });
      assert.equal(await select.inputValue(), "");
    },
    {
      fixtures: aiFixtures({
        aiConfig: {
          provider: { openai: { enabled: true, name: "OpenAI", whitelist: ["gpt-4o", "o3-mini"] } },
          defaultModel: { provider: "openai", model: "gpt-4o" },
        },
      }),
    },
  );
});

test("U-AI16: Katalog aktualisieren zeigt das neue Datum, Fehler landen im Fehlerfeld", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock, "ki-modelle");
      assert.equal(
        await page.locator("#ai-catalog-updated").textContent(),
        "Katalogstand: 04.07.2026 (Snapshot)",
      );

      await page.locator("#ai-catalog-refresh").click();
      await waitForCall(page, "ai_catalog_refresh");
      await mock.waitForPending();
      await waitForText(page, "#ai-catalog-updated", "28.05.2026 (Cache)");
      assert.equal(await page.locator("#ai-catalog-refresh").isEnabled(), true);
      assert.equal(
        await page.locator("#ai-catalog-refresh").textContent(),
        "Anbieter-/Modellkatalog aktualisieren",
      );

      await mock.setFixtures({ commandErrors: { ai_catalog_refresh: "models.dev nicht erreichbar" } });
      await page.locator("#ai-catalog-refresh").click();
      await waitForCall(page, "ai_catalog_refresh", 2);
      await mock.waitForPending();
      await page.locator("#ai-models-error").waitFor({ state: "visible" });

      assert.match(await page.locator("#ai-models-error").textContent(), /models\.dev nicht erreichbar/);
      assert.equal(
        await page.locator("#ai-catalog-updated").textContent(),
        "Katalogstand: 28.05.2026 (Cache)",
        "Stand bleibt unveraendert",
      );
      assert.equal(await page.locator("#ai-catalog-refresh").isEnabled(), true);
    },
    {
      fixtures: aiFixtures({
        catalogRefresh: { ...clone(CATALOG), source: "cache", updatedAt: "1780000000" },
      }),
    },
  );
});

test("U-AI17: Tastatur wechselt die Settings-Tabs", async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);

      await page.locator("#settings-tab-ki-anbieter").focus();
      await page.keyboard.press("ArrowRight");
      await page.locator("#settings-panel-ki-modelle").waitFor({ state: "visible" });
      assert.equal(await page.locator("#settings-tab-ki-modelle").getAttribute("aria-selected"), "true");
      assert.equal(await page.locator("#settings-panel-ki-anbieter").isVisible(), false);
      assert.equal(await page.evaluate(() => document.activeElement?.id), "settings-tab-ki-modelle");

      await page.keyboard.press("ArrowRight");
      await page.locator("#settings-panel-websuche").waitFor({ state: "visible" });
      assert.equal(await page.locator("#settings-tab-websuche").getAttribute("aria-selected"), "true");
      assert.equal(await page.evaluate(() => document.activeElement?.id), "settings-tab-websuche");

      await page.keyboard.press("ArrowLeft");
      await page.locator("#settings-panel-ki-modelle").waitFor({ state: "visible" });
      assert.equal(await page.locator("#settings-tab-ki-modelle").getAttribute("aria-selected"), "true");

      await page.keyboard.press("ArrowLeft");
      await page.locator("#settings-panel-ki-anbieter").waitFor({ state: "visible" });
      assert.equal(await page.locator("#settings-tab-ki-anbieter").getAttribute("aria-selected"), "true");
      assert.equal(await page.evaluate(() => document.activeElement?.id), "settings-tab-ki-anbieter");
    },
    { fixtures: aiFixtures() },
  );
});

// Befund 1: Custom-Provider werden erst im Backend validiert. Der Dialog ruft
// ai_custom_upsert also auch mit leerem Namen/leerer URL auf und zeigt erst
// danach die Fehlermeldung. Erwartet waere eine Pruefung vor dem Aufruf.
test("U-AI18: Pflichtfelder werden ohne Backend-Aufruf geprueft", { todo: true }, async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);

      await page.locator("#ai-custom-add").click();
      await page.locator("#ai-custom-id").fill("local-llm");
      await page.locator("#ai-custom-save").click();
      await mock.waitForPending();

      assert.equal(await page.locator("#ai-custom-error").isVisible(), true);
      assert.equal((await invokeArgs(page, "ai_custom_upsert")).length, 0, "kein ai_custom_upsert-Aufruf");
    },
    { fixtures: aiFixtures() },
  );
});

// Befund 2: #ai-custom-form haengt in bindAiConfigEvents UND in initSettingsAi
// je einen submit-Listener ein. Jedes Speichern ruft die Backend-Commands
// deshalb doppelt auf.
test("U-AI19: Ein Speichern ruft ai_custom_upsert genau einmal", { todo: true }, async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);

      await page.locator("#ai-custom-add").click();
      await page.locator("#ai-custom-id").fill("local-llm");
      await page.locator("#ai-custom-name").fill("Lokales LLM");
      await page.locator("#ai-custom-base-url").fill("http://127.0.0.1:11434/v1");
      await page.locator("#ai-custom-key").fill("lokal-key");
      await page.locator("#ai-custom-save").click();
      await mock.waitForPending();

      assert.equal((await invokeArgs(page, "ai_custom_upsert")).length, 1, "genau ein ai_custom_upsert-Aufruf");
      assert.equal((await invokeArgs(page, "ai_auth_set")).length, 1, "genau ein ai_auth_set-Aufruf");
    },
    { fixtures: aiFixtures() },
  );
});

// Befund 3: invokeUi loggt und zeigt String(error) statt errorMessage(error);
// die Fehlerfelder der KI-Tabs bekommen dadurch das Praefix "Error: ".
test("U-AI20: Fehlerfelder zeigen genau die Backend-Meldung", { todo: true }, async () => {
  await withApp(
    async (page, mock) => {
      await openSettings(page, mock);
      await mock.setFixtures({ commandErrors: { ai_provider_enable: "Anbieter nicht verfügbar" } });

      await page.locator("#ai-provider-enabled-anthropic").click({ force: true });
      await mock.waitForPending();

      assert.equal(await page.locator("#ai-providers-error").textContent(), "Anbieter nicht verfügbar");
    },
    { fixtures: aiFixtures() },
  );
});
