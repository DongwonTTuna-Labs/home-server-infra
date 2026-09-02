import { access } from "node:fs/promises";
import path from "node:path";
import type { Download, Locator, Page } from "playwright-core";
import { GwpError } from "../../shared/errors.js";
import { fileSize, mkdirp, sha256File } from "../../shared/fsx.js";
import type { DownloadParams, DownloadResult } from "../../shared/types.js";
import type { BrowserSession } from "../browser.js";
import {
  FILENAME_PATTERN,
  PANEL_DOWNLOAD_SELECTOR,
  artifactControlLocators,
  type ArtifactControlLocator,
} from "../selectors.js";
// 클릭 즉시 파일을 내려주는 "직접 다운로드 버튼"이 download 이벤트를 낼 때까지 기다리는
// 결정 창(窓). 이 안에 이벤트도 미리보기 패널도 없으면 늦게 오는 다운로드를 한 번 더 본다.
const DECISION_WINDOW_MS = 10_000;
// 미리보기 패널의 Download 버튼을 누른 뒤 실제 download 이벤트까지의 여유.
const PANEL_DOWNLOAD_MS = 30_000;
// 결정 창에서 아무 신호도 없을 때, 늦게 도착하는 직접 다운로드를 마지막으로 기다리는 여유.
const LATE_DIRECT_MS = 4_000;
let panelMarkerSequence = 0;
export class ArtifactDownloader {
  private readonly attempts = new Map<string, number>();
  constructor(private readonly outboxDir: string) {}
  async download(
    session: BrowserSession,
    params: DownloadParams,
  ): Promise<DownloadResult> {
    const key = `${params.conversationUrl}\n${params.controlIndex}`;
    const attempt = (this.attempts.get(key) ?? 0) + 1;
    if (attempt > 2) throw new GwpError("artifact_failed", "artifact control exceeded two attempts");
    this.attempts.set(key, attempt);
    const page = await session.findConversationPage(params.conversationUrl)
      ?? await session.open(params.conversationUrl);
    const controls = await artifactControlLocators(page);
    const control = controls[params.controlIndex];
    if (!control) throw new GwpError("artifact_failed", `artifact control ${params.controlIndex} is absent`);
    return downloadArtifactControl(page, control, this.outboxDir, params.controlIndex);
  }
}
// 컨트롤 하나를 실제로 내려받아 outbox에 저장하는 핵심 절차. 컨트롤 해석(어느 턴의 몇 번째)과
// 분리해 두어, 특정 턴의 컨트롤을 직접 넘겨 라이브로 검증할 수 있게 한다.
export async function downloadArtifactControl(
  page: Page,
  control: ArtifactControlLocator,
  outboxDir: string,
  controlIndex = 0,
): Promise<DownloadResult> {
  try {
    const download = await triggerDownload(page, control);
    const filename = chooseFilename(control.label, download.suggestedFilename());
    await mkdirp(outboxDir);
    const outboxPath = await uniqueOutboxPath(outboxDir, controlIndex, filename);
    await download.saveAs(outboxPath);
    const failure = await download.failure();
    if (failure) throw new Error(failure);
    return {
      filename,
      outboxPath,
      sha256: await sha256File(outboxPath),
      sizeBytes: await fileSize(outboxPath),
    };
  } catch (error) {
    if (error instanceof GwpError) throw error;
    throw new GwpError("artifact_failed", `artifact download failed: ${String(error)}`, { cause: error });
  }
}
async function triggerDownload(page: Page, control: ArtifactControlLocator): Promise<Download> {
  if (control.kind === "inline") {
    const event = page.waitForEvent("download", { timeout: PANEL_DOWNLOAD_MS });
    await control.locator.click({ timeout: 10_000 }).catch((error) => {
      void event.catch(() => undefined);
      throw error;
    });
    return event;
  }
  // 비인라인 컨트롤은 두 갈래로 렌더된다:
  //  (1) 클릭하면 곧바로 파일이 내려오는 직접 다운로드 버튼 (예: "<파일명> 다운로드" 텍스트 버튼)
  //  (2) 클릭하면 파일 미리보기 패널이 열리고, 그 안의 Download 버튼을 눌러야 하는 엔터티
  // ChatGPT는 같은 산출물을 응답마다 (1)/(2) 어느 쪽으로도 낼 수 있어(특히 생성 zip은 (1),
  // 심층 리서치 첨부는 (2)가 잦다), 먼저 (1)로 시도하고 다운로드 신호가 없으면 (2)로 폴백한다.
  const marker = String(++panelMarkerSequence);
  await page.locator(PANEL_DOWNLOAD_SELECTOR).evaluateAll((nodes, value) => {
    for (const node of nodes) node.setAttribute("data-gwp-download-baseline", value as string);
  }, marker);
  // 클릭 전에 다운로드 리스너를 건다. 직접 버튼이면 이 리스너가 곧바로 이벤트를 받는다.
  const directSettled = page.waitForEvent("download", { timeout: PANEL_DOWNLOAD_MS })
    .then((download) => download, () => null);
  let direct: Download | null = null;
  let directResolved = false;
  void directSettled.then((download) => { direct = download; directResolved = true; });
  await control.locator.click({ timeout: 10_000 }).catch((error) => {
    void directSettled;
    throw error;
  });
  const deadline = Date.now() + DECISION_WINDOW_MS;
  for (;;) {
    if (directResolved && direct) return direct;
    const panel = await freshPanelDownload(page, marker);
    if (panel) {
      const event = page.waitForEvent("download", { timeout: PANEL_DOWNLOAD_MS });
      await panel.click({ timeout: 10_000 }).catch((error) => {
        void event.catch(() => undefined);
        throw error;
      });
      return event;
    }
    if (Date.now() >= deadline) break;
    await page.waitForTimeout(100);
  }
  // 패널도 없고 직접 다운로드도 아직이면, 늦게 오는 직접 다운로드를 마지막으로 기다린다.
  const late = await Promise.race([
    directSettled,
    page.waitForTimeout(LATE_DIRECT_MS).then(() => null),
  ]);
  if (late) return late;
  throw new Error(`file preview for ${control.label} did not expose Download`);
}
async function freshPanelDownload(page: Page, baselineMarker: string): Promise<Locator | null> {
  const controls = page.locator(PANEL_DOWNLOAD_SELECTOR);
  const count = await controls.count();
  for (let index = 0; index < count; index += 1) {
    const control = controls.nth(index);
    if (await control.getAttribute("data-gwp-download-baseline") !== baselineMarker
      && await control.isVisible().catch(() => false)) return control;
  }
  return null;
}
// ChatGPT는 직접 다운로드 버튼의 다운로드에 제네릭 이름("download", 확장자 없음)을 붙이는
// 경우가 있다. 그럴 때는 화면에 보인 파일명(control.label)으로 교정한다. suggestedFilename이
// 실제 확장자를 가진 이름이면 그대로 존중한다.
function chooseFilename(label: string, suggested: string): string {
  const fromSuggested = sanitizeBasename(suggested);
  const suggestedIsReal = /\.[a-z0-9]{1,8}$/iu.test(fromSuggested)
    && fromSuggested.toLowerCase() !== "download";
  if (suggestedIsReal) return fromSuggested;
  const fromLabel = (label.match(FILENAME_PATTERN)?.[0] ?? "").trim();
  if (fromLabel) return fromLabel;
  return safeBasename(suggested);
}
function sanitizeBasename(value: string): string {
  return path.basename(value).replace(/[\u0000-\u001f]/gu, "").trim();
}
function safeBasename(value: string): string {
  const filename = sanitizeBasename(value);
  if (!filename || filename === "." || filename === "..") {
    throw new GwpError("artifact_failed", "download supplied an invalid filename");
  }
  return filename;
}
async function uniqueOutboxPath(directory: string, index: number, filename: string): Promise<string> {
  for (let suffix = 0; suffix < 1_000; suffix += 1) {
    const prefix = `.gwp-${process.pid}-${index}-${suffix}-`;
    const candidate = path.join(directory, `${prefix}${filename}`);
    try {
      await access(candidate);
    } catch {
      return candidate;
    }
  }
  throw new GwpError("artifact_failed", "outbox filename space exhausted");
}
