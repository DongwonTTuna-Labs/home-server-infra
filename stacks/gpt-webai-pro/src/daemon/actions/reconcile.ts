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
    return bindFoundPage(session, await scanPages(session, openPages, params.promptSha256));
  }

  const alreadyOpen = openPages.filter((page) => page.url() === params.conversationUrl);
  if (alreadyOpen.length > 0) {
    return bindFoundPage(session, await scanPages(session, alreadyOpen, params.promptSha256));
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
    const result = bindFoundPage(session, fallback);
    return result.found ? result : { found: false, proven: false };
  }

  if (!session.isConversationUrl(opened.url())) {
    const fallback = await scanPages(
      session,
      await session.relevantPages(),
      params.promptSha256,
    );
    const result = bindFoundPage(session, fallback);
    return result.found ? result : { found: false, proven: false };
  }

  return bindFoundPage(
    session,
    await scanPages(session, [opened], params.promptSha256),
  );
}

function bindFoundPage(session: BrowserSession, outcome: ScanOutcome): ReconcileResult {
  if (outcome.result.found && outcome.page) session.bindPage(outcome.page);
  return outcome.result;
}

async function scanPages(
  session: BrowserSession,
  pages: readonly Page[],
  promptSha256: string,
): Promise<ScanOutcome> {
  let observedConversation = false;
  for (const page of pages) {
    const conversationUrl = page.url();
    if (!session.isConversationUrl(conversationUrl)) continue;
    const turns = await readTurns(page).catch(() => []);
    if (turns.length === 0) continue;
    observedConversation = true;
    const user = [...turns].reverse().find((turn) => (
      turn.role === "user" && sha256Text(turn.text) === promptSha256
    ));
    if (!user) continue;
    const assistant = turns.find((turn) => (
      turn.role === "assistant" && turn.domIndex > user.domIndex
    ));
    return {
      result: {
        found: true,
        conversationUrl,
        userTurnId: user.dataMessageId,
        ...(assistant ? { assistantTurnId: assistant.dataMessageId } : {}),
        proven: true,
      },
      page,
    };
  }
  return { result: { found: false, proven: observedConversation }, page: null };
}
