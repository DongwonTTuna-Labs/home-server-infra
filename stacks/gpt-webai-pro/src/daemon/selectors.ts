import type { Locator, Page } from "playwright-core";
import type { ArtifactControl } from "../shared/types.js";
export const COMPOSER_SELECTORS = [
  "#prompt-textarea",
  '[contenteditable="true"][role="textbox"]',
  'textarea[placeholder*="Message" i]',
  'textarea[placeholder*="메시지" i]',
] as const;
export const INTELLIGENCE_PILL_SELECTOR = "form button[aria-haspopup]";
export const INTELLIGENCE_OPTION_SELECTOR = '[role="menu"] [role="menuitemradio"]';
// 2026-09 GPT-6 UI: 알약을 열면 나오는 피커 본문. 안에 모델 선택(menuitem "Select model" →
// menuitemradio Latest/GPT-5.6 Sol/…)과 생각 강도 슬라이더([role=slider], 0..max, max=Pro)가 있다.
export const INTELLIGENCE_PICKER_CONTENT_SELECTOR = '[data-testid="composer-intelligence-picker-content"]';
export const POWER_SLIDER_SELECTOR = `${INTELLIGENCE_PICKER_CONTENT_SELECTOR} [role="slider"]`;
export const POWER_STATUS_SELECTOR = '[data-testid="composer-model-picker-slider-simple-view"]';
export const MODEL_SELECT_ITEM_SELECTOR = `${INTELLIGENCE_PICKER_CONTENT_SELECTOR} [role="menuitem"][aria-label="Select model" i]`;
export const MODEL_VERSION_OPTION_SELECTOR = `${INTELLIGENCE_PICKER_CONTENT_SELECTOR} [role="menuitemradio"]`;
// 버전 배지 토큰: "6", "5.5", "6.1" 처럼 숫자만으로 된 줄/단어.
const VERSION_TOKEN = /^\d+(?:\.\d+)*$/u;
export const SEND_BUTTON_SELECTORS = [
  'button[data-testid*="send" i]',
  'button[aria-label*="send" i]',
  'button[aria-label*="보내" i]',
] as const;
export const UPLOAD_BUTTON_SELECTORS = [
  'button[aria-label*="attach" i]',
  'button[aria-label*="upload" i]',
  'button[aria-label*="file" i]',
  'button[aria-label*="첨부" i]',
  'button[data-testid*="upload" i]',
] as const;
export const FILE_INPUT_SELECTOR = 'input[type="file"]';
export const TURN_SELECTOR = "[data-message-author-role]";
export const ASSISTANT_TURN_SELECTOR = '[data-message-author-role="assistant"]';
export const PANEL_DOWNLOAD_SELECTOR = [
  'button[aria-label="Download" i]',
  '[role="button"][aria-label="Download" i]',
  // 계정 UI가 한국어면 미리보기 패널의 다운로드 버튼 aria-label이 "다운로드" 계열이다.
  'button[aria-label*="다운로드"]',
  '[role="button"][aria-label*="다운로드"]',
].join(",");
export const STOP_PATTERN = /stop generating|stop responding|stop answering|stop-button|중지|정지/i;
export const FILENAME_PATTERN = /[^\s:/\\"'<>|]+\.[a-z0-9]{1,8}\b/iu;
const ARTIFACT_CONTROL_SELECTOR = [
  "a[download]",
  "button",
  '[role="button"]',
].join(",");
export const COPY_CONTROL_SELECTOR = [
  '[data-testid*="copy" i]',
  '[aria-label*="copy" i]',
  '[aria-label*="복사" i]',
].join(",");
const LOGIN_CONTROL_SELECTOR = [
  '[data-testid*="login" i]',
  "a",
  "button",
  '[role="dialog"]',
].join(",");
const PROVIDER_LIMIT_SELECTOR = [
  '[role="dialog"]',
  '[role="alert"]',
  '[data-testid*="rate-limit" i]',
  '[data-testid*="provider-limit" i]',
].join(",");
export interface TurnObservation {
  role: "user" | "assistant";
  dataMessageId: string;
  text: string;
  domIndex: number;
}
export function assistantAfter(
  turns: readonly TurnObservation[],
  user: Pick<TurnObservation, "domIndex">,
): TurnObservation | undefined {
  return turns.find((turn) => turn.role === "assistant" && turn.domIndex > user.domIndex);
}
export interface ChipObservation {
  filename: string;
  complete: boolean;
  rootPath: number[];
  seedPath: number[];
}
export interface ArtifactControlLocator {
  locator: Locator;
  kind: "inline" | "entity" | "image";
  label: string;
}
function normalizeMarkdown(value: string, preserveFenceLanguages: boolean): string {
  return value
    .replace(/\r\n?/gu, "\n")
    .split("\n")
    .flatMap((raw) => {
      const line = raw.trim();
      const fence = line.match(/^```([\p{L}\p{N}_.+-]+)?$/u);
      return fence ? (preserveFenceLanguages && fence[1] ? [fence[1]] : []) : [line];
    })
    .join("\n")
    .replace(/\n+/gu, "\n")
    .trim();
}
export function normalizePromptText(value: string): string { return normalizeMarkdown(value, false); }
function promptVariants(expectedPrompt: string): string[] {
  return [...new Set([normalizePromptText(expectedPrompt), normalizeMarkdown(expectedPrompt, true)])];
}
export function renderedBodyForMatch(renderedText: string): string {
  const lines = normalizePromptText(renderedText).split("\n");
  let firstBodyLine = 0;
  while (firstBodyLine + 1 < lines.length
    && FILENAME_PATTERN.test(lines[firstBodyLine]!)
    && /^(?:file|파일)$/iu.test(lines[firstBodyLine + 1]!)) {
    firstBodyLine += 2;
  }
  return lines.slice(firstBodyLine).join("\n").trim();
}
export function renderedTurnMatchesPrompt(
  renderedText: string,
  expectedPrompt: string,
): boolean {
  const renderedBody = renderedBodyForMatch(renderedText);
  return promptVariants(expectedPrompt).some((prompt) => Boolean(prompt
    && (renderedBody === prompt || renderedBody.endsWith(prompt))));
}
// 대형 프롬프트는 ChatGPT 렌더가 원문과 수백 자 단위로 어긋날 수 있어(마크다운 렌더,
// 중간 축약) 완전 일치가 실패한다. 이 loose 판정은 identity가 이미 다른 근거로 좁혀진
// 경우(방금 클릭한 탭의 유일한 새 user 턴, URL 앵커로 연 대화의 유일한 user 턴)에만
// 쓴다 — 열린 탭 텍스트 스캔의 identity 증명으로는 쓰지 않는다.
export const LOOSE_MATCH_MIN_PROMPT_CHARS = 4_096;
const LOOSE_MATCH_EDGE_CHARS = 1_000;
export function renderedTurnMatchesPromptLoose(
  renderedText: string,
  expectedPrompt: string,
): boolean {
  const renderedBody = renderedBodyForMatch(renderedText);
  return promptVariants(expectedPrompt).some((prompt) => (
    prompt.length >= LOOSE_MATCH_MIN_PROMPT_CHARS
    && renderedBody.length >= Math.floor(prompt.length * 0.9)
    && renderedBody.length <= prompt.length + 200
    && renderedBody.slice(0, LOOSE_MATCH_EDGE_CHARS) === prompt.slice(0, LOOSE_MATCH_EDGE_CHARS)
    && renderedBody.slice(-LOOSE_MATCH_EDGE_CHARS) === prompt.slice(-LOOSE_MATCH_EDGE_CHARS)
  ));
}
// 실 UI는 user 턴 마크다운을 렌더링해 문서 전체에서 문법 문자가 소실된다 (2026-07-29
// 라이브 실측: firstDiff=476, tailMatch=0, 길이 98% — 앞뒤 경계 비교로는 잡을 수 없음).
// 이 tier는 identity가 다른 근거로 이미 확정된 유일 턴(방금 클릭한 탭의 단일 새 user 턴,
// URL 앵커 대화의 단일 user 턴)에 대한 최종 sanity로만 쓴다.
export function renderedTurnLengthSane(
  renderedText: string,
  expectedPrompt: string,
): boolean {
  const renderedBody = renderedBodyForMatch(renderedText);
  return promptVariants(expectedPrompt).some((prompt) => (
    prompt.length >= LOOSE_MATCH_MIN_PROMPT_CHARS
    && renderedBody.length >= Math.floor(prompt.length * 0.85)
    && renderedBody.length <= Math.ceil(prompt.length * 1.1)
  ));
}
export function renderedTurnMatchEvidence(
  renderedText: string,
  expectedPrompt: string,
): string {
  const prompt = normalizePromptText(expectedPrompt);
  const renderedBody = renderedBodyForMatch(renderedText);
  const limit = Math.min(prompt.length, renderedBody.length);
  let firstDiff = 0;
  while (firstDiff < limit && prompt[firstDiff] === renderedBody[firstDiff]) firstDiff += 1;
  let tailMatch = 0;
  while (tailMatch < limit
    && prompt[prompt.length - 1 - tailMatch] === renderedBody[renderedBody.length - 1 - tailMatch]) {
    tailMatch += 1;
  }
  return `renderedLen=${renderedBody.length} promptLen=${prompt.length} firstDiff=${firstDiff} tailMatch=${tailMatch}`;
}
const CHIP_OBSERVER_SCRIPT = String.raw`(() => {
  const filename = /[^\s:/\\"'<>|]+\.[a-z0-9]{1,8}\b/iu;
  const visible = (node) => {
    if (!(node instanceof HTMLElement)) return false;
    const rect = node.getBoundingClientRect();
    const style = getComputedStyle(node);
    return rect.width > 0 && rect.height > 0
      && style.display !== "none" && style.visibility !== "hidden" && style.opacity !== "0";
  };
  const domPath = (node) => {
    const result = [];
    let current = node;
    while (current && current !== document.documentElement) {
      const parent = current.parentElement;
      if (!parent) return null;
      result.unshift(Array.prototype.indexOf.call(parent.children, current));
      current = parent;
    }
    return current === document.documentElement ? result : null;
  };
  const nameOf = (node) => node.getAttribute("aria-label") || node.getAttribute("title")
    || node.innerText || node.textContent || "";
  const textbox = document.querySelector(
    '#prompt-textarea,[contenteditable="true"][role="textbox"],textarea',
  );
  const scope = (textbox && (textbox.closest("form") || textbox.parentElement?.parentElement))
    || document.body;
  const seeds = Array.from(scope.querySelectorAll(
    'button,[role="button"],[aria-label],[title]',
  )).filter((node) => visible(node)
    && !node.closest('[data-testid*="profile" i]')
    && filename.test(nameOf(node)));
  return seeds.map((seed) => {
    const accessibleName = nameOf(seed);
    const duplicate = accessibleName.match(
      /([^\s:/\\"'<>|]+)\s+(\(([1-9]|[1-9][0-9])\)\.[a-z0-9]{1,8})\b/iu,
    );
    const match = duplicate
      ? duplicate[1] + " " + duplicate[2]
      : (accessibleName.match(filename)?.[0] || "").trim();
    if (!match) return null;
    // 이미지 타일은 본문 글자 없이 group의 접근성 이름과 제거 버튼에만 파일명을 둔다.
    const group = seed.closest('[role="group"][aria-label]');
    let root = group && filename.test(group.getAttribute("aria-label") || "") ? group : null;
    if (root && (!visible(root) || !root.querySelector('button,[role="button"]'))) return null;
    if (!root) {
      root = seed.matches('button,[role="button"]') ? seed.parentElement : seed;
      while (root) {
        const hasControl = Boolean(root.querySelector('button,[role="button"]'));
        const showsName = filename.test(root.innerText || root.textContent || "");
        if (hasControl && showsName && visible(root)) break;
        root = root.parentElement;
      }
    }
    if (!root) return null;
    const rootPath = domPath(root);
    const seedPath = domPath(seed);
    if (!rootPath || !seedPath) return null;
    const busy = root.getAttribute("aria-busy") === "true"
      || Boolean(root.querySelector('[role="progressbar"],[aria-busy="true"]'));
    const imagePending = Array.from(root.querySelectorAll("img")).some(image => (
      !image.complete || image.naturalWidth === 0 || image.naturalHeight === 0
    ));
    return { filename: match, complete: !busy && !imagePending, rootPath, seedPath };
  }).filter(Boolean);
})()`;
function normalizeLabel(value: string): string {
  return value.normalize("NFC").trim().replace(/\s+/gu, " ").toLocaleLowerCase();
}
/**
 * 알약/라디오 라벨을 power 라벨로 정규화한다. 버전 배지("5.5" 둘째 줄, "6" 첫째 줄)는 제거한다.
 *   "Pro" → "pro" / "Instant\n5.5" → "instant" / "6\nPro" → "pro" / "Extra  High" → "extra high"
 */
export function normalizeIntelligenceLabel(value: string): string {
  const tokens = value.normalize("NFC").trim().split(/\s+/u).filter((token) => token.length > 0);
  return tokens.filter((token) => !VERSION_TOKEN.test(token)).join(" ").toLocaleLowerCase();
}
/** 알약 라벨을 (버전, power)로 나눈다. "6\nPro" → {version:"6", power:"Pro"}, "Pro" → {version:null, power:"Pro"}. */
export function parsePillLabel(value: string): { version: string | null; power: string; display: string } {
  const tokens = value.normalize("NFC").trim().split(/\s+/u).filter((token) => token.length > 0);
  const version = tokens.find((token) => VERSION_TOKEN.test(token)) ?? null;
  const power = tokens.filter((token) => !VERSION_TOKEN.test(token)).join(" ");
  return { version, power, display: tokens.join(" ") };
}
export function normalizeChipStem(value: string): string {
  return normalizeLabel(value)
    .replace(/\.[a-z0-9]{1,8}$/u, "")
    .replace(/\s*\(([1-9]|[1-9][0-9])\)$/u, "");
}
export async function visibleFirst(
  page: Page,
  selectors: readonly string[],
): Promise<Locator | null> {
  for (const selector of selectors) {
    const locator = page.locator(selector).first();
    if (await locator.isVisible().catch(() => false)) return locator;
  }
  return null;
}
export async function findIntelligencePill(
  page: Page,
  intelligenceLabels: readonly string[],
  timeoutMs = 20_000,
): Promise<Locator | null> {
  // 컨테이너 부팅 직후에는 컴포저 pill이 아직 hydrate되지 않았을 수 있어 폴링한다.
  // 1순위: 라벨 집합과 정규화 일치하는 유일한 후보. 2순위(새 UI의 미지 라벨 대비): 텍스트가 있는
  // aria-haspopup 버튼이 form 안에 하나뿐이면 그것 (첨부 "+" 버튼은 텍스트가 없어 제외된다).
  const wanted = new Set(intelligenceLabels.map(normalizeIntelligenceLabel));
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const candidates = page.locator(INTELLIGENCE_PILL_SELECTOR);
    const matches: Locator[] = [];
    const textual: Locator[] = [];
    const count = await candidates.count();
    for (let index = 0; index < count; index += 1) {
      const candidate = candidates.nth(index);
      if (!await candidate.isVisible().catch(() => false)) continue;
      const label = await candidate.innerText().catch(() => "");
      if (label.trim().length > 0) textual.push(candidate);
      if (wanted.has(normalizeIntelligenceLabel(label))) matches.push(candidate);
    }
    if (matches.length === 1) return matches[0]!;
    if (matches.length === 0 && textual.length === 1) return textual[0]!;
    if (Date.now() >= deadline) return null;
    await page.waitForTimeout(500);
  }
}
export async function findIntelligenceOption(
  page: Page,
  targetLabels: readonly string[],
): Promise<Locator | null> {
  const wanted = new Set(targetLabels.map(normalizeIntelligenceLabel));
  const candidates = page.locator(INTELLIGENCE_OPTION_SELECTOR);
  const matches: Locator[] = [];
  const count = await candidates.count();
  for (let index = 0; index < count; index += 1) {
    const candidate = candidates.nth(index);
    if (!await candidate.isVisible().catch(() => false)) continue;
    const label = await candidate.innerText().catch(() => "");
    if (wanted.has(normalizeIntelligenceLabel(label))) matches.push(candidate);
  }
  return matches.length === 1 ? matches[0]! : null;
}
export async function waitForIntelligenceMenu(page: Page, timeoutMs: number): Promise<boolean> {
  // 구 UI(라디오 목록)든 새 UI(피커 본문)든 하나가 보이면 열린 것이다.
  return page.locator(`${INTELLIGENCE_OPTION_SELECTOR}, ${INTELLIGENCE_PICKER_CONTENT_SELECTOR}`).first()
    .waitFor({ state: "visible", timeout: timeoutMs })
    .then(() => true, () => false);
}
/** 새 UI의 생각 강도 슬라이더. 없으면 null (구 UI). */
export interface PowerControl {
  input: Locator;
  slider: Locator;
}
export async function findPowerControl(page: Page): Promise<PowerControl | null> {
  const slider = page.locator(POWER_SLIDER_SELECTOR).first();
  if (await page.locator(POWER_SLIDER_SELECTOR).count() !== 1) return null;
  if (await slider.isVisible() && await slider.getAttribute("tabindex") !== "-1") {
    return { input: slider, slider };
  }
  const input = page.locator(INTELLIGENCE_PICKER_CONTENT_SELECTOR).getByRole("menuitem", { name: "Power", exact: true });
  return await input.count() === 1 && await input.isVisible() ? { input, slider } : null;
}
/** 슬라이더 옆 상태 문구 ("Pro, 5 of 5. Use Left and Right arrow keys…"). */
export async function readPowerStatusText(page: Page): Promise<string> {
  return page.locator(POWER_STATUS_SELECTOR).first().innerText().catch(() => "");
}
/** 새 UI의 "Select model" 항목(aria-expanded로 버전 라디오를 펼친다). 없으면 null. */
export async function findModelSelectItem(page: Page): Promise<Locator | null> {
  const item = page.locator(MODEL_SELECT_ITEM_SELECTOR).first();
  return await item.isVisible().catch(() => false) ? item : null;
}
/** 피커 본문 안 모델 버전 라디오 중 라벨이 일치하는 유일한 것. */
export async function findModelVersionOption(page: Page, label: string): Promise<Locator | null> {
  const wanted = normalizeLabel(label);
  const candidates = page.locator(MODEL_VERSION_OPTION_SELECTOR);
  const matches: Locator[] = [];
  const count = await candidates.count();
  for (let index = 0; index < count; index += 1) {
    const candidate = candidates.nth(index);
    if (!await candidate.isVisible().catch(() => false)) continue;
    const text = await candidate.innerText().catch(() => "");
    if (normalizeLabel(text) === wanted) matches.push(candidate);
  }
  return matches.length === 1 ? matches[0]! : null;
}
export async function readCurrentModelLabel(
  page: Page,
  intelligenceLabels: readonly string[],
): Promise<string> {
  const pill = await findIntelligencePill(page, intelligenceLabels);
  return pill ? pill.innerText().catch(() => "") : "";
}
export async function readTurns(page: Page): Promise<TurnObservation[]> {
  return page.locator(TURN_SELECTOR).evaluateAll((nodes) => nodes.map((node, domIndex) => {
    const element = node as HTMLElement;
    const rect = element.getBoundingClientRect();
    const role = element.getAttribute("data-message-author-role");
    if ((role !== "user" && role !== "assistant") || rect.width <= 0 || rect.height <= 0) return null;
    const dataMessageId = element.getAttribute("data-message-id") || "";
    if (!dataMessageId) return null;
    return {
      role,
      dataMessageId,
      text: (element.innerText || element.textContent || "").trim(),
      domIndex,
    };
  }).filter((item): item is TurnObservation => item !== null));
}
export type TurnMeta = Omit<TurnObservation, "text">;
// innerText는 강제 layout을 유발한다 — 87KB user 턴이 있는 페이지에서 250ms 주기
// 전체 추출은 확정 루프를 분 단위로 늘렸다(2026-07-29 실측). 확정/스캔 루프는 이
// 경량 관측으로 돌고, 텍스트는 매칭 대상 턴만 readTurnTextById로 뽑는다.
export async function readTurnsShallow(page: Page): Promise<TurnMeta[]> {
  return page.locator(TURN_SELECTOR).evaluateAll((nodes) => nodes.map((node, domIndex) => {
    const element = node as HTMLElement;
    const rect = element.getBoundingClientRect();
    const role = element.getAttribute("data-message-author-role");
    if ((role !== "user" && role !== "assistant") || rect.width <= 0 || rect.height <= 0) return null;
    const dataMessageId = element.getAttribute("data-message-id") || "";
    if (!dataMessageId) return null;
    return { role, dataMessageId, domIndex };
  }).filter((item): item is Omit<TurnObservation, "text"> => item !== null));
}
export async function readTurnTextById(page: Page, dataMessageId: string, excludeUiToggle = false): Promise<string | null> {
  return page.locator(TURN_SELECTOR).evaluateAll((nodes, { expected, excludeUiToggle }) => {
    for (const node of nodes) {
      const element = node as HTMLElement;
      if (element.getAttribute("data-message-id") !== expected) continue;
      let text = (element.innerText || element.textContent || "").trim();
      if (excludeUiToggle) {
        // 실 이미지 user 턴의 Show more/Show less는 프롬프트 뒤에 붙는 UI 버튼이다.
        // 버튼의 실제 텍스트가 접미사로 일치할 때만 제외해 프롬프트 본문은 보존한다.
        const toggle = element.querySelector<HTMLElement>('[data-testid="collapsible-user-message-toggle"]');
        const label = (toggle?.innerText || toggle?.textContent || "").trim();
        if (label && text.endsWith(label)) text = text.slice(0, -label.length).trimEnd();
      }
      return text;
    }
    return null;
  }, { expected: dataMessageId, excludeUiToggle });
}
export async function readAssistantAnswer(page: Page, assistantTurnId?: string): Promise<string> {
  let turn = await assistantLocator(page, assistantTurnId);
  if (!turn && assistantTurnId) turn = await assistantLocator(page);
  if (!turn) return "";
  return turn.evaluate((node, filenameSource) => {
    const root = node as HTMLElement;
    let answer = root.innerText || root.textContent || "";
    const filename = new RegExp(filenameSource as string, "iu");
    const controls = root.querySelectorAll('a[download],button,[role="button"]');
    for (const candidate of Array.from(controls)) {
      const control = candidate as HTMLElement;
      const name = control.getAttribute("download") || control.getAttribute("aria-label")
        || control.getAttribute("title") || control.innerText || control.textContent || "";
      const matched = name.match(filename)?.[0];
      if (!matched) continue;
      let block = control;
      for (let hop = 0; hop < 3; hop += 1) {
        const parent = block.parentElement;
        if (!parent || parent === root) break;
        const text = parent.innerText || parent.textContent || "";
        const residue = text.replace(matched, "").replace(/download|다운로드/giu, "")
          .replace(/[\s\p{P}\p{S}]/gu, "");
        if (residue) break;
        block = parent;
      }
      const removable = block.innerText || block.textContent || "";
      if (removable) answer = answer.replace(removable, "");
    }
    return answer.replace(/\n{3,}/gu, "\n\n").trim();
  }, FILENAME_PATTERN.source).catch(() => "");
}
export async function generationActive(page: Page): Promise<boolean> {
  return page.locator('button,[role="button"]').evaluateAll((nodes, source) => {
    const pattern = new RegExp(source as string, "i");
    return nodes.some((node) => {
      const element = node as HTMLElement;
      const rect = element.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return false;
      const label = `${element.getAttribute("aria-label") || ""} ${element.getAttribute("data-testid") || ""} ${element.innerText || element.textContent || ""}`;
      return pattern.test(label);
    });
  }, STOP_PATTERN.source).catch(() => false);
}
export async function observeAttachmentChips(page: Page): Promise<ChipObservation[]> {
  const candidates = await page.evaluate<ChipObservation[]>(CHIP_OBSERVER_SCRIPT)
    .catch(() => [] as ChipObservation[]);
  const groups = new Map<string, ChipObservation[]>();
  for (const candidate of candidates) {
    if (!pathPrefix(candidate.rootPath, candidate.seedPath)) continue;
    const key = candidate.rootPath.join(".");
    const group = groups.get(key) ?? [];
    group.push(candidate);
    groups.set(key, group);
  }
  const roots: ChipObservation[] = [];
  for (const group of groups.values()) {
    const labels = new Set(group.map((item) => normalizeLabel(item.filename)));
    if (labels.size !== 1 || labels.has("")) continue;
    roots.push([...group].sort((left, right) => (
      right.seedPath.length - left.seedPath.length || comparePath(left.seedPath, right.seedPath)
    ))[0]!);
  }
  return roots.filter((candidate) => !roots.some((other) => (
    other !== candidate
    && candidate.rootPath.length < other.rootPath.length
    && pathPrefix(candidate.rootPath, other.rootPath)
  ))).sort((left, right) => comparePath(left.rootPath, right.rootPath));
}
export async function answerActionVisible(
  page: Page,
  assistantTurnId?: string,
): Promise<boolean> {
  const turn = await assistantLocator(page, assistantTurnId);
  if (!turn) return false;
  // 실 UI(2026-07-27 실측)에서 답변 액션 바(copy 등)는 [data-message-author-role] 노드
  // 내부가 아니라 그 조상의 형제로 렌더된다. turn에서 조상으로 올라가며 찾되,
  // 다른 turn까지 포함하는 넓은 컨테이너(대화 전체)에 도달하면 중단한다.
  // 주의: evaluate 콜백 안에 내부 함수 선언 금지 — tsx(esbuild) keepNames가 __name()으로
  // 감싸는데 브라우저에는 __name이 없어 ReferenceError가 난다 (칩 스캐너가 String.raw인 이유).
  return turn.evaluate((node, copySelector) => {
    let scope: Element | null = node;
    for (let hop = 0; hop < 6 && scope; hop += 1) {
      if (scope.querySelectorAll("[data-message-author-role]").length > 1) break;
      for (const control of Array.from(scope.querySelectorAll(copySelector))) {
        const rect = control.getBoundingClientRect();
        const style = getComputedStyle(control);
        if (rect.width > 0 && rect.height > 0
          && style.display !== "none" && style.visibility !== "hidden") return true;
      }
      scope = scope.parentElement;
    }
    return false;
  }, COPY_CONTROL_SELECTOR).catch(() => false);
}
export async function artifactControlLocators(
  page: Page,
  assistantTurnId?: string,
  strict = false,
): Promise<ArtifactControlLocator[]> {
  let turn = await assistantLocator(page, assistantTurnId);
  if (!turn && assistantTurnId && !strict) turn = await assistantLocator(page);
  if (!turn) return [];
  const candidates = turn.locator(ARTIFACT_CONTROL_SELECTOR);
  const result: ArtifactControlLocator[] = [];
  for (let index = 0; index < await candidates.count(); index += 1) {
    const candidate = candidates.nth(index);
    if (!await candidate.isVisible().catch(() => false)) continue;
    const descriptor = await candidate.evaluate((node, filenameSource) => {
      const inline = node.matches("a[download]");
      const accessible = node.getAttribute("aria-label")
        || node.getAttribute("title")
        || node.textContent
        || "";
      const downloadName = inline ? node.getAttribute("download") || "" : "";
      const pattern = new RegExp(filenameSource as string, "iu");
      const filename = (downloadName.match(pattern)?.[0] || accessible.match(pattern)?.[0] || "").trim();
      return { inline, filename };
    }, FILENAME_PATTERN.source).catch(() => ({ inline: false, filename: "" }));
    if (!descriptor.filename) continue;
    result.push({
      locator: candidate,
      kind: descriptor.inline ? "inline" : "entity",
      label: descriptor.filename,
    });
  }
  return result;
}
export async function artifactControls(
  page: Page,
  assistantTurnId?: string,
): Promise<ArtifactControl[]> {
  const locators = await artifactControlLocators(page, assistantTurnId);
  return locators.map((control, index) => ({ index, label: control.label }));
}
export async function readinessObservation(
  page: Page,
  intelligenceLabels: readonly string[],
  settleTimeoutMs = 25_000,
): Promise<{
  state: "ready" | "needs_login" | "provider_limit" | "unknown";
  modelLabel: string;
}> {
  // 컨테이너 부팅 직후에는 chatgpt.com이 아직 로드 중이라 단발 관찰이 unknown이 된다.
  // unknown이 아닌 상태가 관찰될 때까지 폴링한다.
  const deadline = Date.now() + settleTimeoutMs;
  for (;;) {
    const observed = await readinessObservationOnce(page, intelligenceLabels);
    if (observed.state !== "unknown" || Date.now() >= deadline) return observed;
    await page.waitForTimeout(500);
  }
}
async function readinessObservationOnce(
  page: Page,
  intelligenceLabels: readonly string[],
): Promise<{
  state: "ready" | "needs_login" | "provider_limit" | "unknown";
  modelLabel: string;
}> {
  let state: "ready" | "needs_login" | "provider_limit" | "unknown" = "unknown";
  const loginNodes = page.locator(LOGIN_CONTROL_SELECTOR);
  for (let index = 0; index < await loginNodes.count(); index += 1) {
    const candidate = loginNodes.nth(index);
    if (!await candidate.isVisible().catch(() => false)) continue;
    if (/\blog in\b|\bsign up\b|로그인|가입/i.test(await locatorLabel(candidate))) {
      state = "needs_login";
      break;
    }
  }
  if (state === "unknown") {
    const blocking = page.locator(PROVIDER_LIMIT_SELECTOR);
    for (let index = 0; index < await blocking.count(); index += 1) {
      const candidate = blocking.nth(index);
      if (!await candidate.isVisible().catch(() => false)) continue;
      if (/too many requests|rate limit|message cap|try again later|요청 한도|잠시 후/i.test(await locatorLabel(candidate))) {
        state = "provider_limit";
        break;
      }
    }
  }
  if (state === "unknown" && await visibleFirst(page, COMPOSER_SELECTORS)) state = "ready";
  return { state, modelLabel: await readCurrentModelLabel(page, intelligenceLabels) };
}
async function locatorLabel(locator: Locator): Promise<string> {
  const aria = await locator.getAttribute("aria-label").catch(() => null);
  if (aria) return aria;
  return locator.innerText().catch(() => "");
}
async function assistantLocator(page: Page, dataMessageId?: string): Promise<Locator | null> {
  const turns = page.locator(ASSISTANT_TURN_SELECTOR);
  if (dataMessageId) {
    for (let index = 0; index < await turns.count(); index += 1) {
      const turn = turns.nth(index);
      if (await turn.getAttribute("data-message-id").catch(() => null) !== dataMessageId) continue;
      if (await turn.isVisible().catch(() => false)) return turn;
    }
    return null;
  }
  for (let index = await turns.count() - 1; index >= 0; index -= 1) {
    const turn = turns.nth(index);
    if (await turn.isVisible().catch(() => false)) return turn;
  }
  return null;
}
function pathPrefix(prefix: number[], value: number[]): boolean {
  return prefix.length <= value.length && prefix.every((item, index) => item === value[index]);
}
function comparePath(left: number[], right: number[]): number {
  const length = Math.min(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    if (left[index] !== right[index]) return left[index]! - right[index]!;
  }
  return left.length - right.length;
}
