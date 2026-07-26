import process from 'node:process';

import { downloadArtifacts } from '../artifacts.mjs';
import {
  DEFAULT_STABLE_MS,
  artifactDownloadFailureStatus,
  artifactExpectationFromArgs,
  artifactExpectationRequiresControls,
  artifactFilenamesFromText,
  conversationUrlMatchesSession,
  jsonOut,
  numberArg,
  progressPrologueAnswer,
  validConversationUrl,
  valueAfter,
} from '../common.mjs';
import {
  selectPage,
  withBrowser,
} from '../browser.mjs';
import {
  assistantTurns,
  generationActive,
  latestAnswerState,
} from '../turns.mjs';
import { hasProviderLimitDiagnostics } from '../provider-limit.mjs';
import {
  observeR13Session,
  persistTerminalAnswer,
} from '../session-rebind.mjs';
import {
  hasUnverifiedBottomScrollDiagnostics,
  scrollBottomUnverifiedPayload,
} from './scroll-gate.mjs';
import {
  captureDiagnostics,
  targetIdForUrl,
} from './shared.mjs';

export async function commandPoll(args) {
  const sessionId = valueAfter(args, '--session');
  const timeoutSeconds = numberArg(args, '--timeout', 300);
  const artifactExpectation = artifactExpectationFromArgs(args, 'optional');
  const stableMs = Number.parseInt(process.env.GPT_WEBAI_RESPONSE_STABLE_MS || '', 10) || DEFAULT_STABLE_MS;
  if (!sessionId) {
    jsonOut({ ok: true, vendor: 'chatgpt', status: 'provider.schema_drift', reason: 'provider.schema_drift', message: 'missing session id' });
    process.exit(2);
  }
  await withBrowser(async browser => {
    const page = await selectPage(browser, sessionId);
    const conversationUrl = page.url();
    const diagnostics = [await captureDiagnostics(page, 'poll-start-before-wait', sessionId)];
    if (!validConversationUrl(conversationUrl) || !conversationUrlMatchesSession(conversationUrl, sessionId)) {
      jsonOut({
        ok: true,
        vendor: 'chatgpt',
        status: 'session.start_unconfirmed',
        reason: validConversationUrl(conversationUrl) ? 'session.url_mismatch' : 'session.start_unconfirmed',
        sessionId,
        conversationUrl,
        diagnostics,
      });
      return;
    }

    if (hasUnverifiedBottomScrollDiagnostics(diagnostics)) {
      jsonOut(scrollBottomUnverifiedPayload({
        sessionId,
        url: conversationUrl,
        diagnostics,
        status: 'session.running_unverified',
        includeExitCode: true,
      }));
      process.exitCode = 124;
      return;
    }

    const deadline = Date.now() + timeoutSeconds * 1000;
    let lastHash = '';
    let stableSince = 0;
    let latestTurns = [];
    while (Date.now() < deadline) {
      latestTurns = await assistantTurns(page);
      const active = await generationActive(page);
      const last = latestTurns[latestTurns.length - 1];
      const hash = last?.textSha256 || '';
      if (last?.text && !active) {
        const progressPrologue = progressPrologueAnswer(last.text);
        if (hash === lastHash) {
          if (!progressPrologue && stableSince && Date.now() - stableSince >= stableMs) break;
        } else {
          lastHash = hash;
          stableSince = Date.now();
        }
      } else {
        stableSince = 0;
      }
      await page.waitForTimeout(750);
    }

    const state = await latestAnswerState(page);
    const url = page.url();
    if (!state.answerText || state.activeTurn) {
      diagnostics.push(await captureDiagnostics(page, 'poll-running-or-empty', sessionId));
      if (hasUnverifiedBottomScrollDiagnostics(diagnostics)) {
        jsonOut(scrollBottomUnverifiedPayload({
          sessionId,
          url,
          state,
          diagnostics,
          status: 'session.running_unverified',
          includeExitCode: true,
        }));
        process.exitCode = 124;
        return;
      }
      jsonOut({
        ok: true,
        vendor: 'chatgpt',
        status: 'running',
        sessionId,
        conversationUrl: url,
        activeTurn: state.activeTurn,
        assistantCount: state.assistantCount,
        userCount: state.userCount,
        turnEvidence: state.turnEvidence,
        diagnostics,
      });
      process.exitCode = 124;
      return;
    }

    if (state.reason === 'answer.progress_prologue') {
      diagnostics.push(await captureDiagnostics(page, 'poll-progress-prologue', sessionId));
      if (hasUnverifiedBottomScrollDiagnostics(diagnostics)) {
        jsonOut(scrollBottomUnverifiedPayload({
          sessionId,
          url,
          state,
          diagnostics,
          status: 'session.running_unverified',
        }));
        return;
      }
      jsonOut({
        ok: true,
        vendor: 'chatgpt',
        status: 'running',
        reason: 'answer.progress_prologue',
        sessionId,
        targetId: targetIdForUrl(url),
        conversationUrl: url,
        activeTurn: state.activeTurn,
        assistantCount: state.assistantCount,
        userCount: state.userCount,
        answerText: state.answerText,
        assistantTurn: state.assistantTurn,
        turnEvidence: state.turnEvidence,
        responseStableMs: stableMs,
        diagnostics,
      });
      return;
    }

    diagnostics.push(await captureDiagnostics(page, 'poll-terminal-before-artifacts', sessionId));
    const providerLimitPayload = terminalProviderLimitPayload({ sessionId, url, state, diagnostics });
    if (providerLimitPayload) {
      jsonOut(providerLimitPayload);
      return;
    }
    if (hasUnverifiedBottomScrollDiagnostics(diagnostics)) {
      jsonOut(scrollBottomUnverifiedPayload({
        sessionId,
        url,
        state,
        diagnostics,
        status: 'scroll.bottom_unverified',
      }));
      return;
    }

    const artifactExpected = artifactExpectationRequiresControls(artifactExpectation, state.answerText);
    const expectedFilenames = artifactFilenamesFromText(state.answerText);
    const { artifacts, artifactCandidates, warnings, downloadCandidateCount, bottomScroll } = await downloadArtifacts(page, sessionId, { turnIndexes: [state.assistantTurn.turnIndex], expectedFilenames });
    const failureStatus = artifactDownloadFailureStatus({ artifactExpected, artifacts, warnings, downloadCandidateCount });
    jsonOut({
      ok: true,
      vendor: 'chatgpt',
      status: failureStatus || 'done',
      reason: failureStatus || undefined,
      sessionId,
      targetId: targetIdForUrl(url),
      conversationUrl: url,
      answerText: state.answerText,
      assistantTurn: state.assistantTurn,
      artifactExpectation,
      artifacts,
      artifactCandidates,
      warnings,
      downloadCandidateCount,
      artifactDiscoveryBottomScroll: bottomScroll,
      turnEvidence: state.turnEvidence,
      responseStableMs: failureStatus ? undefined : stableMs,
      diagnostics,
    });
  });
}

export function terminalProviderLimitPayload({ sessionId, url, state = {}, diagnostics = [] } = {}) {
  if (!hasProviderLimitDiagnostics(diagnostics)) return null;
  return {
    ok: true,
    vendor: 'chatgpt',
    status: 'provider_limit',
    reason: 'provider.limit',
    sessionId,
    targetId: targetIdForUrl(url),
    conversationUrl: url,
    answerText: state.answerText,
    assistantTurn: state.assistantTurn,
    turnEvidence: state.turnEvidence,
    diagnostics,
  };
}

export async function handlePoll(context, overrides = {}) {
  const { request, page, artifactsRoot, evidenceRefs } = context;
  const dependencies = {
    observeR13Session,
    persistTerminalAnswer,
    ...overrides,
  };
  const captureEvidence = context.captureEvidence ?? (async () => evidenceRefs);
  const expected = request.operationData.expected;
  const currentUrl = page.url();
  if (currentUrl === 'https://chatgpt.com/' || currentUrl === 'https://chatgpt.com') {
    return pollFailure(expected, 'session.url_rejected_root', null);
  }
  if (!conversationUrlMatchesSession(currentUrl, expected.sessionId)) {
    const actualSessionId = currentUrl.match(/^https:\/\/chatgpt\.com\/c\/([^/?#]+)/)?.[1];
    const observed = actualSessionId
      ? await dependencies.observeR13Session(page, {
        expected,
        pageBindingGeneration: expected.pageBindingGeneration,
        sessionId: actualSessionId,
      })
      : null;
    return pollFailure(
      expected,
      actualSessionId ? 'session.url_rejected_mismatch' : 'session.missing',
      observed?.observedEcho ?? null,
    );
  }

  const deadline = Date.now() + Math.min(
    request.deadlineMs,
    request.operationData.pollTimeoutSeconds * 1_000,
  );
  await captureEvidence();
  let observation = await dependencies.observeR13Session(page, {
    expected,
    pageBindingGeneration: expected.pageBindingGeneration,
    sessionId: expected.sessionId,
  });
  while (observation.activeTurn && Date.now() < deadline) {
    await captureEvidence();
    await page.waitForTimeout(Math.min(750, Math.max(1, deadline - Date.now())));
    observation = await dependencies.observeR13Session(page, {
      expected,
      pageBindingGeneration: expected.pageBindingGeneration,
      sessionId: expected.sessionId,
    });
  }
  if (observation.activeTurn || !observation.answerText) {
    return {
      ok: true,
      status: 'running',
      providerReason: null,
      operationData: {
        answerRelPath: null,
        answerSha256: null,
        answerSizeBytes: null,
        bottomProof: null,
        expected,
        observedEcho: observation.observedEcho,
        pollState: 'running',
        terminalAssistantTurnId: null,
      },
    };
  }
  const terminalAssistantTurnId = observation.observedEcho.visibleAssistantTurnId;
  if (!terminalAssistantTurnId) {
    return pollFailure(expected, 'session.content_unavailable', observation.observedEcho);
  }
  const terminal = await dependencies.persistTerminalAnswer({
    answerText: observation.answerText,
    artifactsRoot,
    operationId: request.operationData.pollAttemptId,
    terminalAssistantTurnId,
  });
  return {
    ok: true,
    status: 'done',
    providerReason: null,
    operationData: {
      answerRelPath: terminal.answerRelPath,
      answerSha256: terminal.answerSha256,
      answerSizeBytes: terminal.answerSizeBytes,
      bottomProof: null,
      expected,
      observedEcho: {
        ...observation.observedEcho,
        activeTurn: false,
        terminalAnswerSha256: terminal.answerSha256,
        visibleAssistantTurnId: terminalAssistantTurnId,
      },
      pollState: 'terminal',
      terminalAssistantTurnId,
    },
  };
}

function pollFailure(expected, providerReason, observedEcho) {
  return {
    ok: false,
    status: [
      'session.provider_limit',
      'session.login_required',
      'session.subscription_required',
    ].includes(providerReason) ? 'blocked' : 'failed',
    providerReason,
    operationData: {
      answerRelPath: null,
      answerSha256: null,
      answerSizeBytes: null,
      bottomProof: null,
      expected,
      observedEcho,
      pollState: 'failed',
      terminalAssistantTurnId: null,
    },
  };
}
