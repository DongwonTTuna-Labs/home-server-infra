import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { chromium } from "playwright-core";
import { ArtifactDownloader } from "../src/daemon/actions/download.js";
import { generatedImageControls } from "../src/daemon/actions/images.js";
import { pollConversation } from "../src/daemon/actions/poll.js";
import { sendMessage } from "../src/daemon/actions/send.js";
import { BrowserSession } from "../src/daemon/browser.js";
import { sha256Text } from "../src/shared/fsx.js";
import { inspectImageFile } from "../src/supervisor/image-batch.js";
import { findChromium } from "./fake-chatgpt/chromium.js";
import { startFakeChatGpt } from "./fake-chatgpt/server.js";

test("image-only carousel counts outputs, downloads selected originals, and allows same-daemon resume", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-image-cards-"));
  const fake = await startFakeChatGpt();
  const browser = await chromium.launch({ executablePath: await findChromium(), headless: true });
  t.after(async () => { await browser.close(); await fake.close(); await rm(directory, { recursive: true, force: true }); });
  const session = await BrowserSession.fromBrowser(browser, fake.baseUrl("image-set"));
  const prompt = "Generate five different images in this order.";
  const sent = await sendMessage(session, { prompt, files: [], imageCount: 5 }, {
    target: ["Pro"], intelligence: ["Instant", "Medium", "High", "Extra High", "Pro"],
  });
  const params = { ...sent, promptSha256: sha256Text(prompt), imageCount: 5, waitMs: 6_000 };
  const result = await pollConversation(session, params);
  assert.equal(result.state, "complete");
  assert.equal(result.answerMarkdown, "");
  assert.equal(result.artifactControls?.length, 5);
  const page = (await session.findConversationPage(sent.conversationUrl))!;
  assert.equal(await page.locator('img[alt="Generated image"]').count(), 16);
  // 재개 직후 실 UI는 이미지 없이 이름 있는 preview와 썸네일 자리만 먼저 만든다.
  const preview = page.locator('div[role="button"]').filter({ has: page.locator('img[alt="Generated image"]') });
  await preview.evaluate((node) => node.setAttribute("aria-label", "Generated image"));
  await page.locator('img[alt="Generated image"]').evaluateAll((nodes) => {
    for (const node of nodes) {
      const placeholder = document.createElement("template");
      placeholder.dataset.imagePlaceholder = "true";
      node.replaceWith(placeholder);
      placeholder.content.append(node);
    }
  });
  await page.locator('div[role="button"]').evaluate((node) => node.removeAttribute("aria-label"));
  const hydrating = await pollConversation(session, { ...params, waitMs: 10_000 });
  assert.equal(hydrating.state, "generating", "an empty assistant before the gallery acquires its label is not a zero-image completion");
  await page.locator('div[role="button"]').evaluate((node) => node.setAttribute("aria-label", "Generated image"));
  const loading = await pollConversation(session, { ...params, waitMs: 24_000 });
  assert.equal(loading.state, "generating", "a completed response with a loading gallery must remain resumable");
  await page.locator('template[data-image-placeholder]').evaluateAll((nodes) => {
    for (const node of nodes) node.replaceWith((node as HTMLTemplateElement).content);
  });
  const downloader = new ArtifactDownloader(directory);
  const base = { conversationUrl: sent.conversationUrl, userTurnId: sent.userTurnId, imageCount: 5 };
  const first = await downloader.download(session, { ...base, controlIndex: 0 });
  const second = await downloader.download(session, { ...base, controlIndex: 1 });
  assert.notDeepEqual(await readFile(first.outboxPath), await readFile(second.outboxPath));
  assert.deepEqual(await inspectImageFile(second.outboxPath), { extension: "png", width: 512, height: 384 });
  await page.locator('div[role="button"]').filter({ has: page.locator('img[alt="Generated image"]') }).click();
  assert.equal(await page.getByRole("dialog").count(), 1);
  await downloader.download(session, { ...base, controlIndex: 0 });
  const resumed = await downloader.download(session, { ...base, controlIndex: 0 });
  assert.equal(resumed.sha256, first.sha256);
  await assert.rejects(() => downloader.download(session, { ...base, userTurnId: "unrelated-user", controlIndex: 0 }), /sole confirmed user/);
  await page.locator('[data-image-index="4"]').evaluate((node) => node.remove());
  const partial = await pollConversation(session, params);
  assert.equal(partial.artifactControls?.length, 4);
  await page.evaluate(() => {
    const extra = document.createElement("div"); extra.dataset.messageAuthorRole = "user";
    extra.dataset.messageId = "later-user"; extra.textContent = "unrelated follow-up";
    document.querySelector("main")!.append(extra);
  });
  await assert.rejects(() => generatedImageControls(page, sent.userTurnId), /sole confirmed user/);
});

test("a text-only image response preserves the explanation instead of treating an empty gallery as success", async (t) => {
  const fake = await startFakeChatGpt();
  const browser = await chromium.launch({ executablePath: await findChromium(), headless: true });
  t.after(async () => { await browser.close(); await fake.close(); });
  const session = await BrowserSession.fromBrowser(browser, fake.baseUrl("happy"));
  const prompt = "Generate one image.";
  const sent = await sendMessage(session, { prompt, files: [], imageCount: 1 }, {
    target: ["Pro"], intelligence: ["Instant", "Medium", "High", "Extra High", "Pro"],
  });
  const result = await pollConversation(session, { ...sent, imageCount: 1, promptSha256: sha256Text(prompt), waitMs: 12_000 });
  assert.equal(result.state, "complete");
  assert.deepEqual(result.artifactControls, []);
  assert.equal(result.answerMarkdown, "fake answer");
  assert.equal(result.answerSha256, sha256Text("fake answer"));
});

test("a single generated image with a descriptive alt is collected without thumbnail controls", async (t) => {
  const previousConfirm = process.env.GWP_CONFIRM_WINDOW_MS;
  process.env.GWP_CONFIRM_WINDOW_MS = "1000";
  t.after(() => { if (previousConfirm === undefined) delete process.env.GWP_CONFIRM_WINDOW_MS; else process.env.GWP_CONFIRM_WINDOW_MS = previousConfirm; });
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-single-image-"));
  const fake = await startFakeChatGpt();
  const browser = await chromium.launch({ executablePath: await findChromium(), headless: true });
  t.after(async () => { await browser.close(); await fake.close(); await rm(directory, { recursive: true, force: true }); });
  const session = await BrowserSession.fromBrowser(browser, fake.baseUrl("image-single"));
  const prompt = "Generate one map image.\nKeep four numbered stops.";
  const sent = await sendMessage(session, { prompt, files: [], imageCount: 1 }, {
    target: ["Pro"], intelligence: ["Instant", "Medium", "High", "Extra High", "Pro"],
  });
  const result = await pollConversation(session, { ...sent, imageCount: 1, promptSha256: sha256Text(prompt), waitMs: 6_000 });
  assert.equal(result.state, "complete");
  assert.equal(result.artifactControls?.length, 1);
  const downloaded = await new ArtifactDownloader(directory).download(session, {
    conversationUrl: sent.conversationUrl, userTurnId: sent.userTurnId, imageCount: 1, controlIndex: 0,
  });
  assert.deepEqual(await inspectImageFile(downloaded.outboxPath), { extension: "png", width: 512, height: 384 });
});

test("image completion accepts Korean response copy without mistaking the user's copy action for completion", async (t) => {
  const fake = await startFakeChatGpt();
  const browser = await chromium.launch({ executablePath: await findChromium(), headless: true });
  t.after(async () => { await browser.close(); await fake.close(); });
  const session = await BrowserSession.fromBrowser(browser, fake.baseUrl("image-single"));
  const prompt = "Generate one image.";
  const sent = await sendMessage(session, { prompt, files: [], imageCount: 1 }, {
    target: ["Pro"], intelligence: ["Instant", "Medium", "High", "Extra High", "Pro"],
  });
  const page = (await session.findConversationPage(sent.conversationUrl))!;
  await page.getByRole("button", { name: "Copy response", exact: true }).waitFor();
  await page.evaluate(() => {
    document.querySelector('[aria-label="Copy response"]')!.remove();
    document.querySelector('[data-message-author-role="assistant"]')!.remove();
    const userCopy = document.createElement("button");
    userCopy.setAttribute("aria-label", "복사");
    userCopy.textContent = "복사";
    document.querySelector('[data-message-author-role="user"]')!.after(userCopy);
  });
  const params = { ...sent, imageCount: 1, promptSha256: sha256Text(prompt), waitMs: 4_000 };
  assert.equal((await pollConversation(session, params)).state, "generating");
  await page.evaluate(() => {
    const responseCopy = document.createElement("button");
    responseCopy.setAttribute("aria-label", "복사");
    responseCopy.textContent = "복사";
    document.querySelector("main")!.append(responseCopy);
  });
  const result = await pollConversation(session, params);
  assert.equal(result.state, "complete");
  assert.equal(result.artifactControls?.length, 1);
});
