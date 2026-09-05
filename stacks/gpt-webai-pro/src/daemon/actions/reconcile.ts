import type { Page } from "playwright-core";
import type { ReconcileParams, ReconcileResult } from "../../shared/types.js";
import type { BrowserSession } from "../browser.js";
import {
  assistantAfter,
  readTurns,
  readTurnsShallow,
  readTurnTextById,
  renderedTurnLengthSane,
  renderedTurnMatchesPrompt,
  renderedTurnMatchesPromptLoose,
  renderedTurnMatchEvidence,
} from "../selectors.js";
import { imageSentTurnMatches } from "./images.js";
// 무거운 대화는 domcontentloaded 후 턴 렌더까지 수 초가 걸린다. 렌더 전 스캔은
// 빈 대화로 읽혀 거짓 부재 증명(→중복 재전송)이 된다 — 2026-07-29 라이브 사고.
async function waitForTurnRender(page: Page): Promise<void> {
  const deadline = Date.now() + envMs("GWP_RECONCILE_RENDER_WAIT_MS", 20_000);
  while (Date.now() < deadline) {
    const turns = await readTurnsShallow(page).catch(() => null);
    if (turns === null || turns.length > 0) return;
    await page.waitForTimeout(500);
  }
}
function envMs(name: string, fallback: number): number {
  const value = Number(process.env[name]);
  return Number.isFinite(value) && value > 0 ? value : fallback;
}
export async function reconcileSend(
  session: BrowserSession,
  params: ReconcileParams,
): Promise<ReconcileResult> {
  const openPages = await session.relevantPages();
  const anchors = uniqueStrings([
    params.pendingConversationUrl,
    params.conversationUrl,
  ]);
  if (params.pendingUserTurnId) {
    const observed = await scanUserTurnAnchor(session, openPages, params.pendingUserTurnId);
    if (observed.found) return observed;
    for (const anchor of anchors.filter((url) => session.isConversationUrl(url))) {
      const opened = await session.open(anchor).catch(() => undefined);
      if (opened) await waitForTurnRender(opened);
      const rebound = await scanUserTurnAnchor(
        session,
        await session.relevantPages(),
        params.pendingUserTurnId,
      );
      if (rebound.found) return rebound;
    }
    return { found: false, proven: false };
  }
  for (const anchor of anchors) {
    const exact = openPages.filter((page) => page.url() === anchor);
    if (exact.length > 0 && session.isConversationUrl(anchor)) {
      for (const page of exact) await waitForTurnRender(page);
      return scanPages(session, exact, params, true, true);
    }
  }
  const navigationUrl = anchors.find((url) => session.isConversationUrl(url));
  if (navigationUrl) {
    try {
      const opened = await session.open(navigationUrl);
      if (session.isConversationUrl(opened.url())) {
        await waitForTurnRender(opened);
        return scanPages(session, [opened], params, true, true);
      }
    } catch {
      // Losing an anchor cannot authorize either text-only binding or retry.
    }
  }
  const fallbackPages = await session.relevantPages();
  // Without a durable anchor, an empty surviving tab cannot prove that a
  // post-click send did not land in a tab lost with the daemon/browser.
  return scanPages(session, fallbackPages, params, false, false);
}
async function scanPages(
  session: BrowserSession,
  pages: readonly Page[],
  params: ReconcileParams,
  canProveAbsence: boolean,
  // URL 앵커(우리 DB에 기록된 우리 대화)로 좁혀진 스캔에서만 loose 매칭을 허용한다.
  // 앵커 없는 열린 탭 스캔에서 loose는 identity 증명이 아니므로 금지 (§5.3).
  anchored: boolean,
): Promise<ReconcileResult> {
  const baseline = new Set(params.preClickBaseline ?? []);
  let unreadable = false;
  let ambiguous = false;
  let unboundMatch = false;
  let unmatchedNewUsers = false;
  let readable = 0;
  let evidence: string | undefined;
  const matches: ReconcileResult[] = [];
  for (const page of pages) {
    const conversationUrl = page.url();
    const turns = await readTurns(page).catch(() => null);
    if (!turns) {
      unreadable = true;
      continue;
    }
    if (anchored && turns.length === 0) {
      // /c/ 대화가 턴 0개로 읽히면 아직 렌더 전이거나 죽은 뷰다 — 진짜 대화라면 턴이
      // 반드시 존재한다. 이런 페이지는 부재 증명 권한이 없다 (2026-07-29 라이브:
      // 렌더 대기 없던 스캔이 빈 대화를 부재로 오판 → attempt 2 중복 전송).
      unreadable = true;
      continue;
    }
    readable += 1;
    const newUsers = turns.filter((turn) => (
      turn.role === "user" && !baseline.has(turn.dataMessageId)
    ));
    if (params.imageCount) {
      // 전송 확인과 동일하게 이미지 도구의 공백 렌더·접기 버튼을 처리한다.
      // 읽는 도중 턴이 사라지면 부재 증명이나 다른 턴 결속에 사용하지 않는다.
      let missingTurn = false;
      for (const turn of newUsers) {
        const text = await readTurnTextById(page, turn.dataMessageId, true).catch(() => null);
        if (text === null) { missingTurn = true; break; }
        turn.text = text;
      }
      if (missingTurn) { unreadable = true; continue; }
    }
    let matchedBy: ReconcileResult["matchedBy"];
    let promptMatches = newUsers.filter((turn) => (
      renderedTurnMatchesPrompt(turn.text, params.prompt)
      || (params.imageCount && imageSentTurnMatches(turn.text, params.prompt))
    ));
    if (promptMatches.length > 0) matchedBy = "strict";
    if (promptMatches.length === 0 && anchored && newUsers.length === 1) {
      promptMatches = newUsers.filter((turn) => (
        renderedTurnMatchesPromptLoose(turn.text, params.prompt)
      ));
      if (promptMatches.length > 0) matchedBy = "loose";
    }
    // URL 앵커 대화(우리 DB가 가리키는 우리 대화)에 user 턴이 정확히 하나면 그 턴이
    // 곧 우리 전송이다 — 마크다운 렌더로 텍스트 비교가 전부 실패해도 길이 sanity로 회수.
    if (promptMatches.length === 0 && anchored && newUsers.length === 1
      && turns.filter((turn) => turn.role === "user").length === 1) {
      promptMatches = newUsers.filter((turn) => (
        renderedTurnLengthSane(turn.text, params.prompt)
      ));
      if (promptMatches.length > 0) matchedBy = "single_turn";
    }
    if (promptMatches.length === 0 && newUsers.length > 0) {
      // baseline 밖의 새 user 턴이 매칭 없이 존재한다 = 우리 전송이 렌더 변형으로
      // 매칭에 실패했을 수 있다. 이 상태로 부재를 증명하면 재전송(중복)을 승인하게
      // 되므로 절대 금지 (§5.3 fail-closed).
      unmatchedNewUsers = true;
      if (anchored) evidence ??= renderedTurnMatchEvidence(newUsers[0]!.text, params.prompt);
    }
    if (promptMatches.length > 1) {
      ambiguous = true;
      continue;
    }
    const user = promptMatches[0];
    if (!user) continue;
    if (!session.isConversationUrl(conversationUrl)) {
      unboundMatch = true;
      continue;
    }
    const assistant = assistantAfter(turns, user);
    matches.push({
      found: true,
      conversationUrl,
      userTurnId: user.dataMessageId,
      ...(assistant ? { assistantTurnId: assistant.dataMessageId } : {}),
      proven: true,
      ...(matchedBy ? { matchedBy } : {}),
    });
  }
  if (matches.length === 1 && !ambiguous && !unboundMatch) return matches[0]!;
  if (matches.length > 1 || unreadable || ambiguous || unboundMatch) {
    return { found: false, proven: false, ...(evidence ? { evidence } : {}) };
  }
  return {
    found: false,
    proven: canProveAbsence && pages.length > 0 && readable === pages.length && !unmatchedNewUsers,
    ...(evidence ? { evidence } : {}),
  };
}
async function scanUserTurnAnchor(
  session: BrowserSession,
  pages: readonly Page[],
  pendingUserTurnId: string,
): Promise<ReconcileResult> {
  let unreadable = false;
  const matches: Array<{ page: Page; turns: Awaited<ReturnType<typeof readTurns>>; domIndex: number }> = [];
  for (const page of pages) {
    const turns = await readTurns(page).catch(() => null);
    if (!turns) {
      unreadable = true;
      continue;
    }
    for (const user of turns.filter((turn) => (
      turn.role === "user" && turn.dataMessageId === pendingUserTurnId
    ))) matches.push({ page, turns, domIndex: user.domIndex });
  }
  if (unreadable || matches.length !== 1) return { found: false, proven: false };
  const match = matches[0]!;
  const conversationUrl = match.page.url();
  if (!session.isConversationUrl(conversationUrl)) return { found: false, proven: false };
  const assistant = assistantAfter(match.turns, match);
  return {
    found: true,
    conversationUrl,
    userTurnId: pendingUserTurnId,
    ...(assistant ? { assistantTurnId: assistant.dataMessageId } : {}),
    proven: true,
    matchedBy: "turn_anchor",
  };
}
function uniqueStrings(values: Array<string | undefined>): string[] {
  return [...new Set(values.filter((value): value is string => Boolean(value)))];
}
