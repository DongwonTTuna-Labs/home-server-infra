import type { Locator, Page } from "playwright-core";
import type { LabelConfig } from "../../shared/types.js";
import { GwpError } from "../../shared/errors.js";
import { answerActionVisible, COMPOSER_SELECTORS, COPY_CONTROL_SELECTOR, UPLOAD_BUTTON_SELECTORS, visibleFirst } from "../selectors.js";

// 사용자가 지정한 Xhigh: 구 UI의 Extra High, GPT-6 슬라이더의 Pro 바로 아래 Extended.
export function imageLabels(labels: LabelConfig): LabelConfig {
  return { ...labels, target: ["Extra High", "Xhigh", "Extended"], sliderOffsetFromMax: 1 };
}
export async function selectImageTool(page: Page): Promise<void> {
  const button = await visibleFirst(page, UPLOAD_BUTTON_SELECTORS);
  if (!button) throw new GwpError("compose_failed", "image tool menu button is unavailable", { phase: "pre_click" });
  if (await button.getAttribute("aria-expanded") !== "true") await button.click();
  // 실측 2026-09-05: + 팝오버는 role=menuitem 대신 group 안의 텍스트로 렌더된다.
  const option = page.getByRole("group").getByText(/^(Create image|이미지 만들기)$/u, { exact: true });
  await option.waitFor({ state: "visible", timeout: 10_000 });
  await option.click();
}
export async function composeImagePrompt(page: Page, prompt: string): Promise<void> {
  const composer = await visibleFirst(page, COMPOSER_SELECTORS);
  if (!composer) throw new GwpError("compose_failed", "composer is unavailable", { phase: "pre_click" });
  // 새 탭에도 이전 미전송 draft가 복원된다. 소유 요청의 입력을 시작하기 전에 비운다.
  // fill("")은 실 편집기의 화면 DOM만 비우고 도구 선택 시 React draft가 되살아났다.
  // 실제 삭제 키로 editor transaction을 발생시켜 입력 상태도 비운다.
  await composer.click();
  await composer.press("ControlOrMeta+A");
  await composer.press("Backspace");
  if ((await composer.innerText()).trim()) throw new GwpError("compose_failed", "could not clear the restored image draft", { phase: "pre_click" });
  await selectImageTool(page);
  if (!/Create image|이미지 만들기/u.test(await composer.innerText())) {
    throw new GwpError("compose_failed", "image tool is absent from the composer", { phase: "pre_click" });
  }
  await composer.click();
  await composer.press("End");
  await page.keyboard.insertText(`\n${prompt}`);
  const actual = await composer.innerText();
  if (!imageComposerMatches(actual, prompt)) {
    throw new GwpError("compose_failed", `image prompt did not survive composition (expected ${prompt.length} characters, observed ${actual.length})`, { phase: "pre_click" });
  }
}
export function imageComposerMatches(actual: string, prompt: string): boolean {
  // 실 ProseMirror는 문단 사이를 두 줄로 읽는다. 도구 표시 하나와 공백만 정규화한다.
  const normalized = actual.replace(/\s+/gu, " ").trim();
  const expected = prompt.replace(/\s+/gu, " ").trim();
  const prefix = /^(?:Create image|이미지 만들기)\s*/u;
  const suffix = /\s*(?:Create image|이미지 만들기)$/u;
  return (prefix.test(normalized) && normalized.replace(prefix, "") === expected)
    || (suffix.test(normalized) && normalized.replace(suffix, "") === expected);
}
export function imageSentTurnMatches(actual: string, prompt: string): boolean {
  // 이미지 user 턴은 줄바꿈을 일반 공백으로 렌더하기도 한다. 첨부·도구 표시는
  // 입력 앞에 남을 수 있지만 전체 프롬프트의 문자는 모두 일치해야 한다.
  const normalized = actual.replace(/\s+/gu, " ").trim();
  const expected = prompt.replace(/\s+/gu, " ").trim();
  return Boolean(expected) && (normalized === expected || normalized.endsWith(` ${expected}`));
}
export function imageViewer(page: Page): Locator {
  // 다중 세트의 제목은 Media viewer, 단일 이미지는 생성된 이미지 제목이다.
  return page.getByRole("dialog").filter({
    has: page.getByRole("group", { name: /^(Image tools|이미지 도구)$/u }),
  });
}

export interface GeneratedImageControl { locator: Locator; thumbnail: boolean }
export async function imageAnswerActionVisible(page: Page, controls: GeneratedImageControl[], assistantTurnId?: string): Promise<boolean> {
  const lastImage = controls.at(-1);
  if (!lastImage) return answerActionVisible(page, assistantTurnId);
  // 실 이미지 응답에는 assistant 메시지 노드가 없기도 한다. 마지막 이미지 뒤의
  // 복사 액션만 인정해 앞선 사용자 메시지의 동일한 한국어 버튼과 구분한다.
  return lastImage.locator.evaluate((node, selector) => {
    const actions = Array.from(node.closest("main")!.querySelectorAll(selector)).filter((action) => {
      const rect = action.getBoundingClientRect();
      const style = getComputedStyle(action);
      return Boolean(node.compareDocumentPosition(action) & Node.DOCUMENT_POSITION_FOLLOWING)
        && !node.contains(action) && rect.width > 0 && rect.height > 0
        && style.display !== "none" && style.visibility !== "hidden";
    });
    return actions.length === 1;
  }, COPY_CONTROL_SELECTOR);
}
// 다중 세트는 짧은 라벨, 단일 이미지는 제목이 붙은 라벨을 쓴다 (실 UI 관찰).
const GENERATED_IMAGE_SELECTOR = 'img[alt="Generated image"],img[alt^="Generated image: "]';
const GENERATED_IMAGE_NAME = /^Generated image(?:: .+)?$/u;
// 이미지 배치는 항상 새 대화다. 확인한 user anchor 외의 사용자 턴이 있으면 수집하지 않는다.
// 단일 image set은 큰 미리보기 1개 + button 썸네일 N개이며, 썸네일 내부 img 3개는
// 같은 이미지의 transition/blur 레이어다. img 개수가 아니라 썸네일 버튼을 센다.
export async function generatedImageControls(page: Page, userTurnId: string): Promise<GeneratedImageControl[]> {
  const main = page.locator("main").first();
  const users = main.locator('[data-message-author-role="user"]');
  await users.first().waitFor({ state: "visible", timeout: 15_000 });
  if (await users.count() !== 1 || await users.first().getAttribute("data-message-id") !== userTurnId) {
    throw new GwpError("turn_not_found", "image collection requires the sole confirmed user turn of a new conversation");
  }
  // 대화 재개 직후에는 완료된 이미지도 lazy placeholder로 렌더된다.
  const renderedImages = main.locator(GENERATED_IMAGE_SELECTOR);
  if (!await renderedImages.count()) {
    const placeholders = main.getByRole("button", { name: GENERATED_IMAGE_NAME });
    if (await placeholders.count()) {
      await placeholders.first().scrollIntoViewIfNeeded();
    }
  }
  const thumbnails = main.locator('button').filter({ has: page.locator(GENERATED_IMAGE_SELECTOR) });
  const candidates = await thumbnails.count() ? thumbnails
    : main.locator('div[role="button"]').filter({ has: page.locator(GENERATED_IMAGE_SELECTOR) });
  const thumbnail = await thumbnails.count() > 0;
  const result: GeneratedImageControl[] = [];
  for (const locator of await candidates.all()) {
    const isResponse = await locator.evaluate((node, expected) => {
      const user = Array.from(document.querySelectorAll('[data-message-author-role="user"]'))
        .find((item) => item.getAttribute("data-message-id") === expected);
      return Boolean(user && !node.closest('[data-message-author-role="user"]')
        && (user.compareDocumentPosition(node) & Node.DOCUMENT_POSITION_FOLLOWING));
    }, userTurnId);
    if (isResponse) result.push({ locator, thumbnail });
  }
  return result;
}
export async function generatedImagesLoaded(page: Page, controls: GeneratedImageControl[]): Promise<boolean> {
  if (!controls.length) {
    // 완료 응답의 비어 있는 갤러리는 이미지 0장이 아니라 재개 후 로딩 중이다.
    return await page.locator("main").getByRole("button", { name: GENERATED_IMAGE_NAME }).count() === 0;
  }
  for (const control of controls) {
    const loaded = await control.locator.locator(GENERATED_IMAGE_SELECTOR).first().evaluate((node) => (
      node instanceof HTMLImageElement && node.complete && node.naturalWidth >= 256 && node.naturalHeight >= 256
    ));
    if (!loaded) return false;
  }
  return true;
}
export async function imagePreviewControl(page: Page, userTurnId: string, index: number, expectedCount?: number): Promise<Locator> {
  const controls = await generatedImageControls(page, userTurnId);
  // poll 이후에도 갤러리가 재수화될 수 있다. 앞쪽 썸네일 누락으로 index가 밀리면
  // 다른 원본에 입력 ID가 붙으므로 매 다운로드 선택 직전에 전체를 다시 확인한다.
  // 진단용 inspect는 입력 ID를 부여하지 않으므로 예상 수량 없이 현재 원본을 연다.
  if ((expectedCount !== undefined && controls.length !== expectedCount) || !await generatedImagesLoaded(page, controls)) {
    throw new GwpError("artifact_failed", `generated image gallery is incomplete (expected ${expectedCount ?? controls.length}, observed ${controls.length})`);
  }
  const control = controls[index];
  if (!control) throw new GwpError("artifact_failed", `generated image ${index + 1} is absent`);
  if (!control.thumbnail) return control.locator;
  const expectedSource = await control.locator.locator(GENERATED_IMAGE_SELECTOR).first().evaluate((node) => (node as HTMLImageElement).currentSrc);
  await control.locator.click();
  const preview = page.locator('main div[role="button"]').filter({ has: page.locator(GENERATED_IMAGE_SELECTOR) });
  if (await preview.count() !== 1) throw new GwpError("artifact_failed", "selected image preview is not unique");
  const deadline = Date.now() + 10_000;
  while (await preview.locator(GENERATED_IMAGE_SELECTOR).first().evaluate((node) => (node as HTMLImageElement).currentSrc) !== expectedSource) {
    if (Date.now() >= deadline) throw new GwpError("artifact_failed", "preview did not switch to the selected image");
    await page.waitForTimeout(100);
  }
  return preview;
}
