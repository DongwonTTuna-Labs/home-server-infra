import { randomUUID } from "node:crypto";
import path from "node:path";
import type { Page } from "playwright-core";
import { atomicWrite, mkdirp } from "../../shared/fsx.js";
import type { InspectParams, InspectResult } from "../../shared/types.js";
import type { BrowserSession } from "../browser.js";
import { COMPOSER_SELECTORS, UPLOAD_BUTTON_SELECTORS, readCurrentModelLabel, visibleFirst } from "../selectors.js";
import { composeImagePrompt, imagePreviewControl, imageViewer, selectImageTool } from "./images.js";

// 진단은 지정된 대화 또는 빈 새 대화의 main만 기록한다. 사이드바, 프로필, 쿠키,
// 이미지 원본 URL은 수집하지 않으며 전송·생성 중단을 하지 않는다.
// 입력 진단은 빈 새 대화에서만 허용한다.
export async function inspectConversation(
  session: BrowserSession,
  params: InspectParams,
  outboxDir: string,
): Promise<InspectResult> {
  if (params.conversationUrl && (params.openTools || params.selectImageTool || params.imagePrompt)) throw new Error("tools inspection requires a fresh conversation");
  const page = params.conversationUrl
    ? await session.open(params.conversationUrl)
    : await session.newConversation();
  try {
    const main = page.locator("main").first();
    await main.waitFor({ state: "visible", timeout: 30_000 });
    // SPA의 main이 먼저 보이고 모델/도구 컨트롤은 뒤늦게 hydrate될 수 있다.
    const modelLabel = await readCurrentModelLabel(page, ["Instant", "Medium", "High", "Extra High", "Xhigh", "Pro"]);
    let toolsMenuOpened = false;
    if (params.openTools) {
      const button = await visibleFirst(page, UPLOAD_BUTTON_SELECTORS);
      if (button) {
        await button.click();
        toolsMenuOpened = await page.getByText(/^(Create image|이미지 만들기)$/u, { exact: true })
          .waitFor({ state: "visible", timeout: 5_000 }).then(() => true, () => false);
      }
    }
    let diagnosticError: string | undefined;
    if (params.imagePrompt) {
      try { await composeImagePrompt(page, params.imagePrompt); }
      catch (error) { diagnosticError = error instanceof Error ? error.message : String(error); }
    } else if (params.selectImageTool) await selectImageTool(page);
    if (params.openImageIndex !== undefined) {
      if (!params.conversationUrl || !params.userTurnId || !Number.isInteger(params.openImageIndex) || params.openImageIndex < 0) throw new Error("image inspection requires a confirmed conversation and image index");
      try {
        await (await imagePreviewControl(page, params.userTurnId, params.openImageIndex)).click();
        await imageViewer(page).waitFor({ state: "visible", timeout: 10_000 });
      } catch (error) { diagnosticError = error instanceof Error ? error.message : String(error); }
    }
    return { ...await captureInspection(page, outboxDir), toolsMenuOpened, modelLabel, ...(diagnosticError ? { diagnosticError } : {}) };
  } finally {
    if (params.openImageIndex !== undefined) await page.keyboard.press("Escape");
    if (!params.conversationUrl) await page.close();
  }
}
// 이미 열린 페이지의 실패 순간만 기록한다. 탐색이나 모델 피커 대기를 하지 않는다.
export async function captureInspection(page: Page, outboxDir: string): Promise<Pick<InspectResult, "currentUrl" | "screenshotPath" | "snapshotPath">> {
  const main = page.locator("main").first();
  const directory = path.join(outboxDir, "diagnostics", randomUUID());
  await mkdirp(directory);
  const screenshotPath = path.join(directory, "screen.png");
  const snapshotPath = path.join(directory, "snapshot.txt");
  const snapshots = [await main.ariaSnapshot()];
  let visualScope = main;
  for (const dialog of await page.getByRole("dialog").all()) {
    if (await dialog.isVisible()) { snapshots.push(await dialog.ariaSnapshot()); visualScope = dialog; }
  }
  for (const header of await page.locator("header,[role=tablist]").all()) {
    if (await header.isVisible()) snapshots.push(await header.ariaSnapshot());
  }
  for (const menu of await page.getByRole("menu").all()) {
    if (await menu.isVisible()) snapshots.push(await menu.ariaSnapshot());
  }
  const controls = await visualScope.locator("img,button,[role=button]").evaluateAll((nodes) => nodes.map((node) => ({
    tag: node.tagName,
    label: node.getAttribute("aria-label") || node.getAttribute("alt") || node.textContent?.slice(0, 150) || "",
    testid: node.getAttribute("data-testid"),
    className: node.getAttribute("class"),
    title: node.getAttribute("title"),
    dimensions: node instanceof HTMLImageElement ? [node.naturalWidth, node.naturalHeight] : null,
    state: { pressed: node.getAttribute("aria-pressed"), selected: node.getAttribute("aria-selected"),
      disabled: node.getAttribute("aria-disabled"), state: node.getAttribute("data-state") },
    messageAncestor: node.closest("[data-message-id]")?.getAttribute("data-message-id"),
    article: { testid: node.closest("article")?.getAttribute("data-testid"), turn: node.closest("article")?.getAttribute("data-turn") },
    ancestors: [node.parentElement, node.parentElement?.parentElement, node.parentElement?.parentElement?.parentElement]
      .filter((item) => Boolean(item)).map((item) => ({ tag: item!.tagName, role: item!.getAttribute("role"),
        className: item!.getAttribute("class"), testid: item!.getAttribute("data-testid"), messageId: item!.getAttribute("data-message-id") })),
  })));
  const box = await visualScope.boundingBox();
  if (!box) throw new Error("main became invisible during inspection");
  await page.screenshot({ path: screenshotPath, type: "png", clip: visualScope === main ? { x: box.x, y: 0, width: box.width, height: box.y + box.height } : box });
  const composer = await visibleFirst(page, COMPOSER_SELECTORS);
  await atomicWrite(snapshotPath, snapshots.join("\n\n") + "\n\nControls:\n" + JSON.stringify(controls, null, 2)
    + "\n\nComposer text:\n" + (composer ? await composer.innerText() : ""));
  return { currentUrl: page.url(), screenshotPath, snapshotPath };
}
