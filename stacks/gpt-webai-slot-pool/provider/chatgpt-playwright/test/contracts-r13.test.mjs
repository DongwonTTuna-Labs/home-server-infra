import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, open, readFile, readdir, stat, symlink } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { runR13Cli } from '../cli.mjs';

import {
  PROVIDER_OPERATIONS,
  REQUEST_SCHEMA,
  RESPONSE_SCHEMA,
  artifactHostSavedRelPath,
  browserGuidFromWebSocketDebuggerUrl,
  canonicalBytes,
  canonicalSha256,
  deriveArtifactId,
  deriveBrowserPageIdentity,
  deriveChipStableKey,
  deriveDownloadEventId,
  derivePageBindingId,
  deriveSessionBindingId,
  deriveTurnId,
  completeProviderResponse,
  readCanonicalRequest,
  slotCdpPort,
  validateProviderRequest,
  validateProviderResponse,
  writeEvidenceBytes,
} from '../lib/contracts/r13.mjs';

const hex = value => value.repeat(64);
const h256 = value => `sha256:${hex(value)}`;
const typed = (prefix, value) => `${prefix}_${hex(value)}`;

test('R23 identifier preimages are exact and page/session binding domains stay separate', () => {
  assert.equal(
    browserGuidFromWebSocketDebuggerUrl(
      'ws://127.0.0.1:9222/devtools/browser/123E4567-E89B-12D3-A456-426614174000',
    ),
    '123e4567-e89b-12d3-a456-426614174000',
  );
  const page = deriveBrowserPageIdentity({
    browserGuid: '123e4567-e89b-12d3-a456-426614174000',
    cdpBrowserContextId: '',
    cdpTargetId: 'CDP-target-1',
    mainFrameId: 'frame-1',
    loaderId: 'loader-1',
  });
  assert.deepEqual(page, {
    browserContextId: 'ctx_7dde0f135152d9225b2c576feab8679630f2652c732e71b490e4712a70b67c6c',
    targetId: 'target_459c74e6f9ce5cf02b5f060b7aeed137dc8f1583a87435ce73ac1f4df407ffac',
    pageIncarnationId: 'page_fe0510c2d2a43663400cece612bd16b5fb00be36dcae1a5db43f40a9dbadeef7',
  });
  assert.equal(
    derivePageBindingId(page.pageIncarnationId, h256('1')),
    'binding_3b1ac7140e1668977c734d11f2d62160438eaba91412e182ce660a8ecd1f3860',
  );
  assert.equal(
    deriveSessionBindingId('session_1', 'slot-01', 'cohort-a'),
    'binding_23be62dc78ad5e82173bd26d647daf67d3cd45b79fd6a62203ecea0a40cc1c3f',
  );
  assert.equal(
    deriveTurnId('session_1', 'user', 'msg-user-1'),
    'turn_0de2d51a188b8b02c7ab87edb91ccb3effc621b2a63d118fd5c9758b96b9d537',
  );
  assert.equal(
    deriveTurnId('session_1', 'assistant', 'msg-assistant-1'),
    'turn_47398e3ec8f8709aa3ca97554ad9797ecd8ed13f23d53692ca009f85fd61a7dd',
  );
  assert.throws(() => deriveTurnId('session_1', 'assistant', ''), /dataMessageId/);
  assert.throws(() => deriveTurnId('session_1', 'system', 'msg-1'), /authorRole/);
});

test('R23 chip, download, artifact, and host path identities use the closed preimages', () => {
  const pageId = 'page_fe0510c2d2a43663400cece612bd16b5fb00be36dcae1a5db43f40a9dbadeef7';
  const claimId = typed('artifact_claim', '2');
  const controlId = typed('control', '3');
  assert.equal(
    deriveChipStableKey(pageId, 'Report Final', 0),
    'sha256:35d9d13de5bd457d440a5ce475d9d5e698ce732b19147f8890128fb0b6c1df1c',
  );
  const downloadEventId = deriveDownloadEventId(pageId, 'download-guid-1', 'report.zip');
  assert.equal(
    downloadEventId,
    'download_1cbe92311abe1e5d7aa21b91da163e11ac401a760d27235c6a66b3b6d1f9fadc',
  );
  const artifactId = deriveArtifactId(claimId, controlId, downloadEventId);
  assert.equal(
    artifactId,
    'artifact_bbcbc069d97d9f45c9d31bf833f8be22beb621a6bcc117ae7cf53c086eb4aaec',
  );
  assert.equal(
    artifactHostSavedRelPath('r-request-1', claimId, artifactId),
    `artifacts/r-request-1/${claimId}/${artifactId}.download`,
  );
});

function evidenceRef(pathname = 'dom.sanitized.json', value = 'a', mediaType = 'application/json') {
  return { path: pathname, sha256: h256(value), sizeBytes: 1, mediaType };
}

function control(value = '1') {
  return {
    boundingBoxHash: h256(value),
    controlId: typed('control', value),
    disabled: false,
    domPathHash: h256(value),
    labelHash: h256(value),
    role: 'button',
    testIdHash: null,
    visible: true,
  };
}

function pageBinding() {
  return {
    bindingId: typed('binding', '1'),
    bindingGeneration: 1,
    browserContextId: typed('ctx', '2'),
    cohort: 'cohort-a',
    domMutationGeneration: 0,
    leaseGeneration: 1,
    leaseId: typed('lease', '3'),
    pageIncarnationId: typed('page', '4'),
    rootBindingHash: h256('5'),
    runtimeIncarnationId: typed('runtime', '6'),
    runtimeOwnerGeneration: 1,
    runtimeOwnerId: typed('owner', '7'),
    slotId: 'slot-01',
    targetId: typed('target', '8'),
  };
}

function sessionEcho({ terminal = false } = {}) {
  return {
    ...pageBinding(),
    activeTurn: !terminal,
    conversationUrl: 'https://chatgpt.com/c/session_1',
    pageBindingGeneration: 1,
    requestId: 'request-1',
    runId: 'run-1',
    sessionBindingId: typed('binding', '9'),
    sessionId: 'session_1',
    terminalAnswerSha256: terminal ? h256('a') : null,
    visibleAssistantTurnId: terminal ? typed('turn', 'b') : null,
    visibleUserTurnId: typed('turn', 'c'),
  };
}

function expectation() {
  const binding = pageBinding();
  return {
    cohort: binding.cohort,
    conversationUrl: 'https://chatgpt.com/c/session_1',
    lastKnownPageBindingGeneration: 0,
    leaseGeneration: binding.leaseGeneration,
    leaseId: binding.leaseId,
    requestId: 'request-1',
    runId: 'run-1',
    runtimeIncarnationId: binding.runtimeIncarnationId,
    runtimeOwnerGeneration: binding.runtimeOwnerGeneration,
    runtimeOwnerId: binding.runtimeOwnerId,
    sessionId: 'session_1',
    sessionOperationClaimId: typed('claim', 'd'),
    slotId: binding.slotId,
  };
}

function chip() {
  return {
    boundingBoxHash: h256('1'),
    chipStableKey: h256('2'),
    complete: true,
    digest: h256('3'),
    evidenceRefs: [evidenceRef()],
    labelHash: h256('4'),
    visibleSizeBytes: 1,
  };
}

function uploadProof({ stale = false } = {}) {
  return {
    allExpectedComplete: !stale,
    capturedAtMs: 1,
    expectedSetSha256: h256('e'),
    retryIndex: 0,
    staleChips: stale ? [chip()] : [],
    uploadAttemptId: 'upload-1',
    visibleCurrentChips: [],
  };
}

function sendReceipt(kind) {
  const terminal = kind !== 'pre_click';
  return {
    assistantTurnId: terminal ? typed('turn', 'b') : null,
    capturedAtMs: 1,
    conversationUrl: terminal ? 'https://chatgpt.com/c/session_1' : null,
    evidenceRefs: [evidenceRef()],
    kind,
    pageBinding: pageBinding(),
    physicalClickCount: kind === 'post_click' ? 1 : 0,
    promptSha256: h256('f'),
    sendAttemptId: 'send-1',
    sessionId: terminal ? 'session_1' : null,
    userTurnId: terminal ? typed('turn', 'c') : null,
  };
}

function artifactControl() {
  return {
    boundingBoxHash: h256('1'),
    controlId: typed('control', '2'),
    currentTurnId: typed('turn', 'b'),
    disabled: false,
    domPathHash: h256('3'),
    role: 'button',
    visible: true,
    visibleTextHash: h256('4'),
  };
}

function bottomProof() {
  return { atBottom: true, capturedAtMs: 1, evidenceRefs: [evidenceRef()], method: 'scrollbar' };
}

function identity(session = false) {
  return {
    cohort: 'cohort-a',
    operationId: 'operation-1',
    requestId: 'request-1',
    runId: 'run-1',
    sessionId: session ? 'session_1' : null,
    slotId: 'slot-01',
  };
}

function evidence(operation) {
  return {
    cdpRelPath: 'cdp.sanitized.json',
    domRelPath: 'dom.sanitized.json',
    receiptRelPaths: {
      primary: 'provider-receipt.json',
      preClick: ['send-click', 'send-reconcile'].includes(operation) ? 'send.pre-click.receipt.json' : null,
      postClick: operation === 'send-click' ? 'send.post-click.receipt.json' : null,
      reconcile: operation === 'send-reconcile' ? 'send.reconcile.receipt.json' : null,
    },
    screenshotRelPath: 'screenshot.privacy-crop.png',
  };
}

function requestData(operation) {
  const binding = pageBinding();
  const expected = sessionEcho({ terminal: operation.startsWith('artifact-') });
  const artifactClaimId = typed('artifact_claim', '5');
  const terminalAssistantTurnId = typed('turn', 'b');
  switch (operation) {
    case 'status': return { expectedSlotId: 'slot-01', probeAttempt: 0 };
    case 'capture.root': return { requestedModel: 'pro', requestedEffort: 'standard', rediscoveryAttempt: 0 };
    case 'ensure-model': return { pageBinding: binding, requestedModel: 'pro', requestedEffort: 'standard', pickerOpenBudget: 1, stabilizationMs: 500 };
    case 'upload-only': return { pageBinding: binding, attachmentSet: { count: 0, records: [], setSha256: h256('e') }, uploadAttemptId: 'upload-1', retryIndex: 0 };
    case 'clear-upload': return { pageBinding: binding, uploadAttemptId: 'upload-1', clearAttemptId: 'clear-1', staleChips: [chip()] };
    case 'send-click': return { pageBinding: binding, sendAttemptId: 'send-1', uploadProof: uploadProof(), promptInput: { containerRelPath: 'run-1/prompt.txt', sha256: h256('f'), sizeBytes: 1 }, clickBudget: 1 };
    case 'send-reconcile': return { pageBinding: binding, sendAttemptId: 'send-1', preClickReceipt: sendReceipt('pre_click') };
    case 'session-rebind': return { operationKind: 'poll', expectation: expectation(), navigationAttemptLimit: 2, hydrationDeadlineMs: 90_000 };
    case 'poll': return { expected, pollAttemptId: 'poll-1', pollTimeoutSeconds: 1, artifactExpectation: 'none' };
    case 'artifact-discover': return { expected, artifactClaimId, terminalAssistantTurnId, expectation: 'optional' };
    case 'artifact-click-save': return {
      expected,
      artifactClaimId,
      terminalAssistantTurnId,
      control: artifactControl(),
      baseline: {
        baselineSha256: h256('6'),
        capturedAtMs: 1,
        directory: `artifacts/r-request-1/${artifactClaimId}`,
        entries: [],
      },
      controlIndex: 0,
      hostSaveDirectory: `artifacts/r-request-1/${artifactClaimId}`,
    };
    default: throw new Error(`unsupported fixture operation ${operation}`);
  }
}

function request(operation) {
  return {
    deadlineMs: 1,
    evidence: evidence(operation),
    identity: identity(['session-rebind', 'poll', 'artifact-discover', 'artifact-click-save'].includes(operation)),
    operation,
    operationData: requestData(operation),
    schema: REQUEST_SCHEMA,
  };
}

function selectionProof(requested, proofControl = control()) {
  return {
    control: proofControl,
    evidenceRefs: [evidenceRef()],
    observed: requested,
    requested,
    selectedBy: 'already_exact',
    verified: true,
    verifiedAtMs: 1,
  };
}

function responseData(operation) {
  const req = request(operation);
  const expected = req.operationData.expected;
  switch (operation) {
    case 'status': return { healthStatus: 'ready', dockerStatus: 'running', retryAfterMs: null, modelLabel: 'pro', composerReady: true };
    case 'capture.root': return {
      rootBindingCandidate: {
        browserContextId: typed('ctx', '2'), capturedAtMs: 1, composerRootId: typed('root', '1'),
        conversationRootId: typed('root', '2'), domMutationGeneration: 0, effortControl: control('2'),
        evidenceRefs: [evidenceRef()], modelControl: control('1'), normalizedUrl: 'https://chatgpt.com/',
        operationId: 'operation-1', pageIncarnationId: typed('page', '4'), selectorMargin: 50,
        targetId: typed('target', '8'),
      },
      failureProof: null,
    };
    case 'ensure-model': return { modelProof: selectionProof('pro'), effortProof: selectionProof('standard'), failureProof: null, observedPageBinding: pageBinding() };
    case 'upload-only': return { uploadProof: uploadProof(), failureReason: null, observedPageBinding: pageBinding() };
    case 'clear-upload': return { clearAttemptId: 'clear-1', clearedChips: [{ chipStableKey: h256('2'), digest: h256('3'), cleared: true }], observedPageBinding: pageBinding() };
    case 'send-click': return { preClickReceipt: sendReceipt('pre_click'), terminalSendReceipt: sendReceipt('post_click'), observedPageBinding: pageBinding() };
    case 'send-reconcile': return { preClickReceipt: sendReceipt('pre_click'), terminalSendReceipt: sendReceipt('reconciled_turn_start'), observedPageBinding: pageBinding() };
    case 'session-rebind': {
      const observedEcho = sessionEcho();
      return {
        expectation: expectation(), observedEcho, pageBindingGeneration: 1,
        hydrationObservations: [{ sequenceIndex: 0, state: 'active_generation_visible', remainingDeadlineMs: 90_000, observedEcho, evidenceRefs: [evidenceRef()], observedAtMs: 1 }],
        terminalAnswer: null, failureReason: null,
      };
    }
    case 'poll': return { expected, observedEcho: expected, pollState: 'running', answerSha256: null, answerSizeBytes: null, answerRelPath: null, terminalAssistantTurnId: null, bottomProof: null };
    case 'artifact-discover': {
      const proof = bottomProof();
      return {
        controls: [], bottomProof: proof,
        zeroControlProof: { artifactClaimId: req.operationData.artifactClaimId, terminalAssistantTurnId: req.operationData.terminalAssistantTurnId, bottomProof: proof, controlCount: 0, evidenceRefs: [evidenceRef()], capturedAtMs: 1 },
        failureReason: null, observedEcho: expected,
      };
    }
    case 'artifact-click-save': {
      const downloadEventId = deriveDownloadEventId(
        expected.pageIncarnationId,
        'fixture-download-guid',
        'result.zip',
      );
      const artifactId = deriveArtifactId(
        req.operationData.artifactClaimId,
        req.operationData.control.controlId,
        downloadEventId,
      );
      return { downloadReceipt: {
        artifactClaimId: req.operationData.artifactClaimId,
        artifactId,
        browserContextId: expected.browserContextId,
        clickedAtMs: 2,
        control: req.operationData.control,
        conversationUrl: expected.conversationUrl,
        downloadEventId,
        hostSavedRelPath: `artifacts/r-request-1/${req.operationData.artifactClaimId}/${artifactId}.download`,
        listenerArmedAtMs: 1,
        mediaType: 'application/octet-stream',
        pageIncarnationId: expected.pageIncarnationId,
        receivedAtMs: 3,
        sessionId: expected.sessionId,
        sha256: h256('8'),
        sizeBytes: 1,
        slotId: expected.slotId,
        targetId: expected.targetId,
        terminalAssistantTurnId: req.operationData.terminalAssistantTurnId,
      },
      failureReason: null,
      observedEcho: expected,
      };
    }
    default: throw new Error(`unsupported fixture operation ${operation}`);
  }
}

function response(operation) {
  const req = request(operation);
  return {
    identity: req.identity,
    ok: true,
    operation,
    operationData: responseData(operation),
    providerReason: null,
    receipt: evidenceRef('provider-receipt.json'),
    schema: RESPONSE_SCHEMA,
    status: operation === 'poll' ? 'running' : 'done',
  };
}

test('R13 request and success response unions cover all eleven operations', () => {
  assert.deepEqual(PROVIDER_OPERATIONS, [
    'status', 'capture.root', 'ensure-model', 'upload-only', 'clear-upload', 'send-click',
    'send-reconcile', 'session-rebind', 'poll', 'artifact-discover', 'artifact-click-save',
  ]);
  for (const operation of PROVIDER_OPERATIONS) {
    assert.equal(validateProviderRequest(request(operation)).operation, operation);
    assert.equal(validateProviderResponse(response(operation), request(operation)).operation, operation);
  }
  assert.equal(slotCdpPort('slot-01'), 9223);
  assert.equal(slotCdpPort('slot-10'), 9232);
});

test('R13 CLI dispatcher executes every operation through the request-file frame', async t => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'r13-dispatcher-'));
  t.after(async () => import('node:fs/promises').then(({ rm }) => rm(root, { recursive: true, force: true })));
  const calls = [];
  const handlers = Object.fromEntries(PROVIDER_OPERATIONS.map(operation => [
    operation,
    async context => {
      calls.push({ operation, requestOperation: context.request.operation });
      return {
        ok: true,
        operationData: responseData(operation),
        providerReason: null,
        status: operation === 'poll' ? 'running' : 'done',
      };
    },
  ]));

  for (const [index, operation] of PROVIDER_OPERATIONS.entries()) {
    const evidenceRoot = await mkdtemp(path.join(root, `${String(index).padStart(2, '0')}-`));
    const req = request(operation);
    const requestFile = path.join(evidenceRoot, 'provider-request.json');
    let stdoutBytes = null;
    const result = await runR13Cli(['--request-file', requestFile], {
      handlers,
      readCanonicalRequest: async pathname => ({
        evidenceRoot,
        request: req,
        requestBytes: canonicalBytes(req),
        requestFile: pathname,
      }),
      selectExistingPage: async (_browser, sessionId) => ({ sessionId }),
      withBrowserR13: async callback => callback({ kind: 'fake-browser' }),
      writeCanonicalStdout: value => {
        stdoutBytes = canonicalBytes(value);
      },
      writeR13OperationEvidence: async () => [evidenceRef()],
    });

    assert.equal(result.operation, operation);
    assert.equal(result.receipt.path, 'provider-receipt.json');
    assert.deepEqual(JSON.parse(stdoutBytes), result);
    assert.equal(stdoutBytes.at(-1), 0x0a);
    assert.equal((await stat(path.join(evidenceRoot, 'provider-receipt.json'))).mode & 0o777, 0o600);
  }

  assert.deepEqual(calls, PROVIDER_OPERATIONS.map(operation => ({
    operation,
    requestOperation: operation,
  })));
});

test('effort proof accepts standard independently of the model enum', () => {
  assert.doesNotThrow(() => validateProviderResponse(response('ensure-model'), request('ensure-model')));
  const invalid = structuredClone(response('ensure-model'));
  invalid.operationData.effortProof.requested = 'pro';
  invalid.operationData.effortProof.observed = 'pro';
  assert.throws(() => validateProviderResponse(invalid, request('ensure-model')), /effortProof/);
});

test('session-rebind failure has the exact failure field set and root rejection echo rule', () => {
  const req = request('session-rebind');
  const failure = {
    identity: req.identity,
    ok: false,
    operation: 'session-rebind',
    operationData: {
      expectation: expectation(), observedEcho: null, pageBindingGeneration: null,
      hydrationObservations: [], failureReason: 'session.url_rejected_root',
    },
    providerReason: 'session.url_rejected_root',
    receipt: evidenceRef('provider-receipt.json'),
    schema: RESPONSE_SCHEMA,
    status: 'failed',
  };
  assert.doesNotThrow(() => validateProviderResponse(failure, req));
  failure.operationData.terminalAnswer = null;
  assert.throws(() => validateProviderResponse(failure, req), /fields/);
});

test('page-bound failure union permits page-unreachable nulls but preserves known mismatches', () => {
  const ensureRequest = request('ensure-model');
  const ensureFailure = {
    ...response('ensure-model'),
    ok: false,
    status: 'failed',
    providerReason: 'picker.model_absent',
    operationData: {
      modelProof: null,
      effortProof: null,
      failureProof: {
        reason: 'picker.model_absent', pickerOpened: true, requestedModelVisible: false,
        requestedEffortVisible: true, controlIdentityStable: true,
        evidenceRefs: [evidenceRef()], failedAtMs: 1,
      },
      observedPageBinding: null,
    },
  };
  assert.throws(() => validateProviderResponse(ensureFailure, ensureRequest), /failure.binding/);

  const clearRequest = request('clear-upload');
  const clearFailure = {
    ...response('clear-upload'),
    ok: false,
    status: 'failed',
    providerReason: 'upload.chip_removal_failed',
    operationData: {
      clearAttemptId: 'clear-1', failureReason: 'upload.chip_removal_failed',
      attemptedChipKeys: [h256('2')], clearedChips: [], observedPageBinding: null,
    },
  };
  assert.doesNotThrow(() => validateProviderResponse(clearFailure, clearRequest));

  const sendRequest = request('send-click');
  const sendFailure = {
    ...response('send-click'),
    ok: false,
    status: 'failed',
    providerReason: 'send.turn_not_proven',
    operationData: { preClickReceipt: sendReceipt('pre_click'), terminalSendReceipt: null, observedPageBinding: null },
  };
  assert.doesNotThrow(() => validateProviderResponse(sendFailure, sendRequest));

  const bindingMismatch = {
    ...response('ensure-model'),
    ok: false,
    status: 'failed',
    providerReason: 'binding.mismatch',
    operationData: {
      modelProof: null, effortProof: null, failureProof: null, observedPageBinding: null,
    },
  };
  assert.throws(() => validateProviderResponse(bindingMismatch, ensureRequest), /bindingMismatch/);
  bindingMismatch.operationData.observedPageBinding = pageBinding();
  bindingMismatch.operationData.observedPageBinding.bindingGeneration = 2;
  assert.doesNotThrow(() => validateProviderResponse(bindingMismatch, ensureRequest));

  const uploadRequest = request('upload-only');
  const uploadFailure = {
    ...response('upload-only'),
    ok: false,
    status: 'failed',
    providerReason: 'upload.incomplete',
    operationData: {
      uploadProof: null, failureReason: 'upload.incomplete', observedPageBinding: null,
    },
  };
  assert.doesNotThrow(() => validateProviderResponse(uploadFailure, uploadRequest));
});

test('status probe failure keeps its closed observation shape', () => {
  const req = request('status');
  const failure = {
    ...response('status'),
    ok: false,
    status: 'failed',
    providerReason: 'probe.timeout',
    operationData: {
      healthStatus: 'unknown', dockerStatus: 'unknown', retryAfterMs: null,
      modelLabel: 'unknown', composerReady: false,
    },
  };
  assert.doesNotThrow(() => validateProviderResponse(failure, req));
});

test('session mismatch failure preserves a non-null mismatched echo', () => {
  const req = request('session-rebind');
  const failure = {
    identity: req.identity,
    ok: false,
    operation: 'session-rebind',
    operationData: {
      expectation: expectation(), observedEcho: null, pageBindingGeneration: null,
      hydrationObservations: [], failureReason: 'session.url_rejected_mismatch',
    },
    providerReason: 'session.url_rejected_mismatch',
    receipt: evidenceRef('provider-receipt.json'),
    schema: RESPONSE_SCHEMA,
    status: 'failed',
  };
  assert.throws(() => validateProviderResponse(failure, req), /rebind.mismatch/);
  failure.operationData.observedEcho = sessionEcho();
  failure.operationData.observedEcho.sessionId = 'session_2';
  failure.operationData.observedEcho.conversationUrl = 'https://chatgpt.com/c/session_2';
  assert.doesNotThrow(() => validateProviderResponse(failure, req));

  const pollRequest = request('poll');
  const pollFailure = {
    ...response('poll'),
    ok: false,
    status: 'failed',
    providerReason: 'session.url_rejected_mismatch',
    operationData: {
      expected: pollRequest.operationData.expected,
      observedEcho: null,
      pollState: 'failed',
      answerSha256: null,
      answerSizeBytes: null,
      answerRelPath: null,
      terminalAssistantTurnId: null,
      bottomProof: null,
    },
  };
  assert.throws(() => validateProviderResponse(pollFailure, pollRequest), /poll.mismatch/);
  pollFailure.operationData.observedEcho = structuredClone(pollRequest.operationData.expected);
  pollFailure.operationData.observedEcho.sessionId = 'session_2';
  pollFailure.operationData.observedEcho.conversationUrl = 'https://chatgpt.com/c/session_2';
  assert.doesNotThrow(() => validateProviderResponse(pollFailure, pollRequest));
});

test('non-null failure echoes remain bound to the expected session', () => {
  const rebindRequest = request('session-rebind');
  const rebindFailure = {
    ...response('session-rebind'),
    ok: false,
    status: 'failed',
    providerReason: 'session.content_unavailable',
    operationData: {
      expectation: expectation(), observedEcho: sessionEcho(), pageBindingGeneration: null,
      hydrationObservations: [], failureReason: 'session.content_unavailable',
    },
  };
  rebindFailure.operationData.observedEcho.leaseGeneration = 2;
  assert.throws(() => validateProviderResponse(rebindFailure, rebindRequest), /rebind.failure.echo/);

  const pollRequest = request('poll');
  const pollFailure = {
    ...response('poll'),
    ok: false,
    status: 'failed',
    providerReason: 'session.content_unavailable',
    operationData: {
      expected: pollRequest.operationData.expected,
      observedEcho: structuredClone(pollRequest.operationData.expected),
      pollState: 'failed', answerSha256: null, answerSizeBytes: null, answerRelPath: null,
      terminalAssistantTurnId: null, bottomProof: null,
    },
  };
  pollFailure.operationData.observedEcho.targetId = typed('target', 'f');
  assert.throws(() => validateProviderResponse(pollFailure, pollRequest), /poll.failure.echo/);

  const discoverRequest = request('artifact-discover');
  const discoverFailure = {
    ...response('artifact-discover'),
    ok: false,
    status: 'failed',
    providerReason: 'artifact.bottom_unverified',
    operationData: {
      controls: [], bottomProof: null, zeroControlProof: null,
      failureReason: 'artifact.bottom_unverified', observedEcho: sessionEcho({ terminal: true }),
    },
  };
  discoverFailure.operationData.observedEcho.targetId = typed('target', 'f');
  assert.throws(() => validateProviderResponse(discoverFailure, discoverRequest), /artifact-discover.failure.echo/);
  discoverFailure.operationData.observedEcho = null;
  assert.throws(() => validateProviderResponse(discoverFailure, discoverRequest), /artifact-discover.failure.echo/);
});

test('answer-visible rebind success cannot omit terminalAnswer', () => {
  const req = request('session-rebind');
  const value = response('session-rebind');
  value.operationData.hydrationObservations[0].state = 'answer_visible';
  assert.throws(() => validateProviderResponse(value, req), /terminalAnswer.visibility/);

  const sequence = response('session-rebind');
  const first = structuredClone(sequence.operationData.hydrationObservations[0]);
  first.state = 'answer_visible';
  const second = structuredClone(first);
  second.sequenceIndex = 1;
  second.state = 'active_generation_visible';
  second.remainingDeadlineMs -= 1;
  second.observedAtMs += 1;
  sequence.operationData.hydrationObservations = [first, second];
  assert.throws(() => validateProviderResponse(sequence, req), /terminalAnswer.visibility/);
});

test('canonical request reopen rejects noncanonical bytes and unsafe links', async t => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'r13-contract-request-'));
  t.after(async () => import('node:fs/promises').then(({ rm }) => rm(root, { recursive: true, force: true })));
  const pathname = path.join(root, 'request.json');
  const handle = await open(pathname, 'wx', 0o600);
  await handle.writeFile(canonicalBytes(request('status')));
  await handle.close();
  const reopened = await readCanonicalRequest(pathname);
  assert.deepEqual(reopened.request, request('status'));

  const noncanonical = path.join(root, 'noncanonical.json');
  const second = await open(noncanonical, 'wx', 0o600);
  await second.writeFile(JSON.stringify(request('status')));
  await second.close();
  await assert.rejects(readCanonicalRequest(noncanonical), /canonical/);

  const linkPath = path.join(root, 'request-link.json');
  await symlink(pathname, linkPath);
  await assert.rejects(readCanonicalRequest(linkPath), /requestFile.type/);
});

test('immutable receipt uses blank-field ReceiptId, idempotence, and collision quarantine', async t => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'r13-contract-receipt-'));
  t.after(async () => import('node:fs/promises').then(({ rm }) => rm(root, { recursive: true, force: true })));
  const req = request('status');
  const options = {
    request: req,
    evidenceRoot: root,
    ok: true,
    status: 'done',
    providerReason: null,
    operationData: responseData('status'),
    createdAtMs: 1,
  };
  const first = await completeProviderResponse(options);
  const second = await completeProviderResponse(options);
  assert.deepEqual(second, first);
  const receiptPath = path.join(root, 'provider-receipt.json');
  const receiptBytes = await readFile(receiptPath);
  const receipt = JSON.parse(receiptBytes);
  const blank = { ...receipt, receiptId: '' };
  assert.equal(receipt.receiptId, `receipt_${canonicalSha256(blank)}`);
  assert.equal(first.receipt.sha256, `sha256:${canonicalSha256(receipt)}`);
  assert.equal((await stat(receiptPath)).mode & 0o777, 0o600);
  assert.deepEqual((await readdir(root)).filter(name => name.startsWith('.')), []);

  await assert.rejects(completeProviderResponse({ ...options, createdAtMs: 2 }), /immutableCollision/);
  assert.equal(JSON.parse(await readFile(receiptPath)).createdAtMs, 1);
  assert.deepEqual((await readdir(root)).filter(name => name.startsWith('.')), ['.provider-receipt.json.operation-1.tmp']);
});

test('evidence resolution rejects a symlinked parent before writing outside the operation root', async t => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'r13-contract-root-'));
  const outside = await mkdtemp(path.join(os.tmpdir(), 'r13-contract-outside-'));
  t.after(async () => import('node:fs/promises').then(async ({ rm }) => {
    await rm(root, { recursive: true, force: true });
    await rm(outside, { recursive: true, force: true });
  }));
  await symlink(outside, path.join(root, 'escape'));
  await assert.rejects(writeEvidenceBytes({
    evidenceRoot: root,
    relPath: 'escape/untrusted.json',
    bytes: canonicalBytes({ ok: true }),
    mediaType: 'application/json',
    operationId: 'operation-1',
  }), /evidence.parent/);
  await assert.rejects(stat(path.join(outside, 'untrusted.json')), { code: 'ENOENT' });
});
