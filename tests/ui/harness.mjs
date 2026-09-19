import { existsSync } from "node:fs";
import path from "node:path";
import { createServer } from "vite";
import { chromium } from "playwright-core";
import { createMockScript } from "./tauri-mock.mjs";

function browserCandidates() {
  if (process.platform === "win32") {
    const roots = [process.env.PROGRAMFILES, process.env["PROGRAMFILES(X86)"], process.env.LOCALAPPDATA];
    const apps = [
      ["Google", "Chrome", "Application", "chrome.exe"],
      ["Microsoft", "Edge", "Application", "msedge.exe"],
      ["Chromium", "Application", "chrome.exe"],
    ];
    return apps.flatMap((app) => roots.filter(Boolean).map((root) => path.join(root, ...app)));
  }
  if (process.platform === "darwin") {
    return [
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
      "/Applications/Chromium.app/Contents/MacOS/Chromium",
      "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ];
  }
  return [
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/snap/bin/chromium",
  ];
}

export function resolveBrowserPath() {
  if (process.env.CHROMIUM_PATH) return process.env.CHROMIUM_PATH;
  const candidates = browserCandidates();
  const found = candidates.find((candidate) => existsSync(candidate));
  if (!found) {
    throw new Error(
      `Kein Chromium-basierter Browser gefunden (geprüft: ${candidates.join(", ")}). CHROMIUM_PATH setzen.`,
    );
  }
  return found;
}

export async function withApp(fn, options = {}) {
  // Ohne festen UI_TEST_PORT weicht Vite bei belegtem Port auf den nächsten freien aus,
  // damit Testdateien parallel laufen können
  const fixedPort = Number(process.env.UI_TEST_PORT) || 0;
  let server;
  let browser;
  try {
    server = await createServer({
      server: {
        port: fixedPort || 5199,
        strictPort: fixedPort > 0,
      },
      logLevel: "error",
    });
    await server.listen();
    const port = server.httpServer.address().port;

    browser = await chromium.launch({
      executablePath: resolveBrowserPath(),
      headless: true,
      args: ["--no-sandbox", "--disable-setuid-sandbox"],
    });

    const context = await browser.newContext();
    const page = await context.newPage();

    const mockScript = createMockScript(options.fixtures, options.delays);
    await page.addInitScript({ content: mockScript });

    const mock = {
      async setDelays(delays) {
        await page.evaluate((d) => {
          window.__tauriMock.delays = d;
        }, delays);
      },
      async setFixtures(fixtures) {
        await page.evaluate((f) => {
          Object.assign(window.__tauriMock.fixtures, f);
        }, fixtures);
      },
      async waitForPending() {
        await page.waitForFunction(() => window.__tauriMock.pending === 0);
      },
      async getCalls() {
        return await page.evaluate(() => window.__tauriMock.calls);
      },
      async getPending() {
        return await page.evaluate(() => window.__tauriMock.pending);
      },
    };

    await page.goto(`http://localhost:${port}`);
    await page.waitForFunction(
      () =>
        window.__tauriMock &&
        window.__tauriMock.calls.some((c) => c.cmd === "get_videos") &&
        window.__tauriMock.pending === 0,
    );

    await fn(page, mock);
  } finally {
    if (browser) {
      await browser.close().catch(() => {});
    }
    if (server) {
      await server.close().catch(() => {});
    }
  }
}
