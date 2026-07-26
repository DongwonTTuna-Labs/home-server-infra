import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

import {
  buildArtifactControl,
  handleArtifactClickSave,
  handleArtifactDiscover,
  resolveTerminalAssistantTurn,
  resolveArtifactDownloadTarget,
  selectCorrelatedDownloadStart,
  selectTerminalAssistantIndex,
} from '../lib/artifact-download-r13.mjs';
import {
  deriveArtifactId,
  deriveDownloadEventId,
  deriveTurnId,
} from '../lib/contracts/r13.mjs';

const typed = (prefix, value) => `${prefix}_${value.repeat(64)}`;
const h256 = value => `sha256:${value.repeat(64)}`;

const evidenceRefs = [{
  mediaType: 'application/json', path: 'dom.sanitized.json', sha256: h256('e'), sizeBytes: 1,
}];

function expectedEcho() {
  return {
    activeTurn: false,
    bindingGeneration: 2,
    bindingId: typed('binding', '1'),
    browserContextId: typed('ctx', '2'),
    cohort: 'cohort-a',
    conversationUrl: 'https://chatgpt.com/c/session_1',
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
    sessionId: 'session_1',
    slotId: 'slot-01',
    targetId: typed('target', '9'),
    terminalAnswerSha256: h256('a'),
    visibleAssistantTurnId: typed('turn', 'b'),
    visibleUserTurnId: typed('turn', 'c'),
  };
}

function request(operation, control = artifactControl()) {
  const expected = expectedEcho();
  return {
    identity: {
      cohort: 'cohort-a', operationId: 'operation-1', requestId: 'request-1',
      runId: 'run-1', sessionId: expected.sessionId, slotId: expected.slotId,
    },
    operation,
    operationData: operation === 'artifact-discover' ? {
      artifactClaimId: typed('artifact_claim', 'd'),
      expectation: 'optional',
      expected,
      terminalAssistantTurnId: expected.visibleAssistantTurnId,
    } : {
      artifactClaimId: typed('artifact_claim', 'd'),
      baseline: {
        baselineSha256: h256('f'), capturedAtMs: 1,
        directory: `artifacts/r-request-1/${typed('artifact_claim', 'd')}`, entries: [],
      },
      control,
      controlIndex: 0,
      expected,
      hostSaveDirectory: `artifacts/r-request-1/${typed('artifact_claim', 'd')}`,
      terminalAssistantTurnId: expected.visibleAssistantTurnId,
    },
  };
}

function artifactControl() {
  return buildArtifactControl({
    ariaLabel: 'Download report.zip',
    boundingBox: { x: 1, y: 2, width: 100, height: 30 },
    disabled: false,
    domPath: [['html', 0], ['body', 1], ['button', 2]],
    role: 'button',
    tagName: 'button',
    testId: 'download-button',
    visibleText: 'Download report.zip',
  }, typed('turn', 'b'));
}

const bottomProof = {
  atBottom: true, capturedAtMs: 10, evidenceRefs, method: 'dom_terminal_anchor',
};

test('artifact discovery represents exact zero with a non-null ZeroControlProof', async () => {
  const expected = expectedEcho();
  const result = await handleArtifactDiscover({
    request: request('artifact-discover'), page: {}, evidenceRefs,
    observeSession: async () => ({ observedEcho: expected }),
  }, {
    proveBottom: async () => bottomProof,
    discoverArtifactControls: async () => [],
  });
  assert.equal(result.ok, true);
  assert.deepEqual(result.operationData.controls, []);
  assert.equal(result.operationData.zeroControlProof.controlCount, 0);
  assert.equal(result.operationData.zeroControlProof.artifactClaimId, typed('artifact_claim', 'd'));
});

test('artifact control identity is structural and duplicate identities fail closed', async () => {
  const control = artifactControl();
  assert.match(control.controlId, /^control_[0-9a-f]{64}$/);
  assert.equal(control.currentTurnId, typed('turn', 'b'));
  const result = await handleArtifactDiscover({
    request: request('artifact-discover'), page: {}, evidenceRefs,
    observeSession: async () => ({ observedEcho: expectedEcho() }),
  }, {
    proveBottom: async () => bottomProof,
    discoverArtifactControls: async () => [control, control],
  });
  assert.equal(result.ok, false);
  assert.equal(result.providerReason, 'artifact.controls_ambiguous');
  assert.deepEqual(result.operationData.controls, []);
});

test('click-save invokes one listener-before-click attempt and returns its exact receipt', async () => {
  const control = artifactControl();
  const req = request('artifact-click-save', control);
  const downloadEventId = deriveDownloadEventId(
    req.operationData.expected.pageIncarnationId,
    'fixture-download-guid',
    'result.zip',
  );
  const artifactId = deriveArtifactId(
    req.operationData.artifactClaimId,
    control.controlId,
    downloadEventId,
  );
  const receipt = {
    artifactClaimId: req.operationData.artifactClaimId,
    artifactId,
    browserContextId: req.operationData.expected.browserContextId,
    clickedAtMs: 101,
    control,
    conversationUrl: req.operationData.expected.conversationUrl,
    downloadEventId,
    hostSavedRelPath: `${req.operationData.hostSaveDirectory}/${artifactId}.download`,
    listenerArmedAtMs: 100,
    mediaType: 'application/octet-stream',
    pageIncarnationId: req.operationData.expected.pageIncarnationId,
    receivedAtMs: 102,
    sessionId: req.operationData.expected.sessionId,
    sha256: h256('3'),
    sizeBytes: 10,
    slotId: req.operationData.expected.slotId,
    targetId: req.operationData.expected.targetId,
    terminalAssistantTurnId: req.operationData.terminalAssistantTurnId,
  };
  let attempts = 0;
  let evidenceCaptures = 0;
  const result = await handleArtifactClickSave({
    request: req,
    captureEvidence: async () => {
      evidenceCaptures += 1;
      return evidenceRefs;
    },
    observeSession: async () => ({ observedEcho: expectedEcho() }),
  }, {
    clickAndSaveArtifact: async () => {
      attempts += 1;
      return receipt;
    },
  });
  assert.equal(attempts, 1);
  assert.equal(evidenceCaptures, 1);
  assert.equal(result.ok, true);
  assert.equal(result.operationData.downloadReceipt.listenerArmedAtMs, 100);
  assert.ok(result.operationData.downloadReceipt.listenerArmedAtMs
    < result.operationData.downloadReceipt.clickedAtMs);
});

test('artifact save translates only its exact state-relative request prefix into the mounted root', async t => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'r13-artifact-root-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const claimId = typed('artifact_claim', 'd');
  const artifactId = typed('artifact', 'e');
  const target = await resolveArtifactDownloadTarget({
    artifactsRoot: root,
    hostSavedRelPath: `artifacts/r-request-1/${claimId}/${artifactId}.download`,
    requestKey: 'r-request-1',
  });
  assert.equal(target, path.join(root, claimId, `${artifactId}.download`));
  await assert.rejects(resolveArtifactDownloadTarget({
    artifactsRoot: root,
    hostSavedRelPath: `artifacts/r-request-2/${claimId}/${artifactId}.download`,
    requestKey: 'r-request-1',
  }), /artifact.path_unsafe/);
});

test('lost download event fails without a second click attempt', async () => {
  let attempts = 0;
  const result = await handleArtifactClickSave({
    request: request('artifact-click-save'),
    observeSession: async () => ({ observedEcho: expectedEcho() }),
  }, {
    clickAndSaveArtifact: async () => {
      attempts += 1;
      throw new Error('artifact.event_unrecoverable');
    },
  });
  assert.equal(attempts, 1);
  assert.equal(result.ok, false);
  assert.equal(result.providerReason, 'artifact.event_unrecoverable');
  assert.equal(result.operationData.downloadReceipt, null);
});

test('binding or terminal-turn drift fences the physical click', async () => {
  let attempts = 0;
  const mismatch = expectedEcho();
  mismatch.targetId = typed('target', 'f');
  const result = await handleArtifactClickSave({
    request: request('artifact-click-save'),
    observeSession: async () => ({ observedEcho: mismatch }),
  }, {
    clickAndSaveArtifact: async () => {
      attempts += 1;
      throw new Error('must not click');
    },
  });
  assert.equal(attempts, 0);
  assert.equal(result.ok, false);
  assert.equal(result.providerReason, 'artifact.integrity_failed');
  assert.equal(result.operationData.observedEcho.targetId, typed('target', 'f'));
});

test('terminal turn selection uses only the assistant data-message-id identity', () => {
  const sessionId = 'session_1';
  const expected = deriveTurnId(sessionId, 'assistant', 'message-b');
  assert.equal(selectTerminalAssistantIndex([
    { dataMessageId: 'message-a', visible: true },
    { dataMessageId: 'message-b', visible: true },
  ], sessionId, expected), 1);
  assert.equal(selectTerminalAssistantIndex([
    { dataMessageId: null, visible: true },
    { dataMessageId: 'message-a', visible: true },
  ], sessionId, expected), null);
});

test('terminal artifact scope fails closed without the exact conversation-turn article', async () => {
  const message = {
    getAttribute: async name => (name === 'data-message-id' ? 'message-b' : null),
    isVisible: async () => true,
    locator: () => ({ count: async () => 0 }),
  };
  const page = {
    locator: () => ({ count: async () => 1, nth: () => message }),
  };
  const terminalTurnId = deriveTurnId('session_1', 'assistant', 'message-b');
  assert.equal(
    await resolveTerminalAssistantTurn(page, 'session_1', terminalTurnId),
    null,
  );
});

test('CDP download identity rejects unrelated and ambiguous global events', () => {
  const expected = {
    frameId: 'frame-b',
    suggestedFilename: 'report.zip',
    url: 'https://chatgpt.com/backend-api/files/report',
  };
  const event = {
    ...expected,
    guid: 'download-guid-b',
  };
  assert.equal(selectCorrelatedDownloadStart([
    { ...event, frameId: 'frame-a', guid: 'unrelated' },
    event,
  ], expected), event);
  assert.throws(() => selectCorrelatedDownloadStart([
    event,
    { ...event, guid: 'download-guid-c' },
  ], expected), /artifact.event_unrecoverable/);
});
