import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

import { handlePoll } from '../lib/commands/poll.mjs';
import {
  completeProviderResponse,
  validateProviderResponse,
} from '../lib/contracts/r13.mjs';

const typed = (prefix, value) => `${prefix}_${value.repeat(64)}`;
const h256 = value => `sha256:${value.repeat(64)}`;

function echo({ activeTurn = true, terminalHash = null, sessionId = 'session_1' } = {}) {
  return {
    activeTurn,
    bindingGeneration: 2,
    bindingId: typed('binding', '1'),
    browserContextId: typed('ctx', '2'),
    cohort: 'cohort-a',
    conversationUrl: `https://chatgpt.com/c/${sessionId}`,
    domMutationGeneration: 1,
    leaseGeneration: 1,
    leaseId: typed('lease', '3'),
    pageBindingGeneration: 2,
    pageIncarnationId: typed('page', '4'),
    requestId: 'request-1',
    rootBindingHash: h256('5'),
    runId: 'run-1',
    runtimeIncarnationId: typed('runtime', '6'),
    runtimeOwnerGeneration: 1,
    runtimeOwnerId: typed('owner', '7'),
    sessionBindingId: typed('binding', '8'),
    sessionId,
    slotId: 'slot-01',
    targetId: typed('target', '9'),
    terminalAnswerSha256: terminalHash,
    visibleAssistantTurnId: typed('turn', 'a'),
    visibleUserTurnId: typed('turn', 'b'),
  };
}

function request(expected = echo()) {
  return {
    deadlineMs: 2,
    evidence: {
      cdpRelPath: 'cdp.sanitized.json',
      domRelPath: 'dom.sanitized.json',
      receiptRelPaths: {
        postClick: null,
        preClick: null,
        primary: 'provider-receipt.json',
        reconcile: null,
      },
      screenshotRelPath: 'screenshot.privacy-crop.png',
    },
    identity: {
      cohort: 'cohort-a', operationId: 'operation-1', requestId: 'request-1',
      runId: 'run-1', sessionId: 'session_1', slotId: 'slot-01',
    },
    operation: 'poll',
    operationData: {
      artifactExpectation: 'none', expected, pollAttemptId: 'poll-1', pollTimeoutSeconds: 1,
    },
    schema: 'gpt-webai.provider.request.r13.v1',
  };
}

function page(url) {
  return {
    url: () => url,
    waitForTimeout: async () => undefined,
  };
}

test('running poll is observation-only and returns no answer tuple', async () => {
  const expected = echo({ activeTurn: true });
  let observations = 0;
  let captures = 0;
  const result = await handlePoll({
    request: request(expected),
    page: page(expected.conversationUrl),
    artifactsRoot: '/unused',
    captureEvidence: async () => {
      captures += 1;
      return [];
    },
  }, {
    observeR13Session: async () => {
      observations += 1;
      return { activeTurn: true, answerText: '', observedEcho: expected };
    },
  });
  assert.equal(result.ok, true);
  assert.equal(result.status, 'running');
  assert.equal(result.operationData.pollState, 'running');
  assert.equal(result.operationData.answerRelPath, null);
  assert.ok(observations >= 1);
  assert.ok(captures >= observations);
});

test('terminal poll writes exact raw bytes before a valid provider receipt is emitted', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'r13-poll-terminal-'));
  const answerText = 'terminal answer bytes';
  const answerHash = `sha256:${createHash('sha256').update(answerText).digest('hex')}`;
  const expected = echo({ activeTurn: false, terminalHash: answerHash });
  const result = await handlePoll({
    request: request(expected),
    page: page(expected.conversationUrl),
    artifactsRoot: root,
  }, {
    observeR13Session: async () => ({
      activeTurn: false,
      answerText,
      observedEcho: expected,
    }),
  });
  assert.equal(result.status, 'done');
  assert.equal(result.operationData.answerSha256, answerHash);
  assert.equal(
    await readFile(path.join(root, result.operationData.answerRelPath), 'utf8'),
    answerText,
  );
  const response = await completeProviderResponse({
    request: request(expected),
    evidenceRoot: root,
    ok: result.ok,
    status: result.status,
    providerReason: result.providerReason,
    operationData: result.operationData,
  });
  assert.equal(validateProviderResponse(response, request(expected)), response);
});

test('root redirect is a typed poll failure with no fabricated SessionEcho', async () => {
  const expected = echo({ activeTurn: false });
  const result = await handlePoll({
    request: request(expected),
    page: page('https://chatgpt.com/'),
    artifactsRoot: '/unused',
  }, {
    observeR13Session: async () => { throw new Error('must not observe a root URL'); },
  });
  assert.equal(result.providerReason, 'session.url_rejected_root');
  assert.equal(result.operationData.observedEcho, null);
  assert.equal(result.operationData.pollState, 'failed');
});

test('a different conversation is preserved as mismatch evidence without navigation', async () => {
  const expected = echo({ activeTurn: false });
  const mismatch = echo({ activeTurn: false, sessionId: 'other_session' });
  const result = await handlePoll({
    request: request(expected),
    page: page(mismatch.conversationUrl),
    artifactsRoot: '/unused',
  }, {
    observeR13Session: async (_page, input) => {
      assert.equal(input.sessionId, 'other_session');
      return { activeTurn: false, answerText: '', observedEcho: mismatch };
    },
  });
  assert.equal(result.providerReason, 'session.url_rejected_mismatch');
  assert.equal(result.operationData.observedEcho.sessionId, 'other_session');
});
