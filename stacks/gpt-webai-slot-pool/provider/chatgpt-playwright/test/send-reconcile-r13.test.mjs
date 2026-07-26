import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, readFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { handleSendClick } from '../lib/commands/send-click.mjs';
import { handleSendReconcile } from '../lib/commands/send-reconcile.mjs';
import { canonicalBytes } from '../lib/contracts/r13.mjs';
import { sha256Text } from '../lib/common.mjs';
import { selectR13TurnStartPair } from '../lib/send-confirmation.mjs';

const typed = (prefix, character) => `${prefix}_${character.repeat(64)}`;
const h256 = character => `sha256:${character.repeat(64)}`;
const PROMPT_SHA256 = `sha256:${sha256Text('prompt')}`;

function pageBinding() {
  return {
    bindingId: typed('binding', '1'), bindingGeneration: 1,
    browserContextId: typed('ctx', '2'), cohort: 'cohort-a', domMutationGeneration: 0,
    leaseGeneration: 1, leaseId: typed('lease', '3'),
    pageIncarnationId: typed('page', '4'), rootBindingHash: h256('5'),
    runtimeIncarnationId: typed('runtime', '6'), runtimeOwnerGeneration: 1,
    runtimeOwnerId: typed('owner', '7'), slotId: 'slot-01', targetId: typed('target', '8'),
  };
}

function evidenceRef(pathname = 'dom.sanitized.json') {
  return { path: pathname, sha256: h256('a'), sizeBytes: 1, mediaType: 'application/json' };
}

function request(operation) {
  const binding = pageBinding();
  const preClickReceipt = {
    assistantTurnId: null, capturedAtMs: 1, conversationUrl: null,
    evidenceRefs: [evidenceRef()], kind: 'pre_click', pageBinding: binding,
    physicalClickCount: 0, promptSha256: PROMPT_SHA256, sendAttemptId: 'send-1',
    sessionId: null, userTurnId: null,
  };
  return {
    deadlineMs: 30_000,
    evidence: {
      cdpRelPath: 'cdp.sanitized.json', domRelPath: 'dom.sanitized.json',
      receiptRelPaths: {
        primary: 'provider-receipt.json', preClick: 'send.pre-click.receipt.json',
        postClick: operation === 'send-click' ? 'send.post-click.receipt.json' : null,
        reconcile: operation === 'send-reconcile' ? 'send.reconcile.receipt.json' : null,
      },
      screenshotRelPath: 'screenshot.privacy-crop.png',
    },
    identity: {
      cohort: 'cohort-a', operationId: 'operation-1', requestId: 'request-1',
      runId: 'run-1', sessionId: null, slotId: 'slot-01',
    },
    operation,
    operationData: operation === 'send-click' ? {
      clickBudget: 1, pageBinding: binding,
      promptInput: { containerRelPath: 'run-1/prompt.txt', sha256: PROMPT_SHA256, sizeBytes: 6 },
      sendAttemptId: 'send-1',
      uploadProof: {
        allExpectedComplete: true, capturedAtMs: 1, expectedSetSha256: h256('e'),
        retryIndex: 0, staleChips: [], uploadAttemptId: 'upload-1', visibleCurrentChips: [],
      },
    } : { pageBinding: binding, preClickReceipt, sendAttemptId: 'send-1' },
    schema: 'gpt-webai.provider.request.r13.v1',
  };
}

function confirmation() {
  return {
    confirmed: true,
    conversationUrl: 'https://chatgpt.com/c/session_1',
    sessionId: 'session_1',
    userTurnId: typed('turn', 'a'),
    assistantTurnId: typed('turn', 'b'),
  };
}

test('send-click writes pre/post immutable receipts around exactly one physical click', async () => {
  const evidenceRoot = await mkdtemp(path.join(os.tmpdir(), 'r13-send-click-'));
  const req = request('send-click');
  let clickCount = 0;
  let evidenceCaptureCount = 0;
  const result = await handleSendClick({
    request: req, evidenceRoot, page: {}, evidenceRefs: [evidenceRef()],
    captureEvidence: async () => {
      evidenceCaptureCount += 1;
      return [evidenceRef(`post-click-${evidenceCaptureCount}.json`)];
    },
    observePageBinding: async () => pageBinding(),
  }, {
    readPromptInput: async () => Buffer.from('prompt'),
    r13TurnObservations: async () => [],
    fillPrompt: async () => undefined,
    readPromptComposer: async () => 'prompt',
    clickSend: async () => { clickCount += 1; },
    waitForR13SendStartConfirmation: async () => confirmation(),
  });
  assert.equal(clickCount, 1);
  assert.equal(evidenceCaptureCount, 1);
  assert.equal(result.ok, true);
  assert.equal(result.operationData.preClickReceipt.physicalClickCount, 0);
  assert.equal(result.operationData.terminalSendReceipt.physicalClickCount, 1);
  assert.equal(
    result.operationData.terminalSendReceipt.evidenceRefs[0].path,
    'post-click-1.json',
  );
  for (const pathname of ['send.pre-click.receipt.json', 'send.post-click.receipt.json']) {
    const bytes = await readFile(path.join(evidenceRoot, pathname));
    assert.deepEqual(bytes, canonicalBytes(JSON.parse(bytes)));
  }
});

test('send-click never clicks after the page binding changes', async () => {
  const evidenceRoot = await mkdtemp(path.join(os.tmpdir(), 'r13-send-binding-'));
  const req = request('send-click');
  const changed = pageBinding();
  changed.bindingGeneration = 2;
  let clickCount = 0;
  const result = await handleSendClick({
    request: req, evidenceRoot, page: {}, evidenceRefs: [evidenceRef()],
    observePageBinding: async () => changed,
  }, {
    readPromptInput: async () => Buffer.from('prompt'),
    r13TurnObservations: async () => [],
    fillPrompt: async () => undefined,
    readPromptComposer: async () => 'prompt',
    clickSend: async () => { clickCount += 1; },
  });
  assert.equal(clickCount, 0);
  assert.equal(result.ok, false);
  assert.equal(result.providerReason, 'send.turn_not_proven');
  assert.equal(result.operationData.observedPageBinding.bindingGeneration, 2);
});

test('fresh send confirmation requires a new assistant after the new user article', () => {
  const baselineIds = new Set(['old-user', 'old-assistant']);
  const observations = [
    { articleIndex: 0, authorRole: 'user', dataMessageId: 'old-user' },
    { articleIndex: 1, authorRole: 'assistant', dataMessageId: 'old-assistant' },
    { articleIndex: 2, authorRole: 'assistant', dataMessageId: 'unrelated-assistant' },
    { articleIndex: 3, authorRole: 'user', dataMessageId: 'new-user' },
  ];
  assert.deepEqual(selectR13TurnStartPair(observations, baselineIds), {
    user: null,
    assistant: null,
  });
  const currentAssistant = {
    articleIndex: 4,
    authorRole: 'assistant',
    dataMessageId: 'current-assistant',
  };
  observations.push(currentAssistant);
  assert.deepEqual(selectR13TurnStartPair(observations, baselineIds), {
    user: observations[3],
    assistant: currentAssistant,
  });
});

test('send-reconcile is read-only and proves the same prompt turn without a click', async () => {
  const evidenceRoot = await mkdtemp(path.join(os.tmpdir(), 'r13-send-reconcile-'));
  const req = request('send-reconcile');
  const result = await handleSendReconcile({
    request: req, evidenceRoot, page: {}, evidenceRefs: [evidenceRef()],
    observePageBinding: async () => pageBinding(),
  }, {
    reconcileR13TurnStart: async (_page, promptSha256) => {
      assert.equal(promptSha256, PROMPT_SHA256);
      return confirmation();
    },
  });
  assert.equal(result.ok, true);
  assert.equal(result.operationData.terminalSendReceipt.kind, 'reconciled_turn_start');
  assert.equal(result.operationData.terminalSendReceipt.physicalClickCount, 0);
  assert.equal(
    JSON.parse(await readFile(path.join(evidenceRoot, 'send.reconcile.receipt.json'))).payload.kind,
    'reconciled_turn_start',
  );
});

test('send-reconcile reports a changed binding without running turn reconciliation', async () => {
  const evidenceRoot = await mkdtemp(path.join(os.tmpdir(), 'r13-reconcile-binding-'));
  const req = request('send-reconcile');
  const changed = pageBinding();
  changed.bindingGeneration = 2;
  let reconcileCalls = 0;
  const result = await handleSendReconcile({
    request: req, evidenceRoot, page: {}, evidenceRefs: [evidenceRef()],
    observePageBinding: async () => changed,
  }, {
    reconcileR13TurnStart: async () => { reconcileCalls += 1; return confirmation(); },
  });
  assert.equal(reconcileCalls, 0);
  assert.equal(result.ok, false);
  assert.equal(result.providerReason, 'send.turn_not_proven');
  assert.equal(result.operationData.observedPageBinding.bindingGeneration, 2);
});

test('missing server message identities fails turn proof and never creates a terminal receipt', async () => {
  const evidenceRoot = await mkdtemp(path.join(os.tmpdir(), 'r13-send-unproven-'));
  const req = request('send-reconcile');
  const result = await handleSendReconcile({
    request: req, evidenceRoot, page: {}, evidenceRefs: [evidenceRef()],
    observePageBinding: async () => pageBinding(),
  }, {
    reconcileR13TurnStart: async () => ({ confirmed: false }),
  });
  assert.equal(result.ok, false);
  assert.equal(result.providerReason, 'send.turn_not_proven');
  assert.equal(result.operationData.terminalSendReceipt, null);
  await assert.rejects(readFile(path.join(evidenceRoot, 'send.reconcile.receipt.json')), /ENOENT/);
});
