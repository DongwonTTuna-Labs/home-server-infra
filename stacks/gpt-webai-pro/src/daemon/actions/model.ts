import type { Locator, Page } from "playwright-core";
import { GwpError } from "../../shared/errors.js";
import type { LabelConfig } from "../../shared/types.js";
import {
  INTELLIGENCE_OPTION_SELECTOR,
  INTELLIGENCE_PICKER_CONTENT_SELECTOR,
  findIntelligenceOption,
  findIntelligencePill,
  findModelSelectItem,
  findModelVersionOption,
  findPowerSlider,
  normalizeIntelligenceLabel,
  parsePillLabel,
  readCurrentModelLabel,
  readPowerStatusText,
  waitForIntelligenceMenu,
} from "../selectors.js";
const SETTLE_MS = 500;
const SLIDER_SETTLE_MS = 3_000;
const MENU_CLOSE_MS = 5_000;
/**
 * 전송 전에 모델을 보장한다. 두 UI를 모두 다룬다.
 *
 * - 구 UI(2026-07): 단일 Intelligence 라디오. 알약이 `target`이면 메뉴를 열지 않고 끝내고,
 *   아니면 메뉴에서 `target` 라디오를 클릭해 aria-checked를 확인한다.
 * - 새 UI(2026-09, GPT-6): 알약이 "6\nPro"(버전 + power). 메뉴 안에 생각 강도 슬라이더
 *   ([role=slider] 0..max, max=Pro)와 "Select model" 라디오(Latest/GPT-5.6 Sol/…)가 있다.
 *   슬라이더를 End로 최대(=Pro)로 올리고, `labels.modelVersion`(기본 Latest) 라디오가 켜져
 *   있는지 확인·선택한 뒤 메뉴를 닫고 알약을 재검증한다.
 *
 * 어느 경우에도 다른 power/모델로의 대체는 없다 — 목표를 만들 수 없으면 model_unavailable.
 * 반환값은 표시용 라벨("6 Pro" / "Pro")로 주간 사용량 기록의 증거가 된다.
 */
export async function ensurePro(page: Page, labels: LabelConfig): Promise<string> {
  const targets = new Set(labels.target.map(normalizeIntelligenceLabel));
  try {
    const pill = await findIntelligencePill(page, labels.intelligence);
    if (!pill) throw new Error("intelligence picker pill is not uniquely visible");
    let current = await pill.innerText().catch(() => "");
    let parsed = parsePillLabel(current);
    const powerExact = targets.has(normalizeIntelligenceLabel(current));
    // 알약에 버전 토큰("6")이 있으면 새 UI다. 새 UI에서만 모델 버전(Latest)까지 확인한다.
    const needVersionCheck = parsed.version !== null && Boolean(labels.modelVersion);
    if (powerExact && !needVersionCheck) return parsed.display;
    await pill.click({ timeout: 10_000 });
    if (!await waitForIntelligenceMenu(page, 10_000)) {
      throw new Error("intelligence menu did not become visible");
    }
    const slider = await findPowerSlider(page);
    if (slider) {
      if (!powerExact) await raiseSliderToMax(page, slider);
      const status = await readPowerStatusText(page);
      const statusPower = normalizeIntelligenceLabel(status.split(/[,.]/u)[0] ?? "");
      if (!targets.has(statusPower)) {
        throw new Error(`power slider did not reach ${labels.target.join("/")}; status is ${JSON.stringify(status)}`);
      }
    } else if (!powerExact) {
      const target = await findIntelligenceOption(page, labels.target);
      if (!target) {
        throw new Error(`target intelligence is unavailable: ${labels.target.join(", ")}`);
      }
      await target.click({ timeout: 10_000 });
      await page.waitForTimeout(SETTLE_MS);
      if (await target.getAttribute("aria-checked", { timeout: 5_000 }).catch(() => null) !== "true") {
        throw new Error("target intelligence did not become aria-checked");
      }
    }
    if (labels.modelVersion) await ensureModelVersion(page, labels.modelVersion, slider !== null);
    await closeIntelligenceMenu(page);
    current = await readCurrentModelLabel(page, labels.intelligence);
    parsed = parsePillLabel(current);
    if (!targets.has(normalizeIntelligenceLabel(current))) {
      throw new Error(`intelligence pill recheck failed; current label is ${JSON.stringify(current)}`);
    }
    return parsed.display;
  } catch (error) {
    if (error instanceof GwpError) throw error;
    throw new GwpError("model_unavailable", String(error), { phase: "pre_click", cause: error });
  }
}
async function raiseSliderToMax(page: Page, slider: Locator): Promise<void> {
  const max = Number(await slider.getAttribute("aria-valuemax").catch(() => null));
  if (!Number.isFinite(max)) throw new Error("power slider has no aria-valuemax");
  await slider.focus();
  await page.keyboard.press("End");
  const deadline = Date.now() + SLIDER_SETTLE_MS;
  for (;;) {
    const now = Number(await slider.getAttribute("aria-valuenow").catch(() => null));
    if (now === max) return;
    if (Date.now() >= deadline) break;
    await page.waitForTimeout(100);
  }
  // End가 먹지 않는 구현을 위한 보조: 오른쪽 화살표로 한 칸씩.
  for (let step = 0; step < max + 1; step += 1) {
    await page.keyboard.press("ArrowRight");
    await page.waitForTimeout(100);
    if (Number(await slider.getAttribute("aria-valuenow").catch(() => null)) === max) return;
  }
  throw new Error("power slider did not reach its maximum");
}
async function ensureModelVersion(page: Page, version: string, newUi: boolean): Promise<void> {
  const select = await findModelSelectItem(page);
  if (!select) {
    if (newUi) throw new Error("model version selector is unavailable");
    return;
  }
  let option = await findModelVersionOption(page, version);
  if (!option) {
    if (await select.getAttribute("aria-expanded").catch(() => null) !== "true") {
      await select.click({ timeout: 10_000 });
      await page.waitForTimeout(SETTLE_MS);
    }
    option = await findModelVersionOption(page, version);
  }
  if (!option) throw new Error(`model version is unavailable: ${version}`);
  if (await option.getAttribute("aria-checked").catch(() => null) === "true") return;
  await option.click({ timeout: 10_000 });
  await page.waitForTimeout(SETTLE_MS);
  const after = await findModelVersionOption(page, version);
  if (after && await after.getAttribute("aria-checked").catch(() => null) !== "true") {
    throw new Error(`model version ${version} did not become aria-checked`);
  }
}
async function menuOpen(page: Page): Promise<boolean> {
  const picker = page.locator(INTELLIGENCE_PICKER_CONTENT_SELECTOR).first();
  if (await picker.isVisible().catch(() => false)) return true;
  return page.locator(INTELLIGENCE_OPTION_SELECTOR).first().isVisible().catch(() => false);
}
async function closeIntelligenceMenu(page: Page): Promise<void> {
  // 구 UI는 라디오 클릭으로 이미 닫혀 있다. 새 UI는 Escape로 닫는다 (슬라이더 값은 즉시 적용).
  const deadline = Date.now() + MENU_CLOSE_MS;
  let escapes = 0;
  while (await menuOpen(page)) {
    if (escapes >= 2 && Date.now() >= deadline) throw new Error("intelligence menu did not close");
    if (escapes < 2) {
      await page.keyboard.press("Escape");
      escapes += 1;
    }
    await page.waitForTimeout(200);
  }
  await page.waitForTimeout(SETTLE_MS);
}
