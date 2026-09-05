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
  findPowerControl,
  normalizeIntelligenceLabel,
  parsePillLabel,
  readCurrentModelLabel,
  readPowerStatusText,
  waitForIntelligenceMenu,
  POWER_SLIDER_SELECTOR,
  type PowerControl,
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
 *   보이는 Power 입력점에 방향 키를 보내 최대에서 sliderOffsetFromMax만큼 아래로 맞추고, `labels.modelVersion`(기본 Latest) 라디오가 켜져
 *   있는지 확인·선택한 뒤 메뉴를 닫고 알약을 재검증한다.
 *
 * 어느 경우에도 다른 power/모델로의 대체는 없다 — 목표를 만들 수 없으면 model_unavailable.
 * 반환값은 표시용 라벨("6 Pro" / "Extra High" / "6 Extended")로 주간 사용량 기록의 증거가 된다.
 */
export async function ensureIntelligence(page: Page, labels: LabelConfig): Promise<string> {
  const targets = new Set(labels.target.map(normalizeIntelligenceLabel));
  try {
    const pill = await findIntelligencePill(page, labels.intelligence);
    if (!pill) throw new Error("intelligence picker pill is not uniquely visible");
    let current = await pill.innerText().catch(() => "");
    let parsed = parsePillLabel(current);
    const powerExact = targets.has(normalizeIntelligenceLabel(current));
    // 버전 토큰이 있으면 메뉴에서 버전을 확인한다. 구 UI는 Pro 라디오 선택 증거로 구분한다.
    const needVersionCheck = parsed.version !== null && Boolean(labels.modelVersion);
    if (powerExact && !needVersionCheck) return parsed.display;
    await pill.click({ timeout: 10_000 });
    if (!await waitForIntelligenceMenu(page, 10_000)) {
      throw new Error("intelligence menu did not become visible");
    }
    const power = await findPowerControl(page);
    let legacySelectionConfirmed = false;
    if (power) {
      if (!powerExact) await setPower(page, power, labels.sliderOffsetFromMax ?? 0);
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
      legacySelectionConfirmed = true;
    }
    const versionRequired = power !== null || (needVersionCheck && !legacySelectionConfirmed);
    if (labels.modelVersion) await ensureModelVersion(page, labels.modelVersion, versionRequired, pill);
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
async function setPower(page: Page, { input, slider }: PowerControl, offset: number): Promise<void> {
  const minimum = Number(await slider.getAttribute("aria-valuemin") ?? Number.NaN);
  const maximum = Number(await slider.getAttribute("aria-valuemax") ?? Number.NaN);
  const current = Number(await slider.getAttribute("aria-valuenow") ?? Number.NaN);
  if (![minimum, maximum, current, offset].every(Number.isInteger)
    || minimum > current || current > maximum || maximum - minimum > 20
    || offset < 0 || offset > maximum - minimum) {
    throw new Error("power slider has invalid bounds, current value, or target offset");
  }
  const target = maximum - offset;
  const direction = Math.sign(target - current);
  for (let value = current; value !== target;) {
    value += direction;
    await input.press(direction > 0 ? "ArrowRight" : "ArrowLeft");
    await page.waitForFunction(
      ({ selector, value }) => document.querySelector(selector)?.getAttribute("aria-valuenow") === String(value),
      { selector: POWER_SLIDER_SELECTOR, value },
      { timeout: SLIDER_SETTLE_MS },
    );
  }
}
async function ensureModelVersion(page: Page, version: string, required: boolean, pill: Locator): Promise<void> {
  const select = await findModelSelectItem(page);
  if (!select) {
    if (required) throw new Error("model version selector is unavailable");
    return;
  }
  const option = await revealModelVersionOption(page, select, version);
  if (!option) throw new Error(`model version is unavailable: ${version}`);
  if (await option.getAttribute("aria-checked").catch(() => null) === "true") return;
  await option.click({ timeout: 10_000 });
  await page.waitForTimeout(SETTLE_MS);
  let after = await findModelVersionOption(page, version);
  if (!after) {
    if (!await menuOpen(page)) {
      await pill.click({ timeout: 10_000 });
      if (!await waitForIntelligenceMenu(page, 10_000)) throw new Error("intelligence menu did not reopen for model confirmation");
    }
    const reopenedSelect = await findModelSelectItem(page);
    if (reopenedSelect) after = await revealModelVersionOption(page, reopenedSelect, version);
  }
  if (!after || await after.getAttribute("aria-checked").catch(() => null) !== "true") {
    throw new Error(`model version ${version} did not become aria-checked`);
  }
}
async function revealModelVersionOption(page: Page, select: Locator, version: string): Promise<Locator | null> {
  const option = await findModelVersionOption(page, version);
  if (option) return option;
  if (await select.getAttribute("aria-expanded").catch(() => null) !== "true") {
    await select.click({ timeout: 10_000 });
    await page.waitForTimeout(SETTLE_MS);
  }
  return findModelVersionOption(page, version);
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
