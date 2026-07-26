import { downloadArtifacts } from '../artifacts.mjs';
import {
  artifactDownloadFailureStatus,
  artifactExpectationFromArgs,
  artifactExpectationRequiresControls,
  artifactFilenamesFromText,
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
import {
  applyProviderLimitStatus,
  hasProviderLimitDiagnostics,
} from '../provider-limit.mjs';
import {
  applyScrollBottomUnverifiedStatus,
  hasUnverifiedBottomScrollDiagnostics,
  scrollBottomUnverifiedPayload,
} from './scroll-gate.mjs';
import { captureDiagnostics } from './shared.mjs';

export async function commandDownload(args) {
  const sid = valueAfter(args, '--session') || args.find(arg => arg && !arg.startsWith('-'));
  const artifactExpectation = artifactExpectationFromArgs(args, 'required');
  if (!sid) {
    jsonOut({ ok: true, vendor: 'chatgpt', status: 'provider.schema_drift', reason: 'provider.schema_drift', message: 'missing session for download' });
    process.exit(2);
  }
  await withBrowser(async browser => {
    const page = await selectPage(browser, sid);
    const conversationUrl = page.url();
    const diagnostics = [await captureDiagnostics(page, 'download-before-click', sid)];
    if (!validConversationUrl(conversationUrl) || !conversationUrlMatchesSession(conversationUrl, sid)) {
      jsonOut({
        ok: true,
        vendor: 'chatgpt',
        status: 'session.start_unconfirmed',
        reason: validConversationUrl(conversationUrl) ? 'session.url_mismatch' : 'session.start_unconfirmed',
        sessionId: sid,
        conversationUrl,
        diagnostics,
      });
      return;
    }
    if (hasProviderLimitDiagnostics(diagnostics)) {
      jsonOut({
        ok: true,
        vendor: 'chatgpt',
        status: 'provider_limit',
        reason: 'provider.limit',
        sessionId: sid,
        conversationUrl,
        diagnostics,
      });
      return;
    }
    if (hasUnverifiedBottomScrollDiagnostics(diagnostics)) {
      jsonOut(scrollBottomUnverifiedPayload({
        sessionId: sid,
        url: conversationUrl,
        diagnostics,
        status: 'scroll.bottom_unverified',
      }));
      return;
    }
    const state = await waitForConversationHydration(page, DEFAULT_SESSION_HYDRATION_TIMEOUT_MS);
    if (!conversationHydrated(state)) {
      diagnostics.push(await captureDiagnostics(page, 'download-content-unavailable', sid));
      if (hasUnverifiedBottomScrollDiagnostics(diagnostics)) {
        jsonOut(scrollBottomUnverifiedPayload({
          sessionId: sid,
          url: conversationUrl,
          state,
          diagnostics,
          status: 'scroll.bottom_unverified',
        }));
        return;
      }
      jsonOut({
        ok: true,
        vendor: 'chatgpt',
        status: 'session.content_unavailable',
        reason: 'session.content_unavailable',
        sessionId: sid,
        conversationUrl,
        activeTurn: state.activeTurn,
        assistantCount: state.assistantCount,
        userCount: state.userCount,
        turnEvidence: state.turnEvidence,
        diagnostics,
      });
      return;
    }
    const turnIndexes = state.assistantTurn && Number.isFinite(Number(state.assistantTurn.turnIndex)) ? [state.assistantTurn.turnIndex] : [];
    const artifactExpected = artifactExpectationRequiresControls(artifactExpectation, state.answerText);
    const expectedFilenames = artifactFilenamesFromText(state.answerText);
    const { artifacts, artifactCandidates, warnings, downloadCandidateCount, bottomScroll } = await downloadArtifacts(page, sid, { turnIndexes, expectedFilenames });
    diagnostics.push(await captureDiagnostics(page, 'download-after-click', sid));
    const failureStatus = artifactDownloadFailureStatus({ artifactExpected, artifacts, warnings, downloadCandidateCount });
    const payload = downloadPayload({
      status: failureStatus || 'done',
      reason: failureStatus || undefined,
      sessionId: sid,
      conversationUrl,
      state,
      artifactExpectation,
      artifacts,
      artifactCandidates,
      warnings,
      downloadCandidateCount,
      bottomScroll,
      diagnostics,
    });
    if (!applyProviderLimitStatus(payload, diagnostics)) {
      applyScrollBottomUnverifiedStatus(payload, diagnostics, 'scroll.bottom_unverified');
    }
    jsonOut(payload);
  });
}


export function downloadPayload({ status, reason, sessionId, conversationUrl, state = {}, artifactExpectation, artifacts = [], artifactCandidates = [], warnings = [], downloadCandidateCount = 0, bottomScroll, diagnostics = [] } = {}) {
  return {
    ok: true,
    vendor: 'chatgpt',
    status,
    reason,
    sessionId,
    conversationUrl,
    assistantTurn: state.assistantTurn,
    turnEvidence: state.turnEvidence,
    artifactExpectation,
    artifacts,
    artifactCandidates,
    warnings,
    downloadCandidateCount,
    artifactDiscoveryBottomScroll: bottomScroll,
    diagnostics,
  };
}
