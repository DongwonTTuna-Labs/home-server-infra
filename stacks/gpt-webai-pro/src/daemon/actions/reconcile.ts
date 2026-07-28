import type { Page } from "playwright-core";
import type { ReconcileParams, ReconcileResult } from "../../shared/types.js";
import type { BrowserSession } from "../browser.js";
import { assistantAfter, readTurns, renderedTurnMatchesPrompt } from "../selectors.js";
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
      await session.open(anchor).catch(() => undefined);
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
      return scanPages(session, exact, params, true);
    }
  }
  const navigationUrl = anchors.find((url) => session.isConversationUrl(url));
  if (navigationUrl) {
    try {
      const opened = await session.open(navigationUrl);
      if (session.isConversationUrl(opened.url())) {
        return scanPages(session, [opened], params, true);
      }
    } catch {
      // Losing an anchor cannot authorize either text-only binding or retry.
    }
  }
  const fallbackPages = await session.relevantPages();
  // Without a durable anchor, an empty surviving tab cannot prove that a
  // post-click send did not land in a tab lost with the daemon/browser.
  return scanPages(session, fallbackPages, params, false);
}
async function scanPages(
  session: BrowserSession,
  pages: readonly Page[],
  params: ReconcileParams,
  canProveAbsence: boolean,
): Promise<ReconcileResult> {
  const baseline = new Set(params.preClickBaseline ?? []);
  let unreadable = false;
  let ambiguous = false;
  let unboundMatch = false;
  let readable = 0;
  const matches: ReconcileResult[] = [];
  for (const page of pages) {
    const conversationUrl = page.url();
    const turns = await readTurns(page).catch(() => null);
    if (!turns) {
      unreadable = true;
      continue;
    }
    readable += 1;
    const newUsers = turns.filter((turn) => (
      turn.role === "user" && !baseline.has(turn.dataMessageId)
    ));
    const promptMatches = newUsers.filter((turn) => (
      renderedTurnMatchesPrompt(turn.text, params.prompt)
    ));
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
    });
  }
  if (matches.length === 1 && !ambiguous && !unboundMatch) return matches[0]!;
  if (matches.length > 1 || unreadable || ambiguous || unboundMatch) {
    return { found: false, proven: false };
  }
  return {
    found: false,
    proven: canProveAbsence && pages.length > 0 && readable === pages.length,
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
  };
}
function uniqueStrings(values: Array<string | undefined>): string[] {
  return [...new Set(values.filter((value): value is string => Boolean(value)))];
}
