import { createServer } from "vite";
import { chromium } from "playwright-core";
import { createMockScript } from "./tauri-mock.mjs";

export async function withApp(fn, options = {}) {
  const port = Number(process.env.UI_TEST_PORT) || 5199;
  let server;
  let browser;
  try {
    server = await createServer({
      server: {
        port,
        strictPort: true,
      },
      logLevel: "error",
    });
    await server.listen();

    const chromiumPath = process.env.CHROMIUM_PATH || "/usr/bin/chromium";
    browser = await chromium.launch({
      executablePath: chromiumPath,
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
