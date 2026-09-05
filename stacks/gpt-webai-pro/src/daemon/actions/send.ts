import type { Page } from "playwright-core";
import { GwpError, type SendPhase } from "../../shared/errors.js";
import type { LabelConfig, SendParams, SendProgress, SendResult, SendStep } from "../../shared/types.js";
import {
  COMPOSER_SELECTORS,
  FILE_INPUT_SELECTOR,
  SEND_BUTTON_SELECTORS,
  UPLOAD_BUTTON_SELECTORS,
  type ChipObservation,
  normalizeChipStem,
  renderedTurnLengthSane,
  renderedTurnMatchesPrompt,
  renderedTurnMatchesPromptLoose,
  renderedTurnMatchEvidence,
  observeAttachmentChips,
  readTurnsShallow,
  readTurnTextById,
  visibleFirst,
} from "../selectors.js";
import type { BrowserSession } from "../browser.js";
import { ensureIntelligence } from "./model.js";
import { composeImagePrompt, imageLabels, imageSentTurnMatches } from "./images.js";
import { captureInspection } from "./inspect.js";
const HEARTBEAT_MS = 2_500;
// 관측이 경량(readTurnsShallow, 텍스트는 매칭 후보만)이라 짧은 주기를 유지할 수 있다.
const CONFIRM_POLL_MS = 250;
export type SendProgressEmitter = (progress: SendProgress) => void;
export async function sendMessage(
  session: BrowserSession,
  params: SendParams,
  labels: LabelConfig,
  onProgress?: SendProgressEmitter,
  outboxDir?: string,
): Promise<SendResult> {
  const startedAt = Date.now();
  const state = {
    step: "navigate" as SendStep,
    phase: "pre_click" as SendPhase,
    stepStartedAt: startedAt,
    pendingUserTurnId: undefined as string | undefined,
    pendingConversationUrl: undefined as string | undefined,
    preClickBaseline: undefined as string[] | undefined,
    matchDebug: undefined as string | undefined,
  };
  const emit = () => {
    onProgress?.({
      step: state.step,
      phase: state.phase,
      elapsedMs: Date.now() - startedAt,
      stepElapsedMs: Date.now() - state.stepStartedAt,
      ...(state.pendingUserTurnId ? { pendingUserTurnId: state.pendingUserTurnId } : {}),
      ...(state.pendingConversationUrl
        ? { pendingConversationUrl: state.pendingConversationUrl }
        : {}),
      ...(state.preClickBaseline ? { preClickBaseline: state.preClickBaseline } : {}),
      ...(state.matchDebug ? { matchDebug: state.matchDebug } : {}),
    });
  };
  const step = (name: SendStep, phase: SendPhase = state.phase) => {
    state.step = name;
    state.phase = phase;
    state.stepStartedAt = Date.now();
    emit();
  };
  const heartbeat = setInterval(emit, HEARTBEAT_MS);
  let page: Page | null = null;
  let clickStarted = false;
  try {
    emit();
    if (params.imageCount !== undefined && (!Number.isInteger(params.imageCount) || params.imageCount < 1 || params.imageCount > 5)) {
      throw new GwpError("compose_failed", "imageCount must be from 1 through 5", { phase: "pre_click" });
    }
    if (params.imageCount && params.conversationUrl) throw new GwpError("compose_failed", "image batches require a new conversation", { phase: "pre_click" });
    page = params.conversationUrl
      ? await session.open(params.conversationUrl)
      : await session.newConversation();
    step("ensure_model");
    let modelLabel = await ensureIntelligence(page, params.imageCount ? imageLabels(labels) : labels);
    step("compose");
    if (params.imageCount) {
      step("select_image_tool");
      await composeImagePrompt(page, params.prompt);
    } else await fillComposer(page, params.prompt);
    if (params.files.length > 0) {
      step("attach");
      await attachFiles(page, params.files.map((file) => file.containerPath));
      step("verify_chips");
      await waitForExpectedChips(page, params.files.map((file) => file.name), 30_000);
    }
    if (params.imageCount) modelLabel = await ensureIntelligence(page, imageLabels(labels));
    step("baseline");
    const baseline = await readTurnsShallow(page);
    const baselineIds = new Set(baseline.map((turn) => turn.dataMessageId));
    step("wait_send_button");
    // 대형 프롬프트 삽입 직후엔 렌더 jank로 send 버튼 활성화가 수십 초 늦을 수 있다.
    const send = await waitForSendButton(
      page,
      Math.max(30_000, Math.min(180_000, params.prompt.length * 2)),
    );
    if (!send) throw new GwpError("compose_failed", "send button is not ready", { phase: "pre_click" });
    state.preClickBaseline = [...baselineIds];
    clickStarted = true;
    step("click", "post_click");
    try {
      await send.click({ timeout: 10_000 });
    } catch (error) {
      throw new GwpError("click_uncertain", `send click did not return cleanly: ${String(error)}`, {
        phase: "post_click",
        cause: error,
      });
    }
    step("confirm");
    // 확정 창: supervisor가 progress 알림으로 살아있음을 확인하며 기다리므로 넉넉히 잡는다.
    // 94KB 프롬프트가 렌더 jank로 클릭·확정에 분 단위를 쓰는 실측(2026-07-29)이 근거.
    const deadline = Date.now() + envMs("GWP_CONFIRM_WINDOW_MS", 300_000);
    while (Date.now() < deadline) {
      const turns = await readTurnsShallow(page);
      const newUsers = turns.filter((turn) => (
        turn.role === "user" && !baselineIds.has(turn.dataMessageId)
      ));
      if (newUsers.length > 0) {
        // durable 앵커는 텍스트 매칭과 무관하게 관측 즉시 확보한다 (§5.2).
        if (!state.pendingUserTurnId) state.pendingUserTurnId = newUsers[0]!.dataMessageId;
        state.pendingConversationUrl = page.url();
        emit();
      }
      let user: (typeof newUsers)[number] | undefined;
      let matchedBy: SendResult["matchedBy"];
      for (const candidate of newUsers) {
        const text = await readTurnTextById(page, candidate.dataMessageId, Boolean(params.imageCount));
        if (text === null) continue;
        if (renderedTurnMatchesPrompt(text, params.prompt)
          || (params.imageCount && imageSentTurnMatches(text, params.prompt))) {
          user = candidate;
          matchedBy = "strict";
          break;
        }
        // 방금 클릭한 새 대화 탭의 유일한 새 user 턴 = 우리 전송. loose 판정 허용.
        if (newUsers.length === 1 && renderedTurnMatchesPromptLoose(text, params.prompt)) {
          user = candidate;
          matchedBy = "loose";
          break;
        }
        // 실 UI는 user 턴 마크다운을 렌더링해 문서 전반의 문법 문자가 소실된다 —
        // edge 비교(loose)도 실패한다 (2026-07-29 라이브: firstDiff=476, tailMatch=0).
        // identity는 이미 이 탭+baseline+단일 새 턴으로 확정이므로 길이 sanity만 본다.
        if (newUsers.length === 1 && renderedTurnLengthSane(text, params.prompt)) {
          user = candidate;
          matchedBy = "single_turn";
          break;
        }
        state.matchDebug = renderedTurnMatchEvidence(text, params.prompt);
      }
      const assistant = user && turns.find((turn) => (
        turn.role === "assistant"
        && !baselineIds.has(turn.dataMessageId)
        && turn.domIndex > user.domIndex
      ));
      const conversationUrl = page.url();
      // 착지 확정은 user 턴 + 비루트 대화 URL로 충분하다. assistant 턴은 Pro+대형 첨부에서
      // 확정 창(300s) 안에 렌더되지 않을 수 있으므로 요구하지 않는다 — 있으면 넘기고,
      // 없으면 poll이 생성 완료를 판정한다 (reconcile turn_anchor와 동일 기준, DESIGN §5.2).
      if (user && session.isConversationUrl(conversationUrl)) {
        return {
          conversationUrl,
          userTurnId: user.dataMessageId,
          modelLabel,
          ...(assistant ? { assistantTurnId: assistant.dataMessageId } : {}),
          ...(matchedBy ? { matchedBy } : {}),
        };
      }
      await page.waitForTimeout(CONFIRM_POLL_MS);
    }
    throw new GwpError(
      "click_uncertain",
      "new user turn, following assistant turn, and non-root conversation URL were not all observed"
        + (state.matchDebug ? ` (${state.matchDebug})` : ""),
      { phase: "post_click" },
    );
  } catch (error) {
    const gwp = error instanceof GwpError ? error : null;
    const diagnostic = gwp?.kind === "chip_mismatch" && page && outboxDir
      ? await captureInspection(page, outboxDir)
        .then(result => `; attachment diagnostic: ${result.snapshotPath}`, cause => `; attachment capture failed: ${String(cause)}`)
      : "";
    throw new GwpError(
      gwp?.kind ?? (clickStarted ? "click_uncertain" : "compose_failed"),
      (gwp?.detail ?? String(error)) + diagnostic,
      {
        // Entering send.click() transfers authority irrevocably to post-click.
        phase: clickStarted ? "post_click" : gwp?.phase ?? "pre_click",
        cause: error,
        ...(gwp?.networkEvidence ? { networkEvidence: true } : {}),
        ...(clickStarted && page
          ? {
              ...(state.pendingUserTurnId ? { pendingUserTurnId: state.pendingUserTurnId } : {}),
              pendingConversationUrl: page.url(),
              preClickBaseline: state.preClickBaseline ?? [],
            }
          : {}),
      },
    );
  } finally {
    clearInterval(heartbeat);
  }
}
async function fillComposer(page: Page, prompt: string): Promise<void> {
  const composer = await visibleFirst(page, COMPOSER_SELECTORS);
  if (!composer) throw new GwpError("compose_failed", "composer is not visible", { phase: "pre_click" });
  await composer.click({ timeout: 10_000 });
  try {
    // 대형 프롬프트는 ProseMirror 반영이 느리다 — fill 창을 크기에 비례해 늘린다.
    await composer.fill(prompt, {
      timeout: Math.max(10_000, Math.min(60_000, prompt.length)),
    });
  } catch {
    await page.keyboard.press(process.platform === "darwin" ? "Meta+A" : "Control+A");
    await page.keyboard.insertText(prompt);
  }
}
async function attachFiles(page: Page, files: string[]): Promise<void> {
  const input = page.locator(FILE_INPUT_SELECTOR).first();
  if (await input.count() > 0) {
    await input.setInputFiles(files, { timeout: 30_000 });
    return;
  }
  const upload = await visibleFirst(page, UPLOAD_BUTTON_SELECTORS);
  if (!upload) throw new GwpError("compose_failed", "upload control is not available", { phase: "pre_click" });
  const chooser = page.waitForEvent("filechooser", { timeout: 10_000 });
  await upload.click({ timeout: 10_000 });
  await (await chooser).setFiles(files);
}
async function waitForExpectedChips(
  page: Page,
  expectedNames: string[],
  timeoutMs: number,
): Promise<void> {
  const expected = counts(expectedNames.map(normalizeChipStem));
  const deadline = Date.now() + timeoutMs;
  let latest: ChipObservation[] = [];
  while (Date.now() < deadline) {
    latest = await observeAttachmentChips(page);
    const observed = counts(latest.map((chip) => normalizeChipStem(chip.filename)));
    if (latest.length === expectedNames.length
      && latest.every((chip) => chip.complete)
      && sameCounts(expected, observed)) return;
    await page.waitForTimeout(250);
  }
  throw new GwpError(
    "chip_mismatch",
    `attachment chips did not match: expected=${JSON.stringify([...expected])} observed=${JSON.stringify(latest)}`,
    { phase: "pre_click" },
  );
}
async function waitForSendButton(page: Page, timeoutMs: number) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const send = await visibleFirst(page, SEND_BUTTON_SELECTORS);
    if (send && await send.isEnabled().catch(() => false)) return send;
    await page.waitForTimeout(250);
  }
  return null;
}
function counts(values: string[]): Map<string, number> {
  const result = new Map<string, number>();
  for (const value of values) result.set(value, (result.get(value) ?? 0) + 1);
  return result;
}
function sameCounts(left: Map<string, number>, right: Map<string, number>): boolean {
  return left.size === right.size && [...left].every(([key, value]) => right.get(key) === value);
}
function envMs(name: string, fallback: number): number {
  const value = Number(process.env[name]);
  return Number.isFinite(value) && value > 0 ? value : fallback;
}
