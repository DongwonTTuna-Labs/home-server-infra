import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, readFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { selectExistingPage } from '../lib/browser-session.mjs';
import { handleSessionRebind } from '../lib/session-rebind.mjs';

const typed = (prefix, character) => `${prefix}_${character.repeat(64)}`;
const h256 = character => `sha256:${character.repeat(64)}`;

function expectation() {
  return {
    cohort: 'cohort-a', conversationUrl: 'https://chatgpt.com/c/session_1',
    lastKnownPageBindingGeneration: 1, leaseGeneration: 1, leaseId: typed('lease', '1'),
    requestId: 'request-1', runId: 'run-1', runtimeIncarnationId: typed('runtime', '2'),
    runtimeOwnerGeneration: 1, runtimeOwnerId: typed('owner', '3'), sessionId: 'session_1',
    sessionOperationClaimId: typed('claim', '4'), slotId: 'slot-01',
  };
}

function request() {
  return {
    identity: { operationId: 'operation-1' },
    operationData: {
      expectation: expectation(), hydrationDeadlineMs: 90_000,
      navigationAttemptLimit: 2, operationKind: 'resume',
    },
  };
}

function echo({ sessionId = 'session_1', active = true, terminal = false } = {}) {
  const expected = expectation();
  return {
    activeTurn: active,
    bindingGeneration: 1,
    bindingId: typed('binding', '5'),
    browserContextId: typed('ctx', '6'),
    cohort: expected.cohort,
    conversationUrl: `https://chatgpt.com/c/${sessionId}`,
    domMutationGeneration: 0,
    leaseGeneration: expected.leaseGeneration,
    leaseId: expected.leaseId,
    pageBindingGeneration: 2,
    pageIncarnationId: typed('page', '7'),
    requestId: expected.requestId,
    rootBindingHash: h256('8'),
    runId: expected.runId,
    runtimeIncarnationId: expected.runtimeIncarnationId,
    runtimeOwnerGeneration: expected.runtimeOwnerGeneration,
    runtimeOwnerId: expected.runtimeOwnerId,
    sessionBindingId: typed('binding', '9'),
    sessionId,
    slotId: expected.slotId,
    targetId: typed('target', 'a'),
    terminalAnswerSha256: terminal ? h256('b') : null,
    visibleAssistantTurnId: typed('turn', 'c'),
    visibleUserTurnId: typed('turn', 'd'),
  };
}

const evidenceRefs = [{
  path: 'dom.sanitized.json', sha256: h256('e'), sizeBytes: 1, mediaType: 'application/json',
}];

function fakePage(initialUrl, navigationUrl = null) {
  let current = initialUrl;
  let gotoCount = 0;
  return {
    url: () => current,
    goto: async () => {
      gotoCount += 1;
      if (navigationUrl !== null) current = navigationUrl;
    },
    waitForTimeout: async () => undefined,
    gotoCount: () => gotoCount,
  };
}

test('matching pinned session hydrates active generation with one ordered observation', async () => {
  const page = fakePage('https://chatgpt.com/c/session_1');
  let captures = 0;
  const result = await handleSessionRebind({
    request: request(), page, evidenceRefs, artifactsRoot: '/unused',
    captureEvidence: async () => {
      captures += 1;
      return [{ ...evidenceRefs[0], path: `capture-${captures}.json` }];
    },
    observeSession: async () => ({ observedEcho: echo(), activeTurn: true, answerText: '' }),
  });
  assert.equal(page.gotoCount(), 0);
  assert.equal(result.ok, true);
  assert.equal(result.operationData.pageBindingGeneration, 2);
  assert.equal(result.operationData.hydrationObservations.length, 1);
  assert.equal(result.operationData.hydrationObservations[0].state, 'active_generation_visible');
  assert.equal(result.operationData.hydrationObservations[0].evidenceRefs[0].path, 'capture-1.json');
  assert.equal(captures, 1);
  assert.equal(result.operationData.terminalAnswer, null);
});

test('terminal hydration writes exact answer bytes before returning their hash tuple', async () => {
  const artifactsRoot = await mkdtemp(path.join(os.tmpdir(), 'r13-rebind-answer-'));
  const terminalEcho = echo({ active: false, terminal: true });
  const answerText = 'terminal answer';
  const crypto = await import('node:crypto');
  terminalEcho.terminalAnswerSha256 = `sha256:${crypto.createHash('sha256').update(answerText).digest('hex')}`;
  const result = await handleSessionRebind({
    request: request(), page: fakePage('https://chatgpt.com/c/session_1'), evidenceRefs, artifactsRoot,
    observeSession: async () => ({
      observedEcho: terminalEcho, activeTurn: false, answerText,
    }),
  });
  assert.equal(result.ok, true);
  assert.equal(result.operationData.hydrationObservations[0].state, 'answer_visible');
  const tuple = result.operationData.terminalAnswer;
  assert.equal(await readFile(path.join(artifactsRoot, tuple.answerRelPath), 'utf8'), answerText);
  assert.equal(tuple.answerSha256, terminalEcho.terminalAnswerSha256);
  assert.equal(tuple.terminalAssistantTurnId, terminalEcho.visibleAssistantTurnId);
});

test('root redirect fails distinctly with no fabricated echo or hydration observation', async () => {
  const page = fakePage('https://chatgpt.com/', 'https://chatgpt.com/');
  const result = await handleSessionRebind({
    request: request(), page, evidenceRefs, artifactsRoot: '/unused',
    observeSession: async () => { throw new Error('must not observe root as a session'); },
  });
  assert.equal(page.gotoCount(), 2);
  assert.equal(result.providerReason, 'session.url_rejected_root');
  assert.equal(result.operationData.observedEcho, null);
  assert.deepEqual(result.operationData.hydrationObservations, []);
});

test('different non-root conversation preserves a mismatched echo without switching slots', async () => {
  const page = fakePage(
    'https://chatgpt.com/c/other_session',
    'https://chatgpt.com/c/other_session',
  );
  const mismatch = echo({ sessionId: 'other_session' });
  const result = await handleSessionRebind({
    request: request(), page, evidenceRefs, artifactsRoot: '/unused',
    observeSession: async sessionId => {
      assert.equal(sessionId, 'other_session');
      return { observedEcho: mismatch, activeTurn: true, answerText: '' };
    },
  });
  assert.equal(result.providerReason, 'session.url_rejected_mismatch');
  assert.equal(result.operationData.observedEcho.sessionId, 'other_session');
  assert.equal(result.operationData.observedEcho.slotId, 'slot-01');
});

test('pinned page selection never accepts a conversation-id prefix collision', async () => {
  const wrong = fakePage('https://chatgpt.com/c/session_1-suffix');
  const exact = fakePage('https://chatgpt.com/c/session_1?model=pro');
  const browser = {
    contexts: () => [{ pages: () => [wrong, exact] }],
  };
  assert.equal(await selectExistingPage(browser, 'session_1'), exact);
});
