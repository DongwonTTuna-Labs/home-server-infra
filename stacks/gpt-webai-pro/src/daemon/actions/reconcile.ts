import type { Page } from "playwright-core";

import { sha256Text } from "../../shared/fsx.js";
import type { ReconcileParams, ReconcileResult } from "../../shared/types.js";
import type { BrowserSession } from "../browser.js";
import { readTurns } from "../selectors.js";

interface ScanOutcome {
  result: ReconcileResult;
  page: Page | null;
}

export async function reconcileSend(
  session: BrowserSession,
  params: ReconcileParams,
): Promise<ReconcileResult> {
  const openPages = await session.relevantPages();
  if (!params.conversationUrl) {
    // URL/turn identity가 아직 없는 창구간에는 prompt sha만 권위가 있다. send와 이
    // 전체 판정은 daemon mutation 큐가 직렬화하므로 클릭 전 DOM을 관찰할 수 없다.
    // 큐를 지난 뒤 같은 prompt가 둘 이상이면 어느 요청인지 증명할 수 없으므로 닫힌다.
    return (await scanPages(session, openPages, params.promptSha256)).result;
  }

  const alreadyOpen = openPages.filter((page) => page.url() === params.conversationUrl);
  if (alreadyOpen.length > 0) {
    return (await scanPages(session, alreadyOpen, params.promptSha256)).result;
  }

  let opened: Page;
  try {
    opened = await session.open(params.conversationUrl);
  } catch {
    const fallback = await scanPages(
      session,
      await session.relevantPages(),
      params.promptSha256,
    );
    const result = fallback.result;
    return result.found ? result : { found: false, proven: false };
  }

  if (!session.isConversationUrl(opened.url())) {
    const fallback = await scanPages(
      session,
      await session.relevantPages(),
      params.promptSha256,
    );
    const result = fallback.result;
    return result.found ? result : { found: false, proven: false };
  }

  return (await scanPages(session, [opened], params.promptSha256)).result;
}

async function scanPages(
  session: BrowserSession,
  pages: readonly Page[],
  promptSha256: string,
): Promise<ScanOutcome> {
  let observedConversation = false;
  let unreadable = false;
  let unboundMatch = false;
  const matches: Array<{ page: Page; result: ReconcileResult }> = [];
  for (const page of pages) {
    const conversationUrl = page.url();
    const turns = await readTurns(page).catch(() => null);
    if (!turns) {
      unreadable = true;
      continue;
    }
    const user = [...turns].reverse().find((turn) => (
      turn.role === "user" && sha256Text(turn.text) === promptSha256
    ));
    if (!session.isConversationUrl(conversationUrl)) {
      if (user) unboundMatch = true;
      continue;
    }
    observedConversation = true;
    if (!user) continue;
    const assistant = turns.find((turn) => (
      turn.role === "assistant" && turn.domIndex > user.domIndex
    ));
    matches.push({
      result: {
        found: true,
        conversationUrl,
        userTurnId: user.dataMessageId,
        ...(assistant ? { assistantTurnId: assistant.dataMessageId } : {}),
        proven: true,
      },
      page,
    });
  }
  if (matches.length === 1 && !unreadable && !unboundMatch) return matches[0]!;
  if (matches.length > 1 || unreadable || unboundMatch) {
    return { result: { found: false, proven: false }, page: null };
  }
  return { result: { found: false, proven: observedConversation }, page: null };
}
