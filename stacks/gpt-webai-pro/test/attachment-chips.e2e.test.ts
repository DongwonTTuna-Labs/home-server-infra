import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { chromium } from "playwright-core";
import { sendMessage } from "../src/daemon/actions/send.js";
import { BrowserSession } from "../src/daemon/browser.js";
import { normalizeChipStem, observeAttachmentChips } from "../src/daemon/selectors.js";
import { GwpError } from "../src/shared/errors.js";
import { findChromium } from "./fake-chatgpt/chromium.js";
import { startFakeChatGpt } from "./fake-chatgpt/server.js";

test("named image groups stay separate from ZIP chips and wait for image readiness", async (t) => {
  const fake = await startFakeChatGpt();
  const browser = await chromium.launch({ executablePath: await findChromium(), headless: true });
  t.after(async () => { await browser.close(); await fake.close(); });
  const page = await browser.newPage();
  await page.goto(fake.baseUrl("attachments"));
  await page.locator("#prompt-textarea").fill("Edit first.png and second.png; context.zip contains references.");
  await page.locator("#chips").evaluate(node => {
    node.innerHTML = '<div role="group" aria-label="first.png"><button aria-label="Open image: User uploaded image"><img src="/preview/image-0.png"></button><button aria-label="Remove file 1: first.png"></button></div>'
      + '<div role="group" aria-label="second.png" aria-busy="true"><button aria-label="Open image: User uploaded image"><img src="/preview/image-1.png"></button><button aria-label="Remove file 2: second.png"></button></div>'
      + '<div role="group" aria-label="context(1).zip"><button>context(1).zip</button><span>Zip Archive</span><button aria-label="Remove file 3: context(1).zip"></button></div>'
      + '<div role="group" aria-label="hidden.png" style="display:none"><button aria-label="Remove file 4: hidden.png"></button></div>';
  });
  for (const img of await page.locator("#chips img").all()) await img.evaluate(img => (img as HTMLImageElement).decode());
  const first = await observeAttachmentChips(page);
  assert.deepEqual(first.map(({ filename, complete }) => ({ filename, complete })), [
    { filename: "first.png", complete: true }, { filename: "second.png", complete: false }, { filename: "context(1).zip", complete: true },
  ]);
  await page.getByRole("group", { name: "second.png", exact: true }).evaluate(node => {
    node.removeAttribute("aria-busy");
    node.querySelector("img")!.removeAttribute("src");
  });
  assert.equal((await observeAttachmentChips(page)).find(chip => chip.filename === "second.png")?.complete, false);
  await page.getByRole("group", { name: "second.png", exact: true }).locator("img").evaluate(async img => {
    (img as HTMLImageElement).src = "/preview/image-1.png";
    await (img as HTMLImageElement).decode();
  });
  assert.equal((await observeAttachmentChips(page)).every(chip => chip.complete), true);
});

test("UI duplicate suffixes normalize with and without a preceding space", () => {
  assert.equal(normalizeChipStem("context(1).zip"), normalizeChipStem("context.zip"));
  assert.equal(normalizeChipStem("context (1).zip"), normalizeChipStem("context.zip"));
  assert.notEqual(normalizeChipStem("context-final.zip"), normalizeChipStem("context.zip"));
});

test("attachment mismatch keeps pre-click authority and captures the attachment screen", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-chip-failure-"));
  const fake = await startFakeChatGpt();
  const browser = await chromium.launch({ executablePath: await findChromium(), headless: true });
  t.after(async () => { await browser.close(); await fake.close(); await rm(directory, { recursive: true, force: true }); });
  const session = await BrowserSession.fromBrowser(browser, fake.baseUrl("attachments"));
  const file = path.join(directory, "actual.txt");
  await writeFile(file, "synthetic attachment");
  let failure: GwpError | undefined;
  try {
    await sendMessage(session, { prompt: "Do not submit mismatched attachments", files: [
      { name: "different.txt", containerPath: file },
    ] }, { target: ["Pro"], intelligence: ["Instant", "Medium", "High", "Extra High", "Pro"], modelVersion: "Latest" }, undefined, directory);
  } catch (error) {
    assert.ok(error instanceof GwpError);
    failure = error;
  }
  assert.ok(failure);
  assert.equal(failure.kind, "chip_mismatch");
  assert.equal(failure.phase, "pre_click");
  assert.equal((await session.inspectionPage())!.url().includes("/c/"), false);
  assert.equal(await (await session.inspectionPage())!.locator('[data-message-author-role="user"]').count(), 0);
  const snapshotPath = failure.detail.match(/attachment diagnostic: (.+\/snapshot\.txt)/u)?.[1];
  assert.ok(snapshotPath, failure.detail);
  assert.match(await readFile(snapshotPath, "utf8"), /actual\.txt/u);
  assert.deepEqual([...((await readFile(path.join(path.dirname(snapshotPath), "screen.png"))).subarray(0, 8))], [137, 80, 78, 71, 13, 10, 26, 10]);
});
