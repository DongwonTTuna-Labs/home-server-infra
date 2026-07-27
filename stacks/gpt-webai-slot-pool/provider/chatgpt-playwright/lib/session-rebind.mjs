import { createHash } from 'node:crypto';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { conversationIdFromUrl, sha256Text } from './common.mjs';
import {
  derivePageBindingId,
  deriveSessionBindingId,
  deriveTurnId,
} from './contracts/r13.mjs';
import { captureRootState } from './root-selector.mjs';
import {
  assistantTurns,
  generationActive,
  r13TurnSnapshot,
} from './turns.mjs';

export async function handleSessionRebind(context, overrides = {}) {
  const {
    request,
    page,
    evidenceRefs,
    observeSession,
    artifactsRoot,
  } = context;
  const dependencies = { persistTerminalAnswer, ...overrides };
  const captureEvidence = context.captureEvidence ?? (async () => evidenceRefs);
  const { expectation } = request.operationData;
  let navigationError = null;
  for (let attempt = 0; attempt < request.operationData.navigationAttemptLimit; attempt += 1) {
    if (page.url() === expectation.conversationUrl) break;
    try {
      await captureEvidence();
      await page.goto(expectation.conversationUrl, {
        waitUntil: 'domcontentloaded',
        timeout: 30_000,
      });
    } catch (error) {
      navigationError = error;
    }
    if (page.url() === expectation.conversationUrl) break;
  }
  const observedUrl = page.url();
  if (observedUrl === 'https://chatgpt.com/' || observedUrl === 'https://chatgpt.com') {
    return rebindFailure(expectation, 'session.url_rejected_root', null, []);
  }
  const observedSessionId = conversationIdFromUrl(observedUrl);
  if (!observedSessionId) {
    return rebindFailure(
      expectation,
      navigationError ? 'session.rebind_failed' : 'session.missing',
      null,
      [],
    );
  }
  if (observedSessionId !== expectation.sessionId) {
    const mismatch = await observeSession(observedSessionId, observedUrl);
    return rebindFailure(
      expectation,
      'session.url_rejected_mismatch',
      mismatch.observedEcho,
      [],
    );
  }

  const hydrationObservations = [];
  const startedAt = Date.now();
  const deadline = startedAt + request.operationData.hydrationDeadlineMs;
  let latest = null;
  for (let sequenceIndex = 0; sequenceIndex < 50 && Date.now() <= deadline; sequenceIndex += 1) {
    const observationEvidenceRefs = await captureEvidence();
    latest = await observeSession(expectation.sessionId, expectation.conversationUrl);
    const observedAtMs = Date.now();
    const state = hydrationState(latest, sequenceIndex === 49 || observedAtMs >= deadline);
    hydrationObservations.push({
      sequenceIndex,
      state,
      remainingDeadlineMs: Math.max(0, deadline - observedAtMs),
      observedEcho: latest.observedEcho,
      evidenceRefs: observationEvidenceRefs,
      observedAtMs,
    });
    if (state === 'active_generation_visible' || state === 'answer_visible') break;
    await page.waitForTimeout(sequenceIndex < 10 ? 1_000 : 2_000);
  }
  const finalObservation = hydrationObservations.at(-1);
  if (!finalObservation
      || !['active_generation_visible', 'answer_visible'].includes(finalObservation.state)) {
    return rebindFailure(
      expectation,
      'session.content_unavailable',
      latest?.observedEcho ?? null,
      hydrationObservations,
    );
  }
  const terminalAnswer = finalObservation.state === 'answer_visible'
    ? await dependencies.persistTerminalAnswer({
      answerText: latest.answerText,
      artifactsRoot,
      operationId: request.identity.operationId,
      terminalAssistantTurnId: latest.observedEcho.visibleAssistantTurnId,
    })
    : null;
  return {
    ok: true,
    status: 'done',
    providerReason: null,
    operationData: {
      expectation,
      observedEcho: latest.observedEcho,
      pageBindingGeneration: expectation.lastKnownPageBindingGeneration + 1,
      hydrationObservations,
      terminalAnswer,
      failureReason: null,
    },
  };
}

export async function persistTerminalAnswer({
  answerText,
  artifactsRoot,
  operationId,
  terminalAssistantTurnId,
}) {
  const bytes = Buffer.from(answerText, 'utf8');
  const answerRelPath = `answers/${operationId}.answer.md`;
  const target = path.join(artifactsRoot, answerRelPath);
  await mkdir(path.dirname(target), { recursive: true, mode: 0o700 });
  await writeFile(target, bytes, { flag: 'wx', mode: 0o600 });
  return {
    answerRelPath,
    answerSha256: `sha256:${createHash('sha256').update(bytes).digest('hex')}`,
    answerSizeBytes: bytes.length,
    terminalAssistantTurnId,
  };
}

export async function observeR13Session(page, {
  expected,
  pageBindingGeneration,
  sessionId,
}) {
  const conversationUrl = page.url();
  const observedSessionId = conversationIdFromUrl(conversationUrl) || sessionId;
  const [root, turns, assistants, activeTurn] = await Promise.all([
    captureRootState(page),
    r13TurnSnapshot(page, observedSessionId),
    assistantTurns(page),
    generationActive(page),
  ]);
  const latestAssistant = assistants.at(-1) ?? null;
  const terminalAnswerSha256 = !activeTurn && latestAssistant?.text
    ? `sha256:${sha256Text(latestAssistant.text)}`
    : null;
  // Derive the assistant turn id from the SAME node that supplies answerText
  // (the direct `[data-message-author-role]` scan), not the article-based
  // snapshot. The two scans disagree intermittently (the completed answer is
  // found by the direct scan before it is wrapped in a `conversation-turn`
  // article), which left answerText present but the turn id null — so the
  // hydration loop never reached `answer_visible` and timed out. deriveTurnId
  // matches send-confirmation's derivation for the same dataMessageId.
  const assistantTurnId = latestAssistant?.dataMessageId
    ? deriveTurnId(observedSessionId, 'assistant', latestAssistant.dataMessageId)
    : (turns.latestAssistant?.turnId ?? null);
  const generation = pageBindingGeneration
    ?? expected.pageBindingGeneration
    ?? expected.lastKnownPageBindingGeneration + 1;
  const echo = {
    activeTurn,
    bindingGeneration: generation,
    bindingId: derivePageBindingId(root.pageIncarnationId, root.rootBindingHash),
    browserContextId: root.browserContextId,
    cohort: expected.cohort,
    conversationUrl: `https://chatgpt.com/c/${observedSessionId}`,
    domMutationGeneration: root.domMutationGeneration,
    leaseGeneration: expected.leaseGeneration,
    leaseId: expected.leaseId,
    pageBindingGeneration: generation,
    pageIncarnationId: root.pageIncarnationId,
    requestId: expected.requestId,
    rootBindingHash: root.rootBindingHash,
    runId: expected.runId,
    runtimeIncarnationId: expected.runtimeIncarnationId,
    runtimeOwnerGeneration: expected.runtimeOwnerGeneration,
    runtimeOwnerId: expected.runtimeOwnerId,
    sessionBindingId: deriveSessionBindingId(
      observedSessionId,
      expected.slotId,
      expected.cohort,
    ),
    sessionId: observedSessionId,
    slotId: expected.slotId,
    targetId: root.targetId,
    terminalAnswerSha256,
    visibleAssistantTurnId: assistantTurnId,
    visibleUserTurnId: turns.latestUser?.turnId ?? null,
  };
  return {
    activeTurn,
    answerText: latestAssistant?.text ?? '',
    contentUnavailable: !activeTurn && !latestAssistant?.text && turns.observations.length === 0,
    loading: !activeTurn && !latestAssistant?.text && turns.observations.length > 0,
    observedEcho: echo,
  };
}

function hydrationState(observation, finalObservation) {
  if (observation.activeTurn) return 'active_generation_visible';
  if (observation.answerText && observation.observedEcho.visibleAssistantTurnId) {
    return 'answer_visible';
  }
  if (observation.loading) return 'loading_placeholder';
  if (finalObservation || observation.contentUnavailable) return 'content_unavailable';
  return 'blank_transient';
}

function rebindFailure(expectation, providerReason, observedEcho, hydrationObservations) {
  return {
    ok: false,
    status: providerReason === 'session.provider_limit'
      || providerReason === 'session.login_required'
      || providerReason === 'session.subscription_required'
      ? 'blocked'
      : 'failed',
    providerReason,
    operationData: {
      expectation,
      observedEcho,
      pageBindingGeneration: null,
      hydrationObservations,
      failureReason: providerReason,
    },
  };
}
