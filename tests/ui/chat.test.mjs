import test from "node:test";
import assert from "node:assert/strict";
import { withApp } from "./harness.mjs";

const CHAT_SEND_DELAY = 300;

async function openChat(page, videoId) {
  await page.waitForSelector(`.video-item[data-id="${videoId}"]`);
  await page.locator(`.video-item[data-id="${videoId}"]`).click();
  await page.locator('.tab[data-tab="chat"]').click();
  await page.waitForSelector("#chatInput");
}

async function sendQuestion(page, text) {
  await page.locator("#chatInput").fill(text);
  await page.locator("#chatInput").press("Enter");
}

async function chatSendCalls(page) {
  return page.evaluate(() =>
    window.__tauriMock.calls
      .filter((call) => call.cmd === "chat_send")
      .map((call) => ({ args: call.args, result: call.result })),
  );
}

async function emitChatStream(page, payload) {
  await page.evaluate((data) => window.__tauriMock.emit("ai:chat_stream", data), payload);
}

test("U1: Frage senden zeigt Frage und Antwort, leert die Eingabe und wählt den neuen Chat", async () => {
  await withApp(async (page, mock) => {
    await openChat(page, 2);
    await mock.waitForPending();

    await sendQuestion(page, "Was ist das für ein Video?");
    await mock.waitForPending();

    const userBubbles = await page.locator("#chatMessages .chat-row--user .chat-bubble").allTextContents();
    assert.deepEqual(userBubbles, ["Was ist das für ein Video?"], "Frage muss als Benutzerblase erscheinen");

    const answer = await page.locator("#chatMessages .chat-row--assistant .chat-bubble").first().textContent();
    assert.ok(answer?.includes("Antwort vom Mock-Modell"), `Antwortblase fehlt: "${answer}"`);

    assert.strictEqual(await page.locator("#chatInput").inputValue(), "", "#chatInput muss leer sein");

    const options = await page.locator("#chatSelect option").allTextContents();
    assert.ok(
      options.some((label) => label.includes("Was ist das für ein Video?")),
      `#chatSelect muss den neuen Chat enthalten, war: ${JSON.stringify(options)}`,
    );
  });
});

test("U2: Fehlgeschlagenes Senden räumt die Blasen ab und stellt die Frage wieder her", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();

      await sendQuestion(page, "Frage mit Fehler");
      await mock.waitForPending();

      const bubbles = await page.locator("#chatMessages .chat-bubble").count();
      assert.strictEqual(bubbles, 0, "Nach einem Fehler darf keine Blase übrig bleiben");
      assert.strictEqual(
        await page.locator("#chatInput").inputValue(),
        "Frage mit Fehler",
        "Der Fragetext muss wieder im Eingabefeld stehen",
      );
      const status = await page.locator("#statusText").textContent();
      assert.ok(status?.includes("Backend kaputt"), `Status muss den Fehler zeigen, war: "${status}"`);
    },
    { fixtures: { chat: { error: "Backend kaputt" } } },
  );
});

test("U3: Verzögerte Antwort für Video 2 erscheint nicht im Chat von Video 3", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();

      await sendQuestion(page, "Frage Zwei");
      await page.locator('.video-item[data-id="3"]').click();
      await page.locator('.tab[data-tab="chat"]').click();
      await mock.waitForPending();

      const bubbles = await page.locator("#chatMessages .chat-bubble").count();
      assert.strictEqual(bubbles, 0, "Video 3 darf keine Blase aus Video 2 zeigen");
      const text = await page.locator("#chatMessages").textContent();
      assert.ok(!text?.includes("Frage Zwei"), `Video 3 darf die Frage aus Video 2 nicht zeigen: "${text}"`);
    },
    { delays: { chat_send: { 2: CHAT_SEND_DELAY } } },
  );
});

test("U4: Zeitstempel in der Antwort ist ein Sprunglink in den Video-Tab", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();

      await sendQuestion(page, "Wann beginnt der Teil?");
      await mock.waitForPending();

      const link = page.locator('#chatMessages a[data-seek="65"]');
      assert.strictEqual(await link.count(), 1, "Zeitstempel [01:05] muss als Link vorliegen");

      await link.click();
      const videoTabActive = await page
        .locator('.tab[data-tab="video"]')
        .evaluate((el) => el.classList.contains("active"));
      assert.strictEqual(videoTabActive, true, "Klick auf den Zeitstempel muss den Video-Tab aktivieren");
    },
    { fixtures: { chat: { answer: "Der Teil beginnt bei [01:05] im Video." } } },
  );
});

test("U5: Ohne Transkript ist die Chat-Eingabe deaktiviert und der Hinweis sichtbar", async () => {
  await withApp(async (page, mock) => {
    await openChat(page, 1);
    await mock.waitForPending();

    assert.strictEqual(await page.locator("#chatInput").isDisabled(), true, "#chatInput muss deaktiviert sein");
    assert.strictEqual(await page.locator("#chatHint").isVisible(), true, "Der Hinweis muss sichtbar sein");
    const hint = await page.locator("#chatHint").textContent();
    assert.ok(hint?.includes("Für den Chat wird ein Transkript benötigt"), `unerwarteter Hinweis: "${hint}"`);
  });
});

test("U6: Die Benutzerfrage mit HTML erscheint wörtlich als Text", async () => {
  await withApp(async (page, mock) => {
    await openChat(page, 2);
    await mock.waitForPending();

    const question = '<img src=x onerror=alert(1)>';
    await sendQuestion(page, question);
    await mock.waitForPending();

    const userText = await page.locator("#chatMessages .chat-row--user .chat-bubble").first().textContent();
    assert.strictEqual(userText, question, "Die Frage muss wörtlich als Text erscheinen");
    assert.strictEqual(
      await page.locator("#chatMessages img").count(),
      0,
      "Aus der Frage darf kein img-Element entstehen",
    );
  });
});

test("U7: Stopp während der Anfrage ruft chat_cancel mit derselben requestId", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();

      await sendQuestion(page, "Lange Frage");
      await page.waitForFunction(() =>
        window.__tauriMock.calls.some((call) => call.cmd === "chat_send"),
      );

      assert.strictEqual(await page.locator("#chatSend").textContent(), "Stopp");
      await page.locator("#chatSend").click();
      await mock.waitForPending();

      const sent = (await chatSendCalls(page))[0];
      const cancel = await page.evaluate(() =>
        window.__tauriMock.calls.find((call) => call.cmd === "chat_cancel"),
      );
      assert.ok(cancel, "chat_cancel muss aufgerufen worden sein");
      assert.strictEqual(
        cancel.args.requestId,
        sent.args.requestId,
        "chat_cancel muss dieselbe requestId wie chat_send verwenden",
      );

      // Der Abbruch raeumt die provisorischen Blasen ab und stellt die Frage wieder her.
      assert.strictEqual(
        await page.locator("#chatMessages .chat-bubble").count(),
        0,
        "Nach dem Stopp darf keine Blase übrig bleiben",
      );
      assert.strictEqual(
        await page.locator("#chatInput").inputValue(),
        "Lange Frage",
        "Der Fragetext muss wieder im Eingabefeld stehen",
      );
      const status = await page.locator("#statusText").textContent();
      assert.ok(status?.includes("abgebrochen"), `Status muss den Abbruch zeigen, war: "${status}"`);
    },
    { delays: { chat_send: { 2: CHAT_SEND_DELAY } } },
  );
});

test("U8: HTML in der Assistentenantwort wird entschärft", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();

      await sendQuestion(page, "Frage Acht");
      await mock.waitForPending();

      assert.strictEqual(
        await page.locator("#chatMessages img[onerror]").count(),
        0,
        "Kein img mit onerror in der Antwort",
      );
      const answer = await page
        .locator("#chatMessages .chat-row--assistant .chat-bubble")
        .first()
        .textContent();
      assert.ok(answer?.includes("Hallo"), `Antworttext muss erhalten bleiben: "${answer}"`);
    },
    { fixtures: { chat: { answer: '<img src=x onerror=alert(1)> Hallo' } } },
  );
});

test("U9: Nach dem Zurückwechseln erscheint die Runde genau einmal", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();

      await sendQuestion(page, "Frage Neun");
      await page.locator('.video-item[data-id="3"]').click();
      await page.locator('.tab[data-tab="chat"]').click();
      await mock.waitForPending();

      await page.locator('.video-item[data-id="2"]').click();
      await page.locator('.tab[data-tab="chat"]').click();
      await mock.waitForPending();

      const userTexts = await page.locator("#chatMessages .chat-row--user .chat-bubble").allTextContents();
      assert.deepEqual(userTexts, ["Frage Neun"], "Die Frage darf genau einmal sichtbar sein");
      const answerCount = await page
        .locator("#chatMessages .chat-row--assistant .chat-bubble")
        .count();
      assert.strictEqual(answerCount, 1, "Die Antwort darf genau einmal sichtbar sein");
    },
    { delays: { chat_send: { 2: CHAT_SEND_DELAY } } },
  );
});

test("U10: Ein zweites Enter direkt nach dem ersten sendet nicht erneut", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();

      await page.locator("#chatInput").fill("Nur einmal");
      await page.locator("#chatInput").press("Enter");
      // Die Eingabe ist waehrend der Anfrage gesperrt; das zweite Enter wird
      // deshalb als Tastenereignis direkt auf das Feld gegeben.
      await page.locator("#chatInput").evaluate((element) => {
        element.dispatchEvent(
          new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }),
        );
      });
      await mock.waitForPending();

      const sends = await chatSendCalls(page);
      assert.strictEqual(sends.length, 1, "Es darf nur ein chat_send-Aufruf entstehen");
      const userBubbles = await page.locator("#chatMessages .chat-row--user .chat-bubble").count();
      assert.strictEqual(userBubbles, 1, "Es darf nur eine Benutzerblase entstehen");
    },
    { delays: { chat_send: { 2: CHAT_SEND_DELAY } } },
  );
});

test("U11: Der Strom von Chat A erscheint nicht in Chat B", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();

      // Chat B anlegen und abschließen
      await sendQuestion(page, "Frage B");
      await mock.waitForPending();
      const chatBId = (await chatSendCalls(page))[0].result.chat.id;

      // Chat A als neuen Chat starten (verzögert)
      await page.locator("#chatNew").click();
      await sendQuestion(page, "Frage A");
      await page.waitForFunction(() =>
        window.__tauriMock.calls.some((call) => call.cmd === "chat_send" && call.args.text === "Frage A"),
      );

      // Während A läuft zu Chat B wechseln
      await page.selectOption("#chatSelect", String(chatBId));
      await mock.waitForPending();

      const textInB = await page.locator("#chatMessages").textContent();
      assert.ok(!textInB?.includes("Frage A"), `Chat B darf keine Blase von A zeigen: "${textInB}"`);

      // Zurück zu A: dort muss die vollständige Runde stehen
      const chatAId = (await chatSendCalls(page))[1].result.chat.id;
      await page.selectOption("#chatSelect", String(chatAId));
      await mock.waitForPending();

      const userTexts = await page.locator("#chatMessages .chat-row--user .chat-bubble").allTextContents();
      assert.deepEqual(userTexts, ["Frage A"], "Chat A muss nach dem Zurückwechseln vollständig sein");
      const answerCount = await page
        .locator("#chatMessages .chat-row--assistant .chat-bubble")
        .count();
      assert.strictEqual(answerCount, 1, "Chat A muss genau eine Antwort zeigen");
    },
    { delays: { chat_send: { 2: CHAT_SEND_DELAY } } },
  );
});

test("U12: Die Modellwahl im Chat überlebt einen Neuladen", async () => {
  const fixtures = {
    aiConfig: {
      provider: {
        openai: { enabled: true, name: "OpenAI", whitelist: ["gpt-4o", "gpt-4o-mini"] },
      },
      defaultModel: { provider: "openai", model: "gpt-4o" },
    },
    catalog: {
      catalog: {
        openai: {
          id: "openai",
          name: "OpenAI",
          models: {
            "gpt-4o": { id: "gpt-4o", name: "GPT-4o" },
            "gpt-4o-mini": { id: "gpt-4o-mini", name: "GPT-4o mini" },
          },
        },
      },
      source: "cache",
      updatedAt: "2026-09-01T00:00:00Z",
    },
  };

  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();
      await page.waitForFunction(
        () => document.querySelector("#chatModel")?.options.length > 1,
        undefined,
        { timeout: 5000 },
      );
      const choice = JSON.stringify(["openai", "gpt-4o-mini"]);
      await page.selectOption("#chatModel", choice);

      await page.reload();
      await openChat(page, 2);
      await mock.waitForPending();
      await page.waitForFunction(
        () => document.querySelector("#chatModel")?.options.length > 1,
        undefined,
        { timeout: 5000 },
      );
      assert.strictEqual(
        await page.locator("#chatModel").inputValue(),
        choice,
        "#chatModel muss die letzte Wahl zeigen",
      );
      await mock.waitForPending();
    },
    { fixtures },
  );
});

test("U13: Ein Stream-Event mit fremder requestId lässt die Antwortblase unverändert", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();

      await sendQuestion(page, "Frage Dreizehn");
      await page.waitForFunction(() =>
        window.__tauriMock.calls.some((call) => call.cmd === "chat_send"),
      );
      const { requestId } = (await chatSendCalls(page))[0].args;

      await emitChatStream(page, { requestId, videoId: 2, text: "Teil" });
      await page.waitForFunction(() =>
        document
          .querySelector("#chatMessages .chat-row--assistant .chat-bubble")
          ?.textContent?.includes("Teil"),
      );

      await emitChatStream(page, { requestId: "fremde-anfrage", videoId: 2, text: "FREMD" });
      const answer = await page
        .locator("#chatMessages .chat-row--assistant .chat-bubble")
        .first()
        .textContent();
      assert.ok(answer?.includes("Teil"), `Antwortblase muss unverändert bleiben: "${answer}"`);
      assert.ok(!answer?.includes("FREMD"), `Fremdes Event darf nichts ändern: "${answer}"`);

      await mock.waitForPending();
    },
    { delays: { chat_send: { 2: CHAT_SEND_DELAY } } },
  );
});

test("U14: Eine vollständig in Code-Fences gehüllte Antwort bleibt ein Code-Block", async () => {
  const answer = "```\nHallo Code\n```";
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();

      await sendQuestion(page, "Frage Vierzehn");
      await mock.waitForPending();

      const code = page.locator("#chatMessages .chat-row--assistant pre code");
      assert.strictEqual(await code.count(), 1, "Der Code-Block muss sichtbar bleiben");
      assert.ok((await code.textContent())?.includes("Hallo Code"), "Der Code-Block muss den Text enthalten");
      const bubble = await page
        .locator("#chatMessages .chat-row--assistant .chat-bubble")
        .first()
        .textContent();
      assert.ok(!bubble?.includes("```"), `Fences dürfen nicht als Text erscheinen: "${bubble}"`);
    },
    { fixtures: { chat: { answer } } },
  );
});

test("U15: „Neuer Chat“ bleibt nach einem Tabwechsel erhalten", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();
      assert.notStrictEqual(
        await page.locator("#chatSelect").inputValue(),
        "",
        "Der gespeicherte Chat muss zunächst ausgewählt sein",
      );

      await page.locator("#chatNew").click();
      await mock.waitForPending();
      assert.strictEqual(
        await page.locator("#chatSelect").inputValue(),
        "",
        "„Neuer Chat“ muss die leere Auswahl zeigen",
      );

      await page.locator('.tab[data-tab="transcript"]').click();
      await page.locator('.tab[data-tab="chat"]').click();
      await mock.waitForPending();

      assert.strictEqual(
        await page.locator("#chatSelect").inputValue(),
        "",
        "#chatSelect muss auch nach dem Tabwechsel „Neuer Chat“ zeigen",
      );
      assert.strictEqual(
        await page.locator("#chatMessages .chat-row").count(),
        0,
        "Der neue Chat darf keine Blasen zeigen",
      );
    },
    {
      fixtures: {
        chatSeed: {
          2: [
            {
              title: "Alter Chat",
              messages: [
                { role: "user", content: "Alte Frage" },
                { role: "assistant", content: "Alte Antwort" },
              ],
            },
          ],
        },
      },
    },
  );
});

test("U16: Nach Rückkehr zeigt der neue Chat nur die laufende Frage", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();
      await page.locator("#chatNew").click();
      await sendQuestion(page, "Neuer Flug");
      await page.waitForFunction(() =>
        window.__tauriMock.calls.some((call) => call.cmd === "chat_send"),
      );

      await page.locator('.video-item[data-id="3"]').click();
      await page.locator('.tab[data-tab="chat"]').click();
      await page.locator('.video-item[data-id="2"]').click();
      await page.locator('.tab[data-tab="chat"]').click();

      const duringRun = await page.locator("#chatMessages").textContent();
      assert.ok(duringRun?.includes("Neuer Flug"), `Provisorische Blase fehlt: "${duringRun}"`);
      assert.ok(
        !duringRun?.includes("Alte Frage"),
        `Der Verlauf des alten Chats darf nicht erscheinen: "${duringRun}"`,
      );

      await mock.waitForPending();

      const userTexts = await page.locator("#chatMessages .chat-row--user .chat-bubble").allTextContents();
      assert.deepEqual(userTexts, ["Neuer Flug"], "Die Frage darf genau einmal erscheinen");
      assert.strictEqual(
        await page.locator("#chatMessages .chat-row--assistant .chat-bubble").count(),
        1,
        "Die Antwort darf genau einmal erscheinen",
      );
      const options = await page.locator("#chatSelect option").allTextContents();
      assert.ok(
        options.some((label) => label.includes("Neuer Flug")),
        `#chatSelect muss den neuen Chat enthalten: ${JSON.stringify(options)}`,
      );
      assert.notStrictEqual(await page.locator("#chatSelect").inputValue(), "");
    },
    {
      fixtures: {
        chatSeed: {
          2: [
            {
              title: "Alter Chat",
              messages: [
                { role: "user", content: "Alte Frage" },
                { role: "assistant", content: "Alte Antwort" },
              ],
            },
          ],
        },
      },
      // Grosszügig, damit der Ausflug zu Video 3 sicher vor dem Ablauf endet.
      delays: { chat_send: { 2: 1500 } },
    },
  );
});

test("U17: Ein Abschluss für ein anderes Video meldet keinen Erfolg", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();

      await sendQuestion(page, "Frage Siebzehn");
      await page.locator('.video-item[data-id="3"]').click();
      await mock.waitForPending();

      const status = await page.locator("#statusText").textContent();
      assert.ok(
        !status?.includes("Chat-Antwort fertig"),
        `Status darf den Abschluss von Video 2 nicht melden, war: "${status}"`,
      );
    },
    { delays: { chat_send: { 2: CHAT_SEND_DELAY } } },
  );
});

test("U18: Der Chat eines anderen Videos wird trotz laufender Anfrage vollständig aufgebaut", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();

      await sendQuestion(page, "Frage Achtzehn");
      await page.locator('.video-item[data-id="3"]').click();
      await page.locator('.tab[data-tab="chat"]').click();
      await mock.waitForPending();

      const options = await page.locator("#chatSelect option").allTextContents();
      assert.ok(
        options.some((label) => label.includes("Chat Drei")),
        `Die Chat-Liste von Video 3 fehlt: ${JSON.stringify(options)}`,
      );
      const text = await page.locator("#chatMessages").textContent();
      assert.ok(
        text?.includes("Frage Drei") && text?.includes("Antwort Drei"),
        `Der Verlauf von Video 3 fehlt: "${text}"`,
      );
    },
    {
      fixtures: {
        chatSeed: {
          3: [
            {
              title: "Chat Drei",
              messages: [
                { role: "user", content: "Frage Drei" },
                { role: "assistant", content: "Antwort Drei" },
              ],
            },
          ],
        },
      },
      delays: {
        chat_send: { 2: 150 },
        chat_list: { 3: 500 },
        chat_messages: { default: 500 },
      },
    },
  );
});

test("U19: Eine fehlgeschlagene Frage landet als Entwurf im richtigen Chat", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();

      await page.locator("#chatSelect").selectOption({ index: 1 });
      const chatBId = await page.locator("#chatSelect").inputValue();
      await mock.waitForPending();

      await page.locator("#chatSelect").selectOption({ index: 2 });
      const chatAId = await page.locator("#chatSelect").inputValue();
      await mock.waitForPending();
      assert.notStrictEqual(chatAId, chatBId, "Zwei unterschiedliche Chats erwartet");

      await sendQuestion(page, "Frage Neunzehn");
      await page.waitForFunction(() =>
        window.__tauriMock.calls.some((call) => call.cmd === "chat_send"),
      );

      // Während der Anfrage zu Chat B wechseln; dort schlägt sie fehl.
      await page.locator("#chatSelect").selectOption(chatBId);
      await mock.waitForPending();
      assert.strictEqual(
        await page.locator("#chatInput").inputValue(),
        "",
        "Der Entwurf darf nicht im falschen Chat landen",
      );

      await page.locator("#chatSelect").selectOption(chatAId);
      await mock.waitForPending();
      assert.strictEqual(
        await page.locator("#chatInput").inputValue(),
        "Frage Neunzehn",
        "Im richtigen Chat muss die Frage wieder im Eingabefeld stehen",
      );
    },
    {
      fixtures: {
        chat: { error: "Backend kaputt" },
        chatSeed: {
          2: [
            {
              title: "Chat A",
              messages: [
                { role: "user", content: "Frage A" },
                { role: "assistant", content: "Antwort A" },
              ],
            },
            {
              title: "Chat B",
              messages: [
                { role: "user", content: "Frage B" },
                { role: "assistant", content: "Antwort B" },
              ],
            },
          ],
        },
      },
      delays: { chat_send: { 2: 600 } },
    },
  );
});

test("U20: Während der Anfrage ist die Eingabe gesperrt und der Button zeigt Stopp", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();

      await sendQuestion(page, "Frage Zwanzig");
      await page.waitForFunction(() =>
        window.__tauriMock.calls.some((call) => call.cmd === "chat_send"),
      );

      assert.strictEqual(await page.locator("#chatInput").isDisabled(), true, "#chatInput muss gesperrt sein");
      assert.strictEqual(await page.locator("#chatSend").textContent(), "Stopp");
      assert.strictEqual(await page.locator("#chatSend").isEnabled(), true, "#chatSend muss bedienbar bleiben");

      await mock.waitForPending();
      assert.strictEqual(await page.locator("#chatInput").isDisabled(), false, "#chatInput muss wieder frei sein");
    },
    { delays: { chat_send: { 2: CHAT_SEND_DELAY } } },
  );
});

test("U21: Absätze in der Assistentenblase haben keinen eigenen Hintergrund", async () => {
  await withApp(
    async (page, mock) => {
      await openChat(page, 2);
      await mock.waitForPending();

      await sendQuestion(page, "Frage Einundzwanzig");
      await mock.waitForPending();

      const paragraphs = page.locator("#chatMessages .chat-row--assistant .chat-bubble p");
      assert.strictEqual(await paragraphs.count(), 2, "Die Antwort muss zwei Absätze haben");
      const background = await paragraphs
        .first()
        .evaluate((element) => getComputedStyle(element).backgroundColor);
      assert.strictEqual(
        background,
        "rgba(0, 0, 0, 0)",
        "Ein Absatz in der Assistentenblase darf keinen eigenen Hintergrund bekommen",
      );
    },
    { fixtures: { chat: { answer: "Erster Absatz.\n\nZweiter Absatz." } } },
  );
});
