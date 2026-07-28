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
].join(",");
export const STOP_PATTERN = /stop generating|stop responding|stop answering|stop-button|중지|정지/i;
export const FILENAME_PATTERN = /[^\s:/\\"'<>|]+\.[a-z0-9]{1,8}\b/iu;
const ARTIFACT_CONTROL_SELECTOR = [
  "a[download]",
  "button",
  '[role="button"]',
].join(",");
const COPY_CONTROL_SELECTOR = [
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
  kind: "inline" | "entity";
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
export function renderedTurnMatchesPrompt(
  renderedText: string,
  expectedPrompt: string,
): boolean {
  const prompts = new Set([normalizePromptText(expectedPrompt), normalizeMarkdown(expectedPrompt, true)]);
  const lines = normalizePromptText(renderedText).split("\n");
  let firstBodyLine = 0;
  while (firstBodyLine + 1 < lines.length
    && FILENAME_PATTERN.test(lines[firstBodyLine]!)
    && /^(?:file|파일)$/iu.test(lines[firstBodyLine + 1]!)) {
    firstBodyLine += 2;
  }
  const renderedBody = lines.slice(firstBodyLine).join("\n").trim();
  return [...prompts].some((prompt) => Boolean(prompt
    && (renderedBody === prompt || renderedBody.endsWith(prompt))));
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
    let root = seed.matches('button,[role="button"]') ? seed.parentElement : seed;
    while (root) {
      const hasControl = Boolean(root.querySelector('button,[role="button"]'));
      const showsName = filename.test(root.innerText || root.textContent || "");
      if (hasControl && showsName && visible(root)) break;
      root = root.parentElement;
    }
    if (!root) return null;
    const rootPath = domPath(root);
    const seedPath = domPath(seed);
    if (!rootPath || !seedPath) return null;
    const busy = root.getAttribute("aria-busy") === "true"
      || Boolean(root.querySelector('[role="progressbar"],[aria-busy="true"]'));
    return { filename: match, complete: !busy, rootPath, seedPath };
  }).filter(Boolean);
})()`;
function normalizeLabel(value: string): string {
  return value.normalize("NFC").trim().replace(/\s+/gu, " ").toLocaleLowerCase();
}
export function normalizeIntelligenceLabel(value: string): string {
  const firstLine = value.normalize("NFC").trim().split(/\r?\n/u)[0] ?? "";
  return firstLine.trim().replace(/\s+/gu, " ").toLocaleLowerCase();
}
export function normalizeChipStem(value: string): string {
  return normalizeLabel(value)
    .replace(/\.[a-z0-9]{1,8}$/u, "")
    .replace(/ \(([1-9]|[1-9][0-9])\)$/u, "");
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
  const wanted = new Set(intelligenceLabels.map(normalizeIntelligenceLabel));
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const candidates = page.locator(INTELLIGENCE_PILL_SELECTOR);
    const matches: Locator[] = [];
    const count = await candidates.count();
    for (let index = 0; index < count; index += 1) {
      const candidate = candidates.nth(index);
      if (!await candidate.isVisible().catch(() => false)) continue;
      const label = await candidate.innerText().catch(() => "");
      if (wanted.has(normalizeIntelligenceLabel(label))) matches.push(candidate);
    }
    if (matches.length === 1) return matches[0]!;
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
  return page.locator(INTELLIGENCE_OPTION_SELECTOR).first()
    .waitFor({ state: "visible", timeout: timeoutMs })
    .then(() => true, () => false);
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
): Promise<ArtifactControlLocator[]> {
  let turn = await assistantLocator(page, assistantTurnId);
  if (!turn && assistantTurnId) turn = await assistantLocator(page);
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
