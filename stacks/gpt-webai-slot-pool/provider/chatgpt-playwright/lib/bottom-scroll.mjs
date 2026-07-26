
import { makeBottomScrollPageFunction } from './bottom-scroll/page-function.mjs';

const DEFAULT_ATTEMPTS = 4;
const DEFAULT_DELAY_MS = 80;

function positiveInt(value, fallback) {
  const parsed = Number.parseInt(String(value || ''), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

export const scrollPrimaryConversationViewportInPage = makeBottomScrollPageFunction();

async function waitForLayoutDelay(page, delayMs) {
  if (delayMs <= 0) return;
  if (typeof page?.waitForTimeout === 'function') {
    await page.waitForTimeout(delayMs).catch(() => undefined);
    return;
  }
  await new Promise(resolve => setTimeout(resolve, delayMs));
}

export async function scrollPrimaryConversationToBottom(page, options = {}) {
  const attempts = positiveInt(
    options.attempts ?? process.env.GPT_WEBAI_BOTTOM_SCROLL_ATTEMPTS,
    DEFAULT_ATTEMPTS,
  );
  const delayMs = positiveInt(
    options.delayMs ?? process.env.GPT_WEBAI_BOTTOM_SCROLL_DELAY_MS,
    DEFAULT_DELAY_MS,
  );
  const observations = [];

  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      const observation = await page.evaluate(scrollPrimaryConversationViewportInPage);
      observations.push({ attempt, ...observation });
    } catch (error) {
      observations.push({
        attempt,
        schema: 'gpt-webai.bottom-scroll-gate.v1',
        status: 'failed',
        message: error instanceof Error ? error.message : String(error),
        visualScrollbarProof: {
          status: 'unavailable',
          reason: 'scroll_evaluation_failed',
          method: 'dom_scroll_metrics_and_right_edge_scroll_root_scrollbar',
        },
      });
    }
    if (attempt < attempts) await waitForLayoutDelay(page, delayMs);
  }

  const final = observations[observations.length - 1] || { status: 'failed' };
  return {
    schema: 'gpt-webai.bottom-scroll-gate.v1',
    label: options.label || undefined,
    status: final.status || 'unknown',
    attempts: observations.length,
    selected: final.selected,
    primary: final.primary,
    before: final.before,
    candidateCount: final.candidateCount,
    viewport: final.viewport,
    visualScrollbarProof: final.visualScrollbarProof,
    moreContentAffordances: final.moreContentAffordances || [],
    ignoredMoreContentAffordances: final.ignoredMoreContentAffordances || [],
    scrolledTargets: final.scrolledTargets || [],
    observations,
  };
}
