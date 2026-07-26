import {
  conversationUrlMatchesSession,
  DEFAULT_SESSION_HYDRATION_TIMEOUT_MS,
  jsonOut,
  validConversationUrl,
  valueAfter,
} from '../common.mjs';
import {
  selectPage,
  withBrowser,
} from '../browser.mjs';
import {
  conversationHydrated,
  waitForConversationHydration,
} from '../turns.mjs';
import { applyProviderLimitStatus } from '../provider-limit.mjs';
import {
  applyScrollBottomUnverifiedStatus,
  hasUnverifiedBottomScrollDiagnostics,
} from './scroll-gate.mjs';
import {
  attachExpectedArtifactsForTerminalAnswer,
  captureDiagnostics,
  targetIdForUrl,
} from './shared.mjs';

export async function commandSession(args, action) {
  if (action === 'list') {
    jsonOut({ ok: true, vendor: 'chatgpt', status: 'done', sessions: [] });
    return;
  }
  const sid = args.find(arg => arg && !arg.startsWith('-')) || valueAfter(args, '--session');
  if (!sid) {
    jsonOut({ ok: true, vendor: 'chatgpt', status: 'provider.schema_drift', reason: 'provider.schema_drift', message: `missing session for ${action}` });
    process.exit(2);
  }
  await withBrowser(async browser => {
    const page = await selectPage(browser, sid);
    const url = page.url();
    const diagnostics = [await captureDiagnostics(page, `sessions-${action}`, sid)];
    const validSessionUrl = validConversationUrl(url) && conversationUrlMatchesSession(url, sid);
    const state = validSessionUrl
      ? await waitForConversationHydration(page, DEFAULT_SESSION_HYDRATION_TIMEOUT_MS)
      : {};
    const contentUnavailable = validSessionUrl && !conversationHydrated(state);
    const hasFinalVisibleAnswer = validSessionUrl && state.answerText && !state.activeTurn && state.reason !== 'answer.progress_prologue';
    const hasRunningAnswer = validSessionUrl && (state.activeTurn || state.reason === 'answer.progress_prologue');
    const payload = sessionPayload({ action, sid, url, validSessionUrl, contentUnavailable, hasFinalVisibleAnswer, hasRunningAnswer, state, diagnostics });
    if (validSessionUrl && terminalSessionSuccess(payload.status)) {
      diagnostics.push(await captureDiagnostics(page, `sessions-${action}-before-terminal`, sid));
      if (applyProviderLimitStatus(payload, diagnostics)) {
        jsonOut(payload);
        return;
      }
      if (hasUnverifiedBottomScrollDiagnostics(diagnostics)) {
        applyScrollBottomUnverifiedStatus(payload, diagnostics, 'session.running_unverified');
        jsonOut(payload);
        return;
      }
    }
    if (action === 'resume' && hasFinalVisibleAnswer) {
      await attachExpectedArtifactsForTerminalAnswer(page, sid, payload, state.answerText, state.assistantTurn);
      diagnostics.push(await captureDiagnostics(page, `sessions-${action}-after-artifacts`, sid));
      if (!applyProviderLimitStatus(payload, diagnostics)) {
        applyScrollBottomUnverifiedStatus(payload, diagnostics, 'session.running_unverified');
      }
    }
    jsonOut(payload);
  });
}


export function sessionPayload({ action, sid, url, validSessionUrl, contentUnavailable, hasFinalVisibleAnswer, hasRunningAnswer, state = {}, diagnostics = [] } = {}) {
  const payload = {
    ok: true,
    vendor: 'chatgpt',
    status: validSessionUrl ? (contentUnavailable ? 'session.content_unavailable' : hasFinalVisibleAnswer ? 'done' : hasRunningAnswer ? 'running' : action === 'resume' ? 'resumed' : 'show') : 'session.start_unconfirmed',
    reason: validSessionUrl ? (contentUnavailable ? 'session.content_unavailable' : state.reason) : validConversationUrl(url) ? 'session.url_mismatch' : 'session.start_unconfirmed',
    sessionId: sid,
    targetId: validSessionUrl ? targetIdForUrl(url) : undefined,
    conversationUrl: url,
    url,
    activeTurn: validSessionUrl ? state.activeTurn : undefined,
    assistantCount: validSessionUrl ? state.assistantCount : undefined,
    userCount: validSessionUrl ? state.userCount : undefined,
    turnEvidence: validSessionUrl ? state.turnEvidence : undefined,
    diagnostics,
  };
  if (validSessionUrl && state.answerText) {
    payload.answerText = state.answerText;
    payload.assistantTurn = state.assistantTurn;
  }
  return payload;
}

export function terminalSessionSuccess(status) {
  return ['done', 'resumed', 'show'].includes(status);
}
