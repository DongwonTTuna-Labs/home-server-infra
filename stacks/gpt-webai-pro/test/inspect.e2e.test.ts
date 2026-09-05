import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { chromium } from "playwright-core";
import { inspectConversation } from "../src/daemon/actions/inspect.js";
import { BrowserSession } from "../src/daemon/browser.js";
import { ensureIntelligence } from "../src/daemon/actions/model.js";
import { sendMessage } from "../src/daemon/actions/send.js";
import { findChromium } from "./fake-chatgpt/chromium.js";
import { startFakeChatGpt } from "./fake-chatgpt/server.js";

test("inspection preserves the composer and excludes sidebar text from diagnostic files", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-inspect-"));
  const fake = await startFakeChatGpt();
  const browser = await chromium.launch({ executablePath: await findChromium(), headless: true });
  t.after(async () => { await browser.close(); await fake.close(); await rm(directory, { recursive: true, force: true }); });
  const session = await BrowserSession.fromBrowser(browser, fake.baseUrl("happy"));
  const page = (await session.inspectionPage())!;
  await page.locator("#prompt-textarea").fill("unsent draft");
  await page.evaluate(() => {
    const aside = document.createElement("aside");
    aside.textContent = "PRIVATE CHAT TITLE";
    document.body.append(aside);
    history.replaceState({}, "", "/c/inspect-owned");
  });
  const result = await inspectConversation(session, { conversationUrl: page.url() }, directory);
  const snapshot = await readFile(result.snapshotPath, "utf8");
  assert.equal(snapshot.includes("PRIVATE CHAT TITLE"), false);
  assert.equal(await page.locator("#prompt-textarea").innerText(), "unsent draft");
  assert.equal(await page.locator('[data-message-author-role="user"]').count(), 0);
  assert.deepEqual([...((await readFile(result.screenshotPath)).subarray(0, 8))], [137,80,78,71,13,10,26,10]);
});

test("image selection reaches the step below Pro and verifies Extended instead of submitting Pro", async (t) => {
  const fake = await startFakeChatGpt();
  const browser = await chromium.launch({ executablePath: await findChromium(), headless: true });
  t.after(async () => { await browser.close(); await fake.close(); });
  const session = await BrowserSession.fromBrowser(browser, fake.baseUrl("gpt6-legacy-model"));
  const page = (await session.inspectionPage())!;
  const label = await ensureIntelligence(page, {
    target: ["Extended", "Extra High", "Xhigh"],
    intelligence: ["Instant", "Light", "Standard", "Extended", "Pro"],
    modelVersion: "Latest", sliderOffsetFromMax: 1,
  });
  assert.equal(label, "6 Extended");
  assert.equal(await page.locator("#power-slider").getAttribute("aria-valuenow"), "3");
  assert.equal(await page.locator('[data-version="Latest"]').getAttribute("aria-checked"), "true");
  assert.equal(await page.locator('[data-message-author-role="user"]').count(), 0);
});

test("image send preserves the prompt and uses Extra High through tool selection", async (t) => {
  const fake = await startFakeChatGpt();
  const browser = await chromium.launch({ executablePath: await findChromium(), headless: true });
  t.after(async () => { await browser.close(); await fake.close(); });
  const session = await BrowserSession.fromBrowser(browser, fake.baseUrl("happy"));
  const prompt = "Generate five original images, separate outputs, in this order.";
  const result = await sendMessage(session, { prompt, files: [], imageCount: 5 }, {
    target: ["Pro"], intelligence: ["Instant", "Medium", "High", "Extra High", "Pro"],
  });
  assert.equal(result.modelLabel, "Extra High");
  const page = await session.findConversationPage(result.conversationUrl);
  assert.ok(page);
  assert.equal(await page.locator('[data-message-author-role="user"]').count(), 1);
  assert.ok((await page.locator('[data-message-author-role="user"]').innerText()).includes(prompt));
});

test("image power lowers Pro through the visible Power input when the slider is hidden", async (t) => {
  const fake = await startFakeChatGpt();
  const browser = await chromium.launch({ executablePath: await findChromium(), headless: true });
  t.after(async () => { await browser.close(); await fake.close(); });
  const session = await BrowserSession.fromBrowser(browser, fake.baseUrl("gpt6-power-menu"));
  const page = (await session.inspectionPage())!;
  const intelligence = ["Instant", "Medium", "High", "Extra High", "Pro"];
  await ensureIntelligence(page, { target: ["Pro"], intelligence, modelVersion: "Latest" });
  assert.equal(await page.locator("#power-slider").getAttribute("aria-valuenow"), "4");
  const label = await ensureIntelligence(page, {
    target: ["Extra High", "Xhigh", "Extended"], intelligence, modelVersion: "Latest", sliderOffsetFromMax: 1,
  });
  assert.equal(label, "6 Extra High");
  assert.equal(await page.locator("#power-slider").getAttribute("aria-valuenow"), "3");
  assert.equal(await page.locator('[data-version="Latest"]').getAttribute("aria-checked"), "true");
  assert.equal(await page.locator('[data-message-author-role="user"]').count(), 0);
});
