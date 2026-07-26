import { createHash } from 'node:crypto';
import { constants as fsConstants } from 'node:fs';
import {
  link,
  lstat,
  mkdir,
  open,
  readFile,
  realpath,
  unlink,
} from 'node:fs/promises';
import path from 'node:path';

export const REQUEST_SCHEMA = 'gpt-webai.provider.request.r13.v1';
export const RESPONSE_SCHEMA = 'gpt-webai.provider.response.r13.v1';
export const RECEIPT_SCHEMA = 'pr72.receipt.r13.v1';

export const PROVIDER_OPERATIONS = Object.freeze([
  'status',
  'capture.root',
  'ensure-model',
  'upload-only',
  'clear-upload',
  'send-click',
  'send-reconcile',
  'session-rebind',
  'poll',
  'artifact-discover',
  'artifact-click-save',
]);

const OPERATION_SET = new Set(PROVIDER_OPERATIONS);
const H256 = /^sha256:[0-9a-f]{64}$/;
const GENERIC_ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/;
const SESSION_ID = /^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$/;
const PREFIXED = Object.freeze({
  artifactClaimId: /^artifact_claim_[0-9a-f]{64}$/,
  artifactId: /^artifact_[0-9a-f]{64}$/,
  bindingId: /^binding_[0-9a-f]{64}$/,
  browserContextId: /^ctx_[0-9a-f]{64}$/,
  claimId: /^claim_[0-9a-f]{64}$/,
  controlId: /^control_[0-9a-f]{64}$/,
  leaseId: /^lease_[0-9a-f]{64}$/,
  ownerId: /^owner_[0-9a-f]{64}$/,
  pageIncarnationId: /^page_[0-9a-f]{64}$/,
  receiptId: /^receipt_[0-9a-f]{64}$/,
  rootId: /^root_[0-9a-f]{64}$/,
  runtimeIncarnationId: /^runtime_[0-9a-f]{64}$/,
  targetId: /^target_[0-9a-f]{64}$/,
  turnId: /^turn_[0-9a-f]{64}$/,
});
const MEDIA_TYPES = new Set([
  'application/json',
  'image/png',
  'application/octet-stream',
  'text/markdown',
]);
const MODELS = new Set(['pro', 'xhigh']);
const EFFORTS = new Set(['standard', 'high']);
const MODEL_FAILURES = new Set([
  'picker.model_absent',
  'picker.effort_absent',
  'picker.control_drift',
  'picker.selection_timeout',
  'picker.reverify_mismatch',
  'capture.ambiguous',
]);
const UPLOAD_FAILURES = new Set([
  'upload.stale_chip_mismatch',
  'upload.stale_chip_uncleared',
  'upload.incomplete',
  'upload.chip_removal_failed',
]);
const SESSION_FAILURES = new Set([
  'session.rebind_failed',
  'session.pinned_slot_unavailable',
  'session.content_unavailable',
  'session.url_rejected_root',
  'session.url_rejected_mismatch',
  'session.missing',
  'session.hydration_timeout',
  'session.request_binding_missing',
  'session.claim_conflict',
  'session.provider_limit',
  'session.login_required',
  'session.subscription_required',
  'session.schema_drift',
]);
const BLOCKED_REASONS = new Set([
  'session.provider_limit',
  'session.login_required',
  'session.subscription_required',
  'provider.limit',
  'provider.login_required',
  'provider.subscription_required',
]);

export class R13ContractError extends Error {
  constructor(field, message = 'invalid R13 provider contract') {
    super(`${message}: ${field}`);
    this.name = 'R13ContractError';
    this.field = field;
  }
}

export function canonicalBytes(value) {
  validateJsonValue(value, '$');
  return Buffer.from(`${JSON.stringify(sortJson(value))}\n`, 'utf8');
}

export function canonicalSha256(value) {
  return createHash('sha256').update(canonicalBytes(value)).digest('hex');
}

export function browserGuidFromWebSocketDebuggerUrl(webSocketDebuggerUrl) {
  nonEmpty(webSocketDebuggerUrl, 'webSocketDebuggerUrl');
  let parsed;
  try {
    parsed = new URL(webSocketDebuggerUrl);
  } catch {
    throw new R13ContractError('webSocketDebuggerUrl');
  }
  const browserGuid = parsed.pathname.split('/').filter(Boolean).at(-1)?.toLowerCase() ?? '';
  assert(
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(browserGuid),
    'webSocketDebuggerUrl.browserGuid',
  );
  return browserGuid;
}

export function deriveBrowserPageIdentity({
  browserGuid,
  cdpBrowserContextId,
  cdpTargetId,
  mainFrameId,
  loaderId,
}) {
  browserGuidValue(browserGuid, 'browserGuid');
  stringValue(cdpBrowserContextId, 'cdpBrowserContextId');
  nonEmpty(cdpTargetId, 'cdpTargetId');
  nonEmpty(mainFrameId, 'mainFrameId');
  nonEmpty(loaderId, 'loaderId');
  return {
    browserContextId: derivedId('ctx', [
      'pr72.ctx.r13.v1', browserGuid, cdpBrowserContextId,
    ]),
    targetId: derivedId('target', [
      'pr72.target.r13.v1', browserGuid, cdpTargetId,
    ]),
    pageIncarnationId: derivedId('page', [
      'pr72.page.r13.v1', browserGuid, cdpTargetId, mainFrameId, loaderId,
    ]),
  };
}

export function derivePageBindingId(pageIncarnationId, rootBindingHash) {
  prefixed(pageIncarnationId, 'pageIncarnationId', 'pageIncarnationId');
  h256(rootBindingHash, 'rootBindingHash');
  return derivedId('binding', [
    'pr72.page-binding.r13.v1', pageIncarnationId, rootBindingHash,
  ]);
}

export function deriveSessionBindingId(sessionValue, slotId, cohort) {
  sessionId(sessionValue, 'sessionId');
  slot(slotId, 'slotId');
  oneOf(cohort, ['cohort-a', 'cohort-b', 'cohort-c'], 'cohort');
  return derivedId('binding', [
    'pr72.session-binding.r13.v1', sessionValue, slotId, cohort,
  ]);
}

export function deriveTurnId(sessionValue, authorRole, dataMessageId) {
  sessionId(sessionValue, 'sessionId');
  oneOf(authorRole, ['user', 'assistant'], 'authorRole');
  nonEmpty(dataMessageId, 'dataMessageId');
  return derivedId('turn', [
    'pr72.turn.r13.v1', sessionValue, authorRole, dataMessageId,
  ]);
}

export function chipStemHash(normalizedStem) {
  stringValue(normalizedStem, 'normalizedStem');
  return createHash('sha256').update(Buffer.from(normalizedStem, 'utf8')).digest('hex');
}

export function deriveChipStableKey(pageIncarnationId, normalizedStem, dupOrdinal) {
  prefixed(pageIncarnationId, 'pageIncarnationId', 'pageIncarnationId');
  integer(dupOrdinal, 0, 63, 'dupOrdinal');
  return `sha256:${canonicalSha256([
    'pr72.chip.r13.v1', pageIncarnationId, chipStemHash(normalizedStem), dupOrdinal,
  ])}`;
}

export function deriveDownloadEventId(pageIncarnationId, cdpDownloadGuid, suggestedFilename) {
  prefixed(pageIncarnationId, 'pageIncarnationId', 'pageIncarnationId');
  nonEmpty(cdpDownloadGuid, 'cdpDownloadGuid');
  stringValue(suggestedFilename, 'suggestedFilename');
  return derivedId('download', [
    'pr72.download-event.r13.v1', pageIncarnationId, cdpDownloadGuid, suggestedFilename,
  ]);
}

export function deriveArtifactId(artifactClaimId, controlId, downloadEventId) {
  prefixed(artifactClaimId, 'artifactClaimId', 'artifactClaimId');
  prefixed(controlId, 'controlId', 'controlId');
  assert(/^download_[0-9a-f]{64}$/.test(downloadEventId), 'downloadEventId');
  return derivedId('artifact', [
    'pr72.artifact.r13.v1', artifactClaimId, controlId, downloadEventId,
  ]);
}

export function artifactHostSavedRelPath(requestKey, artifactClaimId, artifactId) {
  assert(/^r-[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/.test(requestKey)
    || /^s-[A-Za-z0-9][A-Za-z0-9_-]{0,127}$/.test(requestKey)
    || /^d-[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/.test(requestKey), 'requestKey');
  prefixed(artifactClaimId, 'artifactClaimId', 'artifactClaimId');
  prefixed(artifactId, 'artifactId', 'artifactId');
  const relPath = `artifacts/${requestKey}/${artifactClaimId}/${artifactId}.download`;
  safeRelPath(relPath, 'hostSavedRelPath');
  return relPath;
}

function derivedId(prefix, preimage) {
  return `${prefix}_${canonicalSha256(preimage)}`;
}

function browserGuidValue(value, field) {
  assert(
    typeof value === 'string'
      && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value),
    field,
  );
}

function stringValue(value, field) {
  assert(typeof value === 'string' && !value.includes('\0'), field);
}

export async function readCanonicalRequest(requestFile) {
  assert(typeof requestFile === 'string' && path.isAbsolute(requestFile), 'requestFile');
  const info = await lstat(requestFile);
  assert(info.isFile() && !info.isSymbolicLink() && info.nlink === 1 && (info.mode & 0o777) === 0o600, 'requestFile.type');
  const bytes = await readFile(requestFile);
  assert(bytes.length > 0 && bytes.length <= 1_048_576, 'requestFile.size');
  assert(!bytes.includes(0) && !(bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf), 'requestFile.encoding');
  let value;
  try {
    value = JSON.parse(bytes.toString('utf8'));
  } catch {
    throw new R13ContractError('requestFile.json');
  }
  assert(Buffer.compare(bytes, canonicalBytes(value)) === 0, 'requestFile.canonical');
  validateProviderRequest(value);
  const evidenceRoot = path.dirname(requestFile);
  const actualRoot = await realpath(evidenceRoot);
  assert(path.dirname(await realpath(requestFile)) === actualRoot, 'requestFile.root');
  return { request: value, requestFile, evidenceRoot: actualRoot, requestBytes: bytes };
}

export function validateProviderRequest(request) {
  exactObject(request, ['deadlineMs', 'evidence', 'identity', 'operation', 'operationData', 'schema'], 'request');
  assert(request.schema === REQUEST_SCHEMA, 'request.schema');
  integer(request.deadlineMs, 1, 12_000_000, 'request.deadlineMs');
  validateIdentity(request.identity, 'request.identity');
  validateEvidencePaths(request.evidence, request.operation);
  assert(OPERATION_SET.has(request.operation), 'request.operation');
  validateRequestData(request.operation, request.operationData, request.identity);
  return request;
}

export function validateProviderResponse(response, request) {
  exactObject(response, ['identity', 'ok', 'operation', 'operationData', 'providerReason', 'receipt', 'schema', 'status'], 'response');
  assert(response.schema === RESPONSE_SCHEMA, 'response.schema');
  assert(response.operation === request.operation, 'response.operation');
  assert(deepEqual(response.identity, request.identity), 'response.identity');
  assert(typeof response.ok === 'boolean', 'response.ok');
  assert(['done', 'running', 'blocked', 'failed'].includes(response.status), 'response.status');
  assert(response.ok === (response.providerReason === null), 'response.providerReason.nullability');
  if (response.ok) {
    assert(['done', 'running'].includes(response.status), 'response.status.success');
  } else {
    nonEmpty(response.providerReason, 'response.providerReason');
    assert(response.status === (BLOCKED_REASONS.has(response.providerReason) ? 'blocked' : 'failed'), 'response.status.failure');
  }
  validateEvidenceRef(response.receipt, 'response.receipt');
  assert(response.receipt.path === request.evidence.receiptRelPaths.primary, 'response.receipt.path');
  assert(response.receipt.mediaType === 'application/json', 'response.receipt.mediaType');
  validateResponseData(response.operation, response.operationData, response, request);
  return response;
}

export async function completeProviderResponse({
  request,
  evidenceRoot,
  ok,
  status = ok ? 'done' : 'failed',
  providerReason = null,
  operationData,
  createdAtMs = Date.now(),
}) {
  const provisional = {
    identity: request.identity,
    ok,
    operation: request.operation,
    operationData,
    providerReason,
    receipt: {
      path: request.evidence.receiptRelPaths.primary,
      sha256: `sha256:${'0'.repeat(64)}`,
      sizeBytes: 0,
      mediaType: 'application/json',
    },
    schema: RESPONSE_SCHEMA,
    status,
  };
  validateProviderResponse(provisional, request);
  const receipt = await writeProviderReceipt({ request, evidenceRoot, operationData, createdAtMs });
  const response = { ...provisional, receipt };
  return validateProviderResponse(response, request);
}

export async function writeProviderReceipt({ request, evidenceRoot, operationData, createdAtMs = Date.now() }) {
  timestamp(createdAtMs, 'receipt.createdAtMs');
  const receiptEnvelope = {
    createdAtMs,
    operation: request.operation,
    operationId: request.identity.operationId,
    payload: operationData,
    receiptId: '',
    requestId: request.identity.requestId,
    runId: request.identity.runId,
    schema: RECEIPT_SCHEMA,
    sessionId: request.identity.sessionId,
  };
  receiptEnvelope.receiptId = `receipt_${canonicalSha256(receiptEnvelope)}`;
  const bytes = canonicalBytes(receiptEnvelope);
  const relPath = request.evidence.receiptRelPaths.primary;
  const target = await resolveEvidencePath(evidenceRoot, relPath);
  await writeImmutable(target, bytes, request.identity.operationId);
  await validateReceiptFile(target, request, operationData);
  return {
    path: relPath,
    sha256: `sha256:${createHash('sha256').update(bytes).digest('hex')}`,
    sizeBytes: bytes.length,
    mediaType: 'application/json',
  };
}

export async function validateReceiptFile(receiptPath, request, expectedPayload) {
  const info = await lstat(receiptPath);
  assert(info.isFile() && !info.isSymbolicLink() && info.nlink === 1, 'receipt.type');
  const bytes = await readFile(receiptPath);
  let receipt;
  try {
    receipt = JSON.parse(bytes.toString('utf8'));
  } catch {
    throw new R13ContractError('receipt.json');
  }
  assert(Buffer.compare(bytes, canonicalBytes(receipt)) === 0, 'receipt.canonical');
  exactObject(receipt, ['createdAtMs', 'operation', 'operationId', 'payload', 'receiptId', 'requestId', 'runId', 'schema', 'sessionId'], 'receipt');
  assert(receipt.schema === RECEIPT_SCHEMA, 'receipt.schema');
  prefixed(receipt.receiptId, 'receiptId', 'receipt.receiptId');
  assert(receipt.operation === request.operation, 'receipt.operation');
  assert(receipt.operationId === request.identity.operationId, 'receipt.operationId');
  assert(receipt.requestId === request.identity.requestId, 'receipt.requestId');
  assert(receipt.runId === request.identity.runId, 'receipt.runId');
  assert(receipt.sessionId === request.identity.sessionId, 'receipt.sessionId');
  timestamp(receipt.createdAtMs, 'receipt.createdAtMs');
  assert(deepEqual(receipt.payload, expectedPayload), 'receipt.payload');
  const blank = { ...receipt, receiptId: '' };
  assert(receipt.receiptId === `receipt_${canonicalSha256(blank)}`, 'receipt.receiptId.hash');
  return receipt;
}

export async function writeOperationReceipt({ request, evidenceRoot, relPath, operation, payload, createdAtMs = Date.now() }) {
  assert(['send.pre_click', 'send.post_click', 'send.reconcile'].includes(operation), 'receipt.operation');
  validateSendReceipt(payload, 'receipt.payload');
  const receiptKey = operation === 'send.pre_click' ? 'preClick' : operation === 'send.post_click' ? 'postClick' : 'reconcile';
  assert(request.evidence.receiptRelPaths[receiptKey] === relPath, 'receipt.path');
  assert(request.operation === (operation === 'send.reconcile' ? 'send-reconcile' : 'send-click'), 'receipt.operation.request');
  assert(payload.sendAttemptId === request.operationData.sendAttemptId, 'receipt.payload.sendAttemptId');
  assert(deepEqual(payload.pageBinding, request.operationData.pageBinding), 'receipt.payload.pageBinding');
  if (request.operation === 'send-click') {
    assert(payload.promptSha256 === request.operationData.promptInput.sha256, 'receipt.payload.promptSha256');
  } else {
    assert(payload.promptSha256 === request.operationData.preClickReceipt.promptSha256, 'receipt.payload.promptSha256');
  }
  const expectedKind = operation === 'send.pre_click' ? 'pre_click' : operation === 'send.post_click' ? 'post_click' : 'reconciled_turn_start';
  assert(payload.kind === expectedKind, 'receipt.payload.kind');
  const envelope = {
    createdAtMs,
    operation,
    operationId: request.identity.operationId,
    payload,
    receiptId: '',
    requestId: request.identity.requestId,
    runId: request.identity.runId,
    schema: RECEIPT_SCHEMA,
    sessionId: payload.sessionId,
  };
  envelope.receiptId = `receipt_${canonicalSha256(envelope)}`;
  const bytes = canonicalBytes(envelope);
  const target = await resolveEvidencePath(evidenceRoot, relPath);
  await writeImmutable(target, bytes, request.identity.operationId);
  await validateStageReceiptFile(target, request, envelope);
  return {
    envelope,
    evidenceRef: {
      path: relPath,
      sha256: `sha256:${createHash('sha256').update(bytes).digest('hex')}`,
      sizeBytes: bytes.length,
      mediaType: 'application/json',
    },
  };
}

async function validateStageReceiptFile(receiptPath, request, expected) {
  const info = await lstat(receiptPath);
  assert(info.isFile() && !info.isSymbolicLink() && info.nlink === 1, 'receipt.type');
  const bytes = await readFile(receiptPath);
  let receipt;
  try {
    receipt = JSON.parse(bytes.toString('utf8'));
  } catch {
    throw new R13ContractError('receipt.json');
  }
  assert(Buffer.compare(bytes, canonicalBytes(receipt)) === 0, 'receipt.canonical');
  assert(deepEqual(receipt, expected), 'receipt.envelope');
  const blank = { ...receipt, receiptId: '' };
  assert(receipt.receiptId === `receipt_${canonicalSha256(blank)}`, 'receipt.receiptId.hash');
  assert(receipt.operationId === request.identity.operationId, 'receipt.operationId');
}

export async function writeEvidenceBytes({ evidenceRoot, relPath, bytes, mediaType, operationId }) {
  const value = Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes);
  assert(value.length <= 10_737_418_240, 'evidence.sizeBytes');
  assert(MEDIA_TYPES.has(mediaType), 'evidence.mediaType');
  const target = await resolveEvidencePath(evidenceRoot, relPath);
  await writeImmutable(target, value, operationId);
  return {
    path: relPath,
    sha256: `sha256:${createHash('sha256').update(value).digest('hex')}`,
    sizeBytes: value.length,
    mediaType,
  };
}

export async function writeEvidenceJson(options) {
  return writeEvidenceBytes({ ...options, bytes: canonicalBytes(options.value), mediaType: 'application/json' });
}

export async function resolveEvidencePath(evidenceRoot, relPath) {
  safeRelPath(relPath, 'evidence.relPath');
  const root = await realpath(evidenceRoot);
  const rootInfo = await lstat(root);
  assert(rootInfo.isDirectory() && !rootInfo.isSymbolicLink(), 'evidence.root');
  const components = relPath.split('/');
  let parent = root;
  for (const component of components.slice(0, -1)) {
    parent = path.join(parent, component);
    try {
      await mkdir(parent, { mode: 0o700 });
    } catch (error) {
      if (error?.code !== 'EEXIST') throw error;
    }
    const info = await lstat(parent);
    assert(info.isDirectory() && !info.isSymbolicLink(), 'evidence.parent');
    const actual = await realpath(parent);
    const relative = path.relative(root, actual);
    assert(actual === parent && relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative), 'evidence.parent');
  }
  return path.join(parent, components.at(-1));
}

export function slotCdpPort(slotId) {
  slot(slotId, 'slotId');
  return 9222 + Number(slotId.slice(-2));
}

export function writeCanonicalStdout(value) {
  process.stdout.write(canonicalBytes(value));
}

function validateEvidencePaths(evidence, operation) {
  exactObject(evidence, ['cdpRelPath', 'domRelPath', 'receiptRelPaths', 'screenshotRelPath'], 'request.evidence');
  safeRelPath(evidence.cdpRelPath, 'request.evidence.cdpRelPath');
  safeRelPath(evidence.domRelPath, 'request.evidence.domRelPath');
  safeRelPath(evidence.screenshotRelPath, 'request.evidence.screenshotRelPath');
  assert(evidence.cdpRelPath === 'cdp.sanitized.json', 'request.evidence.cdpRelPath.filename');
  assert(evidence.domRelPath === 'dom.sanitized.json', 'request.evidence.domRelPath.filename');
  assert(evidence.screenshotRelPath === 'screenshot.privacy-crop.png', 'request.evidence.screenshotRelPath.filename');
  exactObject(evidence.receiptRelPaths, ['postClick', 'preClick', 'primary', 'reconcile'], 'request.evidence.receiptRelPaths');
  safeRelPath(evidence.receiptRelPaths.primary, 'request.evidence.receiptRelPaths.primary');
  assert(evidence.receiptRelPaths.primary === 'provider-receipt.json', 'request.evidence.receiptRelPaths.primary.filename');
  const values = evidence.receiptRelPaths;
  for (const key of ['preClick', 'postClick', 'reconcile']) {
    if (values[key] !== null) safeRelPath(values[key], `request.evidence.receiptRelPaths.${key}`);
  }
  if (operation === 'send-click') {
    assert(values.preClick !== null && values.postClick !== null && values.reconcile === null, 'request.evidence.receiptRelPaths.send-click');
    assert(values.preClick === 'send.pre-click.receipt.json' && values.postClick === 'send.post-click.receipt.json', 'request.evidence.receiptRelPaths.send-click.filename');
  } else if (operation === 'send-reconcile') {
    assert(values.preClick !== null && values.postClick === null && values.reconcile !== null, 'request.evidence.receiptRelPaths.send-reconcile');
    assert(values.preClick === 'send.pre-click.receipt.json' && values.reconcile === 'send.reconcile.receipt.json', 'request.evidence.receiptRelPaths.send-reconcile.filename');
  } else {
    assert(values.preClick === null && values.postClick === null && values.reconcile === null, 'request.evidence.receiptRelPaths.operation');
  }
}

function validateIdentity(identity, field) {
  exactObject(identity, ['cohort', 'operationId', 'requestId', 'runId', 'sessionId', 'slotId'], field);
  nullable(identity.cohort, value => oneOf(value, ['cohort-a', 'cohort-b', 'cohort-c'], `${field}.cohort`));
  genericId(identity.operationId, `${field}.operationId`);
  nullable(identity.requestId, value => genericId(value, `${field}.requestId`));
  nullable(identity.runId, value => genericId(value, `${field}.runId`));
  nullable(identity.sessionId, value => sessionId(value, `${field}.sessionId`));
  slot(identity.slotId, `${field}.slotId`);
  assert(identity.runId === null || identity.requestId !== null, `${field}.runId`);
}

function validatePageIdentity(binding, identity, field) {
  assert(binding.slotId === identity.slotId, `${field}.slotId.identity`);
  if (identity.cohort !== null) assert(binding.cohort === identity.cohort, `${field}.cohort.identity`);
}

function validateSessionIdentity(session, identity, field) {
  validatePageIdentity(session, identity, field);
  if (identity.sessionId !== null) assert(session.sessionId === identity.sessionId, `${field}.sessionId.identity`);
  if (identity.requestId !== null) assert(session.requestId === identity.requestId, `${field}.requestId.identity`);
  if (identity.runId !== null) assert(session.runId === identity.runId, `${field}.runId.identity`);
}

function validateRequestData(operation, data, identity) {
  const keys = REQUEST_KEYS[operation];
  exactObject(data, keys, `request.operationData.${operation}`);
  switch (operation) {
    case 'status':
      assert(data.expectedSlotId === identity.slotId, 'request.operationData.expectedSlotId');
      integer(data.probeAttempt, 0, 1, 'request.operationData.probeAttempt');
      break;
    case 'capture.root':
      modelTuple(data.requestedModel, data.requestedEffort, 'request.operationData');
      integer(data.rediscoveryAttempt, 0, 2, 'request.operationData.rediscoveryAttempt');
      break;
    case 'ensure-model':
      validatePageBinding(data.pageBinding, 'request.operationData.pageBinding');
      validatePageIdentity(data.pageBinding, identity, 'request.operationData.pageBinding');
      modelTuple(data.requestedModel, data.requestedEffort, 'request.operationData');
      assert(data.pickerOpenBudget === 1 && data.stabilizationMs === 500, 'request.operationData.ensure-model.literals');
      break;
    case 'upload-only':
      validatePageBinding(data.pageBinding, 'request.operationData.pageBinding');
      validatePageIdentity(data.pageBinding, identity, 'request.operationData.pageBinding');
      validateProviderAttachmentSet(data.attachmentSet, 'request.operationData.attachmentSet');
      genericId(data.uploadAttemptId, 'request.operationData.uploadAttemptId');
      integer(data.retryIndex, 0, 1, 'request.operationData.retryIndex');
      break;
    case 'clear-upload':
      validatePageBinding(data.pageBinding, 'request.operationData.pageBinding');
      validatePageIdentity(data.pageBinding, identity, 'request.operationData.pageBinding');
      genericId(data.uploadAttemptId, 'request.operationData.uploadAttemptId');
      genericId(data.clearAttemptId, 'request.operationData.clearAttemptId');
      array(data.staleChips, 1, 64, 'request.operationData.staleChips').forEach((item, index) => validateChipProof(item, `request.operationData.staleChips[${index}]`));
      break;
    case 'send-click':
      validatePageBinding(data.pageBinding, 'request.operationData.pageBinding');
      validatePageIdentity(data.pageBinding, identity, 'request.operationData.pageBinding');
      genericId(data.sendAttemptId, 'request.operationData.sendAttemptId');
      validateUploadProof(data.uploadProof, 'request.operationData.uploadProof');
      validatePromptInput(data.promptInput, 'request.operationData.promptInput');
      assert(data.clickBudget === 1, 'request.operationData.clickBudget');
      break;
    case 'send-reconcile':
      validatePageBinding(data.pageBinding, 'request.operationData.pageBinding');
      validatePageIdentity(data.pageBinding, identity, 'request.operationData.pageBinding');
      genericId(data.sendAttemptId, 'request.operationData.sendAttemptId');
      validateSendReceipt(data.preClickReceipt, 'request.operationData.preClickReceipt');
      assert(data.preClickReceipt.kind === 'pre_click', 'request.operationData.preClickReceipt.kind');
      break;
    case 'session-rebind':
      oneOf(data.operationKind, ['poll', 'show', 'resume', 'download'], 'request.operationData.operationKind');
      validateSessionExpectation(data.expectation, 'request.operationData.expectation');
      validateSessionIdentity(data.expectation, identity, 'request.operationData.expectation');
      assert(data.navigationAttemptLimit === 2 && data.hydrationDeadlineMs === 90_000, 'request.operationData.rebind.literals');
      break;
    case 'poll':
      validateSessionEcho(data.expected, 'request.operationData.expected');
      validateSessionIdentity(data.expected, identity, 'request.operationData.expected');
      genericId(data.pollAttemptId, 'request.operationData.pollAttemptId');
      integer(data.pollTimeoutSeconds, 1, 10_800, 'request.operationData.pollTimeoutSeconds');
      oneOf(data.artifactExpectation, ['none', 'optional', 'required', 'claimed'], 'request.operationData.artifactExpectation');
      break;
    case 'artifact-discover':
      validateSessionEcho(data.expected, 'request.operationData.expected');
      validateSessionIdentity(data.expected, identity, 'request.operationData.expected');
      prefixed(data.artifactClaimId, 'artifactClaimId', 'request.operationData.artifactClaimId');
      prefixed(data.terminalAssistantTurnId, 'turnId', 'request.operationData.terminalAssistantTurnId');
      oneOf(data.expectation, ['none', 'optional', 'required', 'claimed'], 'request.operationData.expectation');
      break;
    case 'artifact-click-save':
      validateSessionEcho(data.expected, 'request.operationData.expected');
      validateSessionIdentity(data.expected, identity, 'request.operationData.expected');
      prefixed(data.artifactClaimId, 'artifactClaimId', 'request.operationData.artifactClaimId');
      prefixed(data.terminalAssistantTurnId, 'turnId', 'request.operationData.terminalAssistantTurnId');
      validateArtifactControl(data.control, data.terminalAssistantTurnId, 'request.operationData.control');
      validateArtifactBaseline(data.baseline, 'request.operationData.baseline');
      integer(data.controlIndex, 0, 63, 'request.operationData.controlIndex');
      safeRelPath(data.hostSaveDirectory, 'request.operationData.hostSaveDirectory');
      break;
    default:
      throw new R13ContractError('request.operation');
  }
}

function validateResponseData(operation, data, response, request) {
  exactObject(data, RESPONSE_KEYS[operation][response.ok ? 'success' : 'failure'], `response.operationData.${operation}`);
  switch (operation) {
    case 'status':
      validateStatusData(data, response);
      if (!response.ok) oneOf(response.providerReason, ['probe.timeout', 'probe.unreachable'], 'response.providerReason');
      break;
    case 'capture.root':
      validateCaptureData(data, response, request);
      break;
    case 'ensure-model':
      validateEnsureModelData(data, response, request);
      break;
    case 'upload-only':
      validateUploadData(data, response, request);
      break;
    case 'clear-upload':
      validateClearUploadData(data, response, request);
      break;
    case 'send-click':
    case 'send-reconcile':
      validateSendData(data, response, request);
      break;
    case 'session-rebind':
      validateRebindData(data, response, request);
      break;
    case 'poll':
      validatePollData(data, response, request);
      break;
    case 'artifact-discover':
      validateArtifactDiscoverData(data, response, request);
      break;
    case 'artifact-click-save':
      validateArtifactSaveData(data, response, request);
      break;
    default:
      throw new R13ContractError('response.operation');
  }
}

function validateStatusData(data, response) {
  oneOf(data.healthStatus, ['ready', 'ready_model_correction_required', 'login_required', 'subscription_required', 'provider_limit', 'unreachable', 'schema_drift', 'unknown'], 'response.operationData.healthStatus');
  oneOf(data.dockerStatus, ['running', 'exited', 'missing', 'starting', 'stopping', 'unknown'], 'response.operationData.dockerStatus');
  nullable(data.retryAfterMs, value => integer(value, 0, 12_000_000, 'response.operationData.retryAfterMs'));
  oneOf(data.modelLabel, ['pro', 'non_pro', 'unknown'], 'response.operationData.modelLabel');
  assert(typeof data.composerReady === 'boolean', 'response.operationData.composerReady');
  if (response.ok) assert(response.status === 'done', 'response.status.status');
}

function validateCaptureData(data, response, request) {
  if (response.ok) {
    validateRootBindingCandidate(data.rootBindingCandidate, 'response.operationData.rootBindingCandidate');
    assert(data.rootBindingCandidate.operationId === request.identity.operationId, 'response.operationData.rootBindingCandidate.operationId');
    assert(data.failureProof === null && response.providerReason === null, 'response.operationData.capture.success');
  } else {
    assert(data.rootBindingCandidate === null, 'response.operationData.rootBindingCandidate');
    if (data.failureProof !== null) validateFailureProof(data.failureProof, 'response.operationData.failureProof');
    oneOf(response.providerReason, ['capture.ambiguous', 'capture.timeout'], 'response.providerReason');
    if (data.failureProof !== null) assert(data.failureProof.reason === response.providerReason, 'response.operationData.failureProof.reason');
  }
}

function validateEnsureModelData(data, response, request) {
  nullable(data.observedPageBinding, value => validatePageBinding(value, 'response.operationData.observedPageBinding'));
  if (response.ok) {
    assert(data.observedPageBinding !== null && deepEqual(data.observedPageBinding, request.operationData.pageBinding), 'response.operationData.observedPageBinding');
    validateModelProof(data.modelProof, request.operationData.requestedModel, 'response.operationData.modelProof');
    validateEffortProof(data.effortProof, request.operationData.requestedEffort, 'response.operationData.effortProof');
    assert(data.failureProof === null && data.observedPageBinding !== null, 'response.operationData.ensure-model.success');
  } else if (MODEL_FAILURES.has(response.providerReason)) {
    assert(data.modelProof === null && data.effortProof === null && data.failureProof !== null, 'response.operationData.ensure-model.failure');
    assert(data.observedPageBinding !== null, 'response.operationData.ensure-model.failure.binding');
    assert(deepEqual(data.observedPageBinding, request.operationData.pageBinding), 'response.operationData.observedPageBinding');
    validateFailureProof(data.failureProof, 'response.operationData.failureProof');
    assert(data.failureProof.reason === response.providerReason, 'response.operationData.failureProof.reason');
  } else {
    oneOf(response.providerReason, ['provider.schema_drift', 'contract.invalid_provider_envelope', 'binding.mismatch'], 'response.providerReason');
    assert(data.modelProof === null && data.effortProof === null && data.failureProof === null, 'response.operationData.ensure-model.invocation');
    if (response.providerReason === 'binding.mismatch') {
      assert(data.observedPageBinding !== null && !deepEqual(data.observedPageBinding, request.operationData.pageBinding), 'response.operationData.ensure-model.bindingMismatch');
    } else if (data.observedPageBinding !== null) {
      assert(deepEqual(data.observedPageBinding, request.operationData.pageBinding), 'response.operationData.observedPageBinding');
    }
  }
}

function validateUploadData(data, response, request) {
  nullable(data.observedPageBinding, value => validatePageBinding(value, 'response.operationData.observedPageBinding'));
  if (data.uploadProof !== null) {
    validateUploadProof(data.uploadProof, 'response.operationData.uploadProof');
    assert(data.uploadProof.uploadAttemptId === request.operationData.uploadAttemptId, 'response.operationData.uploadProof.uploadAttemptId');
    assert(data.uploadProof.retryIndex === request.operationData.retryIndex, 'response.operationData.uploadProof.retryIndex');
    assert(data.uploadProof.expectedSetSha256 === request.operationData.attachmentSet.setSha256, 'response.operationData.uploadProof.expectedSetSha256');
  }
  if (response.ok) {
    assert(data.uploadProof !== null && data.failureReason === null && data.observedPageBinding !== null, 'response.operationData.upload.success');
    assert(data.uploadProof.allExpectedComplete && data.uploadProof.staleChips.length === 0, 'response.operationData.uploadProof.complete');
    assert(data.uploadProof.visibleCurrentChips.length === request.operationData.attachmentSet.count, 'response.operationData.uploadProof.visibleCurrentChips');
  } else if (response.providerReason === 'upload.stale_chip_mismatch') {
    assert(data.uploadProof !== null && data.failureReason === response.providerReason && data.observedPageBinding !== null && request.operationData.retryIndex === 0, 'response.operationData.upload.mismatch');
  } else {
    assert(data.uploadProof === null && data.failureReason === response.providerReason, 'response.operationData.upload.failure');
    assert(UPLOAD_FAILURES.has(response.providerReason) && response.providerReason !== 'upload.stale_chip_mismatch', 'response.providerReason');
  }
}

function validateClearUploadData(data, response, request) {
  genericId(data.clearAttemptId, 'response.operationData.clearAttemptId');
  assert(data.clearAttemptId === request.operationData.clearAttemptId, 'response.operationData.clearAttemptId.echo');
  nullable(data.observedPageBinding, value => validatePageBinding(value, 'response.operationData.observedPageBinding'));
  const cleared = array(data.clearedChips, response.ok ? 1 : 0, 64, 'response.operationData.clearedChips');
  cleared.forEach((item, index) => validateClearedChip(item, `response.operationData.clearedChips[${index}]`));
  if (response.ok) {
    assert(data.observedPageBinding !== null && cleared.length === request.operationData.staleChips.length, 'response.operationData.clear.success');
    const requested = request.operationData.staleChips.map(chipIdentity).sort();
    const observed = cleared.map(chipIdentity).sort();
    assert(deepEqual(observed, requested), 'response.operationData.clearedChips.set');
  } else {
    assert(data.failureReason === 'upload.chip_removal_failed' && response.providerReason === data.failureReason, 'response.operationData.clear.failure');
    array(data.attemptedChipKeys, 1, 64, 'response.operationData.attemptedChipKeys').forEach((value, index) => h256(value, `response.operationData.attemptedChipKeys[${index}]`));
  }
}

function validateSendData(data, response, request) {
  validateSendReceipt(data.preClickReceipt, 'response.operationData.preClickReceipt');
  assert(data.preClickReceipt.kind === 'pre_click', 'response.operationData.preClickReceipt.kind');
  assert(data.preClickReceipt.sendAttemptId === request.operationData.sendAttemptId, 'response.operationData.preClickReceipt.sendAttemptId');
  if (request.operation === 'send-click') {
    assert(data.preClickReceipt.promptSha256 === request.operationData.promptInput.sha256, 'response.operationData.preClickReceipt.promptSha256');
  } else {
    assert(deepEqual(data.preClickReceipt, request.operationData.preClickReceipt), 'response.operationData.preClickReceipt.echo');
  }
  nullable(data.terminalSendReceipt, value => validateSendReceipt(value, 'response.operationData.terminalSendReceipt'));
  nullable(data.observedPageBinding, value => validatePageBinding(value, 'response.operationData.observedPageBinding'));
  assert(deepEqual(data.preClickReceipt.pageBinding, request.operationData.pageBinding), 'response.operationData.preClickReceipt.binding');
  if (data.terminalSendReceipt !== null) {
    assert(data.terminalSendReceipt.sendAttemptId === data.preClickReceipt.sendAttemptId, 'response.operationData.terminalSendReceipt.sendAttemptId');
    assert(data.terminalSendReceipt.promptSha256 === data.preClickReceipt.promptSha256, 'response.operationData.terminalSendReceipt.promptSha256');
    assert(deepEqual(data.terminalSendReceipt.pageBinding, data.preClickReceipt.pageBinding), 'response.operationData.terminalSendReceipt.binding');
  }
  if (response.ok) {
    assert(data.terminalSendReceipt !== null && data.observedPageBinding !== null, 'response.operationData.send.success');
    const expectedKind = request.operation === 'send-click' ? 'post_click' : 'reconciled_turn_start';
    assert(data.terminalSendReceipt.kind === expectedKind, 'response.operationData.terminalSendReceipt.kind');
  } else {
    oneOf(response.providerReason, ['send.turn_not_proven', 'send.click_timeout'], 'response.providerReason');
  }
}

function validateRebindData(data, response, request) {
  validateSessionExpectation(data.expectation, 'response.operationData.expectation');
  assert(deepEqual(data.expectation, request.operationData.expectation), 'response.operationData.expectation.echo');
  nullable(data.observedEcho, value => validateSessionEcho(value, 'response.operationData.observedEcho'));
  const observations = array(data.hydrationObservations, response.ok ? 1 : 0, 50, 'response.operationData.hydrationObservations');
  observations.forEach((item, index) => validateHydrationObservation(item, index, 'response.operationData.hydrationObservations'));
  validateHydrationOrder(observations, 'response.operationData.hydrationObservations');
  if (response.ok) {
    assert(data.observedEcho !== null && data.failureReason === null, 'response.operationData.rebind.success');
    assert(sessionMatchesExpectation(data.observedEcho, data.expectation), 'response.operationData.observedEcho.expectation');
    integer(data.pageBindingGeneration, 1, 65_535, 'response.operationData.pageBindingGeneration');
    assert(data.pageBindingGeneration === request.operationData.expectation.lastKnownPageBindingGeneration + 1, 'response.operationData.pageBindingGeneration.increment');
    assert(data.observedEcho.pageBindingGeneration === data.pageBindingGeneration, 'response.operationData.observedEcho.pageBindingGeneration');
    const finalState = observations.at(-1).state;
    assert(['active_generation_visible', 'answer_visible'].includes(finalState), 'response.operationData.hydrationObservations.finalState');
    const answerVisible = observations.some(observation => observation.state === 'answer_visible');
    assert(answerVisible === (data.terminalAnswer !== null), 'response.operationData.terminalAnswer.visibility');
    if (data.terminalAnswer !== null) {
      validateTerminalAnswer(data.terminalAnswer, 'response.operationData.terminalAnswer');
      assert(data.observedEcho.terminalAnswerSha256 === data.terminalAnswer.answerSha256, 'response.operationData.terminalAnswer.answerSha256');
      assert(data.observedEcho.visibleAssistantTurnId === data.terminalAnswer.terminalAssistantTurnId, 'response.operationData.terminalAnswer.terminalAssistantTurnId');
    } else {
      assert(data.observedEcho.terminalAnswerSha256 === null, 'response.operationData.terminalAnswer.nullability');
    }
  } else {
    assert(data.pageBindingGeneration === null && data.failureReason === response.providerReason && SESSION_FAILURES.has(response.providerReason), 'response.operationData.rebind.failure');
    if (response.providerReason === 'session.url_rejected_root') {
      assert(data.observedEcho === null && observations.length === 0, 'response.operationData.rebind.root');
    } else if (response.providerReason === 'session.url_rejected_mismatch') {
      assert(data.observedEcho !== null && !sessionMatchesExpectation(data.observedEcho, data.expectation), 'response.operationData.rebind.mismatch');
    } else if (data.observedEcho !== null) {
      assert(sessionMatchesExpectation(data.observedEcho, data.expectation), 'response.operationData.rebind.failure.echo');
    }
  }
}

function validatePollData(data, response, request) {
  validateSessionEcho(data.expected, 'response.operationData.expected');
  assert(deepEqual(data.expected, request.operationData.expected), 'response.operationData.expected.echo');
  nullable(data.observedEcho, value => validateSessionEcho(value, 'response.operationData.observedEcho'));
  if (response.ok) {
    assert(data.observedEcho !== null, 'response.operationData.observedEcho');
    assert(sessionEchoMatchesExpected(data.observedEcho, data.expected), 'response.operationData.observedEcho.expected');
    oneOf(data.pollState, ['running', 'terminal'], 'response.operationData.pollState');
    const terminal = data.pollState === 'terminal';
    assert(response.status === (terminal ? 'done' : 'running'), 'response.status.poll');
    const fields = ['answerSha256', 'answerSizeBytes', 'answerRelPath', 'terminalAssistantTurnId'];
    assert(fields.every(key => terminal ? data[key] !== null : data[key] === null), 'response.operationData.poll.answerNullability');
    if (terminal) {
      h256(data.answerSha256, 'response.operationData.answerSha256');
      integer(data.answerSizeBytes, 0, 10_737_418_240, 'response.operationData.answerSizeBytes');
      safeRelPath(data.answerRelPath, 'response.operationData.answerRelPath');
      prefixed(data.terminalAssistantTurnId, 'turnId', 'response.operationData.terminalAssistantTurnId');
      assert(data.observedEcho.terminalAnswerSha256 === data.answerSha256, 'response.operationData.answerSha256.echo');
      assert(data.observedEcho.visibleAssistantTurnId === data.terminalAssistantTurnId, 'response.operationData.terminalAssistantTurnId.echo');
      nullable(data.bottomProof, value => validateBottomProof(value, 'response.operationData.bottomProof'));
    } else {
      assert(data.bottomProof === null, 'response.operationData.bottomProof');
    }
  } else {
    assert(data.pollState === 'failed' && data.answerSha256 === null && data.answerSizeBytes === null && data.answerRelPath === null && data.terminalAssistantTurnId === null && data.bottomProof === null, 'response.operationData.poll.failure');
    assert(SESSION_FAILURES.has(response.providerReason), 'response.providerReason');
    if (response.providerReason === 'session.url_rejected_root') {
      assert(data.observedEcho === null, 'response.operationData.poll.root');
    } else if (response.providerReason === 'session.url_rejected_mismatch') {
      assert(data.observedEcho !== null && !sessionEchoMatchesExpected(data.observedEcho, data.expected), 'response.operationData.poll.mismatch');
    } else if (data.observedEcho !== null) {
      assert(sessionEchoMatchesExpected(data.observedEcho, data.expected), 'response.operationData.poll.failure.echo');
    }
  }
}

function validateArtifactDiscoverData(data, response, request) {
  nullable(data.observedEcho, value => validateSessionEcho(value, 'response.operationData.observedEcho'));
  const controls = array(data.controls, 0, 64, 'response.operationData.controls');
  controls.forEach((item, index) => validateArtifactControl(item, request.operationData.terminalAssistantTurnId, `response.operationData.controls[${index}]`));
  assert(new Set(controls.map(item => item.controlId)).size === controls.length, 'response.operationData.controls.duplicates');
  if (response.ok) {
    assert(data.observedEcho !== null && data.failureReason === null && data.bottomProof !== null, 'response.operationData.artifact-discover.success');
    assert(sessionEchoMatchesExpected(data.observedEcho, request.operationData.expected), 'response.operationData.observedEcho.expected');
    validateBottomProof(data.bottomProof, 'response.operationData.bottomProof');
    if (controls.length === 0) {
      validateZeroControlProof(data.zeroControlProof, request.operationData.artifactClaimId, request.operationData.terminalAssistantTurnId, 'response.operationData.zeroControlProof');
      assert(deepEqual(data.zeroControlProof.bottomProof, data.bottomProof), 'response.operationData.zeroControlProof.bottomProof');
    } else {
      assert(data.zeroControlProof === null, 'response.operationData.zeroControlProof');
    }
  } else {
    assert(controls.length === 0 && data.bottomProof === null && data.zeroControlProof === null && data.failureReason === response.providerReason, 'response.operationData.artifact-discover.failure');
    oneOf(response.providerReason, ['artifact.controls_ambiguous', 'artifact.bottom_unverified'], 'response.providerReason');
    assert(data.observedEcho !== null && sessionEchoMatchesExpected(data.observedEcho, request.operationData.expected), 'response.operationData.artifact-discover.failure.echo');
  }
}

function validateArtifactSaveData(data, response, request) {
  nullable(data.observedEcho, value => validateSessionEcho(value, 'response.operationData.observedEcho'));
  if (response.ok) {
    validateDownloadReceipt(data.downloadReceipt, 'response.operationData.downloadReceipt');
    assert(data.failureReason === null && data.observedEcho !== null, 'response.operationData.artifact-save.success');
    assert(sessionEchoMatchesExpected(data.observedEcho, request.operationData.expected), 'response.operationData.observedEcho.expected');
    validateDownloadReceiptBinding(data.downloadReceipt, request, 'response.operationData.downloadReceipt');
  } else {
    assert(data.downloadReceipt === null && data.failureReason === response.providerReason, 'response.operationData.artifact-save.failure');
    oneOf(response.providerReason, ['artifact.download_timeout', 'artifact.event_unrecoverable', 'artifact.integrity_failed', 'artifact.path_unsafe'], 'response.providerReason');
    assert(data.observedEcho !== null, 'response.operationData.artifact-save.failure.echo');
  }
}

export function validatePageBinding(value, field = 'pageBinding') {
  const keys = ['bindingId', 'bindingGeneration', 'browserContextId', 'cohort', 'domMutationGeneration', 'leaseGeneration', 'leaseId', 'pageIncarnationId', 'rootBindingHash', 'runtimeIncarnationId', 'runtimeOwnerGeneration', 'runtimeOwnerId', 'slotId', 'targetId'];
  exactObject(value, keys, field);
  prefixed(value.bindingId, 'bindingId', `${field}.bindingId`);
  integer(value.bindingGeneration, 1, 65_535, `${field}.bindingGeneration`);
  prefixed(value.browserContextId, 'browserContextId', `${field}.browserContextId`);
  oneOf(value.cohort, ['cohort-a', 'cohort-b', 'cohort-c'], `${field}.cohort`);
  integer(value.domMutationGeneration, 0, 65_535, `${field}.domMutationGeneration`);
  integer(value.leaseGeneration, 1, 65_535, `${field}.leaseGeneration`);
  prefixed(value.leaseId, 'leaseId', `${field}.leaseId`);
  prefixed(value.pageIncarnationId, 'pageIncarnationId', `${field}.pageIncarnationId`);
  h256(value.rootBindingHash, `${field}.rootBindingHash`);
  prefixed(value.runtimeIncarnationId, 'runtimeIncarnationId', `${field}.runtimeIncarnationId`);
  integer(value.runtimeOwnerGeneration, 1, 65_535, `${field}.runtimeOwnerGeneration`);
  prefixed(value.runtimeOwnerId, 'ownerId', `${field}.runtimeOwnerId`);
  slot(value.slotId, `${field}.slotId`);
  prefixed(value.targetId, 'targetId', `${field}.targetId`);
  return value;
}

export function validateSessionEcho(value, field = 'sessionEcho') {
  const extra = ['activeTurn', 'conversationUrl', 'pageBindingGeneration', 'requestId', 'runId', 'sessionBindingId', 'sessionId', 'terminalAnswerSha256', 'visibleAssistantTurnId', 'visibleUserTurnId'];
  exactObject(value, [...PAGE_BINDING_KEYS, ...extra], field);
  validatePageBinding(pick(value, PAGE_BINDING_KEYS), `${field}.pageBinding`);
  sessionId(value.sessionId, `${field}.sessionId`);
  conversationUrl(value.conversationUrl, value.sessionId, `${field}.conversationUrl`);
  nullable(value.requestId, item => genericId(item, `${field}.requestId`));
  nullable(value.runId, item => genericId(item, `${field}.runId`));
  prefixed(value.sessionBindingId, 'bindingId', `${field}.sessionBindingId`);
  integer(value.pageBindingGeneration, 1, 65_535, `${field}.pageBindingGeneration`);
  nullable(value.visibleUserTurnId, item => prefixed(item, 'turnId', `${field}.visibleUserTurnId`));
  nullable(value.visibleAssistantTurnId, item => prefixed(item, 'turnId', `${field}.visibleAssistantTurnId`));
  assert(typeof value.activeTurn === 'boolean', `${field}.activeTurn`);
  nullable(value.terminalAnswerSha256, item => h256(item, `${field}.terminalAnswerSha256`));
  return value;
}

export function validateSessionExpectation(value, field = 'expectation') {
  exactObject(value, ['cohort', 'conversationUrl', 'lastKnownPageBindingGeneration', 'leaseGeneration', 'leaseId', 'requestId', 'runId', 'runtimeIncarnationId', 'runtimeOwnerGeneration', 'runtimeOwnerId', 'sessionId', 'sessionOperationClaimId', 'slotId'], field);
  sessionId(value.sessionId, `${field}.sessionId`);
  conversationUrl(value.conversationUrl, value.sessionId, `${field}.conversationUrl`);
  slot(value.slotId, `${field}.slotId`);
  oneOf(value.cohort, ['cohort-a', 'cohort-b', 'cohort-c'], `${field}.cohort`);
  nullable(value.sessionOperationClaimId, item => prefixed(item, 'claimId', `${field}.sessionOperationClaimId`));
  prefixed(value.leaseId, 'leaseId', `${field}.leaseId`);
  integer(value.leaseGeneration, 1, 65_535, `${field}.leaseGeneration`);
  prefixed(value.runtimeOwnerId, 'ownerId', `${field}.runtimeOwnerId`);
  integer(value.runtimeOwnerGeneration, 1, 65_535, `${field}.runtimeOwnerGeneration`);
  prefixed(value.runtimeIncarnationId, 'runtimeIncarnationId', `${field}.runtimeIncarnationId`);
  nullable(value.requestId, item => genericId(item, `${field}.requestId`));
  nullable(value.runId, item => genericId(item, `${field}.runId`));
  integer(value.lastKnownPageBindingGeneration, 0, 65_535, `${field}.lastKnownPageBindingGeneration`);
  return value;
}

export function validateSendReceipt(value, field = 'sendReceipt') {
  exactObject(value, ['assistantTurnId', 'capturedAtMs', 'conversationUrl', 'evidenceRefs', 'kind', 'pageBinding', 'physicalClickCount', 'promptSha256', 'sendAttemptId', 'sessionId', 'userTurnId'], field);
  oneOf(value.kind, ['pre_click', 'post_click', 'reconciled_turn_start'], `${field}.kind`);
  genericId(value.sendAttemptId, `${field}.sendAttemptId`);
  validatePageBinding(value.pageBinding, `${field}.pageBinding`);
  h256(value.promptSha256, `${field}.promptSha256`);
  integer(value.physicalClickCount, 0, 1, `${field}.physicalClickCount`);
  timestamp(value.capturedAtMs, `${field}.capturedAtMs`);
  evidenceArray(value.evidenceRefs, 1, 4, `${field}.evidenceRefs`);
  const terminal = value.kind !== 'pre_click';
  assert(value.physicalClickCount === (value.kind === 'post_click' ? 1 : 0), `${field}.physicalClickCount.kind`);
  for (const key of ['userTurnId', 'assistantTurnId', 'sessionId', 'conversationUrl']) {
    assert(terminal ? value[key] !== null : value[key] === null, `${field}.${key}.nullability`);
  }
  if (terminal) {
    prefixed(value.userTurnId, 'turnId', `${field}.userTurnId`);
    prefixed(value.assistantTurnId, 'turnId', `${field}.assistantTurnId`);
    sessionId(value.sessionId, `${field}.sessionId`);
    conversationUrl(value.conversationUrl, value.sessionId, `${field}.conversationUrl`);
  }
  return value;
}

function validateProviderAttachmentSet(value, field) {
  exactObject(value, ['count', 'records', 'setSha256'], field);
  integer(value.count, 0, 64, `${field}.count`);
  const records = array(value.records, 0, 64, `${field}.records`);
  assert(records.length === value.count, `${field}.count`);
  records.forEach((record, index) => {
    exactObject(record, ['containerRelPath', 'mediaType', 'ordinal', 'sizeBytes', 'sourceSha256'], `${field}.records[${index}]`);
    assert(record.ordinal === index, `${field}.records[${index}].ordinal`);
    safeRelPath(record.containerRelPath, `${field}.records[${index}].containerRelPath`);
    h256(record.sourceSha256, `${field}.records[${index}].sourceSha256`);
    integer(record.sizeBytes, 0, 10_737_418_240, `${field}.records[${index}].sizeBytes`);
    nonEmpty(record.mediaType, `${field}.records[${index}].mediaType`);
  });
  h256(value.setSha256, `${field}.setSha256`);
}

function validatePromptInput(value, field) {
  exactObject(value, ['containerRelPath', 'sha256', 'sizeBytes'], field);
  safeRelPath(value.containerRelPath, `${field}.containerRelPath`);
  h256(value.sha256, `${field}.sha256`);
  integer(value.sizeBytes, 0, 10_737_418_240, `${field}.sizeBytes`);
}

function validateChipProof(value, field) {
  exactObject(value, ['boundingBoxHash', 'chipStableKey', 'complete', 'digest', 'evidenceRefs', 'labelHash', 'visibleSizeBytes'], field);
  h256(value.boundingBoxHash, `${field}.boundingBoxHash`);
  h256(value.chipStableKey, `${field}.chipStableKey`);
  assert(typeof value.complete === 'boolean', `${field}.complete`);
  nullable(value.digest, item => h256(item, `${field}.digest`));
  evidenceArray(value.evidenceRefs, 1, 4, `${field}.evidenceRefs`);
  h256(value.labelHash, `${field}.labelHash`);
  nullable(value.visibleSizeBytes, item => integer(item, 0, 10_737_418_240, `${field}.visibleSizeBytes`));
}

function validateUploadProof(value, field) {
  exactObject(value, ['allExpectedComplete', 'capturedAtMs', 'expectedSetSha256', 'retryIndex', 'staleChips', 'uploadAttemptId', 'visibleCurrentChips'], field);
  assert(typeof value.allExpectedComplete === 'boolean', `${field}.allExpectedComplete`);
  timestamp(value.capturedAtMs, `${field}.capturedAtMs`);
  h256(value.expectedSetSha256, `${field}.expectedSetSha256`);
  integer(value.retryIndex, 0, 1, `${field}.retryIndex`);
  genericId(value.uploadAttemptId, `${field}.uploadAttemptId`);
  array(value.staleChips, 0, 64, `${field}.staleChips`).forEach((item, index) => validateChipProof(item, `${field}.staleChips[${index}]`));
  array(value.visibleCurrentChips, 0, 64, `${field}.visibleCurrentChips`).forEach((item, index) => validateChipProof(item, `${field}.visibleCurrentChips[${index}]`));
}

function validateRootBindingCandidate(value, field) {
  exactObject(value, ['browserContextId', 'capturedAtMs', 'composerRootId', 'conversationRootId', 'domMutationGeneration', 'effortControl', 'evidenceRefs', 'modelControl', 'normalizedUrl', 'operationId', 'pageIncarnationId', 'selectorMargin', 'targetId'], field);
  prefixed(value.browserContextId, 'browserContextId', `${field}.browserContextId`);
  timestamp(value.capturedAtMs, `${field}.capturedAtMs`);
  prefixed(value.composerRootId, 'rootId', `${field}.composerRootId`);
  prefixed(value.conversationRootId, 'rootId', `${field}.conversationRootId`);
  integer(value.domMutationGeneration, 0, 65_535, `${field}.domMutationGeneration`);
  validateControlIdentity(value.effortControl, `${field}.effortControl`);
  evidenceArray(value.evidenceRefs, 1, 4, `${field}.evidenceRefs`);
  validateControlIdentity(value.modelControl, `${field}.modelControl`);
  assert(value.normalizedUrl === 'https://chatgpt.com/' || /^https:\/\/chatgpt\.com\/c\/[A-Za-z0-9][A-Za-z0-9_-]{0,127}$/.test(value.normalizedUrl), `${field}.normalizedUrl`);
  genericId(value.operationId, `${field}.operationId`);
  prefixed(value.pageIncarnationId, 'pageIncarnationId', `${field}.pageIncarnationId`);
  integer(value.selectorMargin, 50, 100_000, `${field}.selectorMargin`);
  prefixed(value.targetId, 'targetId', `${field}.targetId`);
}

function validateControlIdentity(value, field) {
  exactObject(value, ['boundingBoxHash', 'controlId', 'disabled', 'domPathHash', 'labelHash', 'role', 'testIdHash', 'visible'], field);
  h256(value.boundingBoxHash, `${field}.boundingBoxHash`);
  prefixed(value.controlId, 'controlId', `${field}.controlId`);
  assert(value.disabled === false && value.visible === true, `${field}.state`);
  h256(value.domPathHash, `${field}.domPathHash`);
  h256(value.labelHash, `${field}.labelHash`);
  oneOf(value.role, ['button', 'combobox', 'menuitem', 'option'], `${field}.role`);
  nullable(value.testIdHash, item => h256(item, `${field}.testIdHash`));
}

function validateFailureProof(value, field) {
  exactObject(value, ['controlIdentityStable', 'evidenceRefs', 'failedAtMs', 'pickerOpened', 'reason', 'requestedEffortVisible', 'requestedModelVisible'], field);
  assert(MODEL_FAILURES.has(value.reason), `${field}.reason`);
  for (const key of ['controlIdentityStable', 'pickerOpened', 'requestedEffortVisible', 'requestedModelVisible']) assert(typeof value[key] === 'boolean', `${field}.${key}`);
  evidenceArray(value.evidenceRefs, 1, 4, `${field}.evidenceRefs`);
  timestamp(value.failedAtMs, `${field}.failedAtMs`);
}

function validateModelProof(value, expected, field) {
  validateSelectionProof(value, expected, MODELS, field);
}

function validateEffortProof(value, expected, field) {
  validateSelectionProof(value, expected, EFFORTS, field);
}

function validateSelectionProof(value, expected, allowed, field) {
  exactObject(value, ['control', 'evidenceRefs', 'observed', 'requested', 'selectedBy', 'verified', 'verifiedAtMs'], field);
  assert(value.requested === expected && value.observed === expected && value.verified === true, `${field}.identity`);
  assert(allowed.has(value.requested), `${field}.requested`);
  validateControlIdentity(value.control, `${field}.control`);
  oneOf(value.selectedBy, ['already_exact', 'picker'], `${field}.selectedBy`);
  evidenceArray(value.evidenceRefs, 1, 4, `${field}.evidenceRefs`);
  timestamp(value.verifiedAtMs, `${field}.verifiedAtMs`);
}

function validateClearedChip(value, field) {
  exactObject(value, ['chipStableKey', 'cleared', 'digest'], field);
  h256(value.chipStableKey, `${field}.chipStableKey`);
  assert(value.cleared === true, `${field}.cleared`);
  nullable(value.digest, item => h256(item, `${field}.digest`));
}

function chipIdentity(value) {
  return `${value.chipStableKey}\u0000${value.digest ?? ''}`;
}

function validateHydrationObservation(value, index, field) {
  exactObject(value, ['evidenceRefs', 'observedAtMs', 'observedEcho', 'remainingDeadlineMs', 'sequenceIndex', 'state'], `${field}[${index}]`);
  assert(value.sequenceIndex === index, `${field}[${index}].sequenceIndex`);
  oneOf(value.state, ['loading_placeholder', 'blank_transient', 'active_generation_visible', 'answer_visible', 'content_unavailable'], `${field}[${index}].state`);
  integer(value.remainingDeadlineMs, 0, 90_000, `${field}[${index}].remainingDeadlineMs`);
  validateSessionEcho(value.observedEcho, `${field}[${index}].observedEcho`);
  evidenceArray(value.evidenceRefs, 1, 4, `${field}[${index}].evidenceRefs`);
  timestamp(value.observedAtMs, `${field}[${index}].observedAtMs`);
}

function validateHydrationOrder(values, field) {
  for (let index = 1; index < values.length; index += 1) {
    assert(values[index].remainingDeadlineMs <= values[index - 1].remainingDeadlineMs, `${field}[${index}].remainingDeadlineMs.order`);
    assert(values[index].observedAtMs >= values[index - 1].observedAtMs, `${field}[${index}].observedAtMs.order`);
  }
}

export function sessionMatchesExpectation(observed, expected) {
  const keys = [
    'sessionId', 'conversationUrl', 'slotId', 'cohort', 'leaseId', 'leaseGeneration',
    'runtimeOwnerId', 'runtimeOwnerGeneration', 'runtimeIncarnationId', 'requestId', 'runId',
  ];
  return keys.every(key => expected[key] === null || deepEqual(observed[key], expected[key]));
}

export function sessionEchoMatchesExpected(observed, expected) {
  return Object.keys(expected).every(key => expected[key] === null || deepEqual(observed[key], expected[key]));
}

function validateTerminalAnswer(value, field) {
  exactObject(value, ['answerRelPath', 'answerSha256', 'answerSizeBytes', 'terminalAssistantTurnId'], field);
  safeRelPath(value.answerRelPath, `${field}.answerRelPath`);
  h256(value.answerSha256, `${field}.answerSha256`);
  integer(value.answerSizeBytes, 0, 10_737_418_240, `${field}.answerSizeBytes`);
  prefixed(value.terminalAssistantTurnId, 'turnId', `${field}.terminalAssistantTurnId`);
}

function validateArtifactControl(value, turnId, field) {
  exactObject(value, ['boundingBoxHash', 'controlId', 'currentTurnId', 'disabled', 'domPathHash', 'role', 'visible', 'visibleTextHash'], field);
  h256(value.boundingBoxHash, `${field}.boundingBoxHash`);
  prefixed(value.controlId, 'controlId', `${field}.controlId`);
  prefixed(value.currentTurnId, 'turnId', `${field}.currentTurnId`);
  assert(value.currentTurnId === turnId, `${field}.currentTurnId.binding`);
  assert(value.disabled === false && value.visible === true, `${field}.state`);
  h256(value.domPathHash, `${field}.domPathHash`);
  oneOf(value.role, ['button', 'link'], `${field}.role`);
  h256(value.visibleTextHash, `${field}.visibleTextHash`);
}

function validateBottomProof(value, field) {
  exactObject(value, ['atBottom', 'capturedAtMs', 'evidenceRefs', 'method'], field);
  assert(value.atBottom === true, `${field}.atBottom`);
  oneOf(value.method, ['scrollbar', 'floating_affordance', 'dom_terminal_anchor'], `${field}.method`);
  timestamp(value.capturedAtMs, `${field}.capturedAtMs`);
  evidenceArray(value.evidenceRefs, 1, 4, `${field}.evidenceRefs`);
}

function validateZeroControlProof(value, claimId, turnId, field) {
  exactObject(value, ['artifactClaimId', 'bottomProof', 'capturedAtMs', 'controlCount', 'evidenceRefs', 'terminalAssistantTurnId'], field);
  assert(value.artifactClaimId === claimId && value.terminalAssistantTurnId === turnId && value.controlCount === 0, `${field}.binding`);
  validateBottomProof(value.bottomProof, `${field}.bottomProof`);
  timestamp(value.capturedAtMs, `${field}.capturedAtMs`);
  evidenceArray(value.evidenceRefs, 1, 4, `${field}.evidenceRefs`);
}

function validateArtifactBaseline(value, field) {
  exactObject(value, ['baselineSha256', 'capturedAtMs', 'directory', 'entries'], field);
  h256(value.baselineSha256, `${field}.baselineSha256`);
  timestamp(value.capturedAtMs, `${field}.capturedAtMs`);
  safeRelPath(value.directory, `${field}.directory`);
  array(value.entries, 0, 128, `${field}.entries`).forEach((entry, index) => {
    exactObject(entry, ['relPath', 'sha256', 'sizeBytes'], `${field}.entries[${index}]`);
    safeRelPath(entry.relPath, `${field}.entries[${index}].relPath`);
    h256(entry.sha256, `${field}.entries[${index}].sha256`);
    integer(entry.sizeBytes, 0, 10_737_418_240, `${field}.entries[${index}].sizeBytes`);
  });
}

function validateDownloadReceipt(value, field) {
  exactObject(value, ['artifactClaimId', 'artifactId', 'browserContextId', 'clickedAtMs', 'control', 'conversationUrl', 'downloadEventId', 'hostSavedRelPath', 'listenerArmedAtMs', 'mediaType', 'pageIncarnationId', 'receivedAtMs', 'sessionId', 'sha256', 'sizeBytes', 'slotId', 'targetId', 'terminalAssistantTurnId'], field);
  prefixed(value.artifactClaimId, 'artifactClaimId', `${field}.artifactClaimId`);
  prefixed(value.artifactId, 'artifactId', `${field}.artifactId`);
  prefixed(value.browserContextId, 'browserContextId', `${field}.browserContextId`);
  timestamp(value.listenerArmedAtMs, `${field}.listenerArmedAtMs`);
  timestamp(value.clickedAtMs, `${field}.clickedAtMs`);
  timestamp(value.receivedAtMs, `${field}.receivedAtMs`);
  assert(value.listenerArmedAtMs < value.clickedAtMs && value.clickedAtMs <= value.receivedAtMs, `${field}.timeOrder`);
  sessionId(value.sessionId, `${field}.sessionId`);
  conversationUrl(value.conversationUrl, value.sessionId, `${field}.conversationUrl`);
  assert(/^download_[0-9a-f]{64}$/.test(value.downloadEventId), `${field}.downloadEventId`);
  safeRelPath(value.hostSavedRelPath, `${field}.hostSavedRelPath`);
  nonEmpty(value.mediaType, `${field}.mediaType`);
  prefixed(value.pageIncarnationId, 'pageIncarnationId', `${field}.pageIncarnationId`);
  h256(value.sha256, `${field}.sha256`);
  integer(value.sizeBytes, 1, 10_737_418_240, `${field}.sizeBytes`);
  slot(value.slotId, `${field}.slotId`);
  prefixed(value.targetId, 'targetId', `${field}.targetId`);
  prefixed(value.terminalAssistantTurnId, 'turnId', `${field}.terminalAssistantTurnId`);
  validateArtifactControl(value.control, value.terminalAssistantTurnId, `${field}.control`);
  assert(
    value.artifactId === deriveArtifactId(
      value.artifactClaimId,
      value.control.controlId,
      value.downloadEventId,
    ),
    `${field}.artifactId.preimage`,
  );
  assert(
    path.posix.basename(value.hostSavedRelPath) === `${value.artifactId}.download`,
    `${field}.hostSavedRelPath.filename`,
  );
  assert(value.mediaType === mediaTypeForPath(value.hostSavedRelPath), `${field}.mediaType.oracle`);
}

function validateDownloadReceiptBinding(value, request, field) {
  const expected = request.operationData.expected;
  assert(value.artifactClaimId === request.operationData.artifactClaimId, `${field}.artifactClaimId.binding`);
  assert(value.terminalAssistantTurnId === request.operationData.terminalAssistantTurnId, `${field}.terminalAssistantTurnId.binding`);
  assert(deepEqual(value.control, request.operationData.control), `${field}.control.binding`);
  assert(value.sessionId === expected.sessionId && value.conversationUrl === expected.conversationUrl, `${field}.session.binding`);
  assert(value.slotId === expected.slotId, `${field}.slotId.binding`);
  assert(value.browserContextId === expected.browserContextId, `${field}.browserContextId.binding`);
  assert(value.pageIncarnationId === expected.pageIncarnationId, `${field}.pageIncarnationId.binding`);
  assert(value.targetId === expected.targetId, `${field}.targetId.binding`);
  assert(path.posix.dirname(value.hostSavedRelPath) === request.operationData.hostSaveDirectory, `${field}.hostSavedRelPath.binding`);
}

function mediaTypeForPath(relPath) {
  const extension = path.posix.extname(relPath).slice(1).toLowerCase();
  return Object.freeze({
    md: 'text/markdown',
    txt: 'text/plain',
    json: 'application/json',
    csv: 'text/csv',
    tsv: 'text/tab-separated-values',
    zip: 'application/zip',
    tar: 'application/x-tar',
    gz: 'application/gzip',
    png: 'image/png',
    jpg: 'image/jpeg',
    jpeg: 'image/jpeg',
    pdf: 'application/pdf',
  })[extension] ?? 'application/octet-stream';
}

function validateEvidenceRef(value, field) {
  exactObject(value, ['mediaType', 'path', 'sha256', 'sizeBytes'], field);
  assert(MEDIA_TYPES.has(value.mediaType), `${field}.mediaType`);
  safeRelPath(value.path, `${field}.path`);
  h256(value.sha256, `${field}.sha256`);
  integer(value.sizeBytes, 0, 10_737_418_240, `${field}.sizeBytes`);
}

function evidenceArray(value, min, max, field) {
  const values = array(value, min, max, field);
  values.forEach((item, index) => validateEvidenceRef(item, `${field}[${index}]`));
  assert(new Set(values.map(item => item.path)).size === values.length, `${field}.duplicates`);
}

async function writeImmutable(target, bytes, operationId) {
  genericId(operationId, 'immutable.operationId');
  const directoryPath = path.dirname(target);
  const temp = path.join(directoryPath, `.${path.basename(target)}.${operationId}.tmp`);
  let handle = null;
  try {
    handle = await open(temp, fsConstants.O_WRONLY | fsConstants.O_CREAT | fsConstants.O_EXCL | fsConstants.O_NOFOLLOW, 0o600);
    await handle.writeFile(bytes);
    await handle.sync();
    await handle.close();
    handle = null;
  } catch (error) {
    if (handle) await handle.close().catch(() => undefined);
    if (error?.code !== 'EEXIST') throw error;
    await verifyImmutableBytes(temp, bytes, `immutableTempCollision.${operationId}`);
  }

  try {
    await link(temp, target);
  } catch (error) {
    if (error?.code !== 'EEXIST') throw error;
    await verifyImmutableBytes(target, bytes, `immutableCollision.${operationId}`);
  }

  try {
    await unlink(temp);
  } catch (error) {
    if (error?.code !== 'ENOENT') throw error;
  }

  const directory = await open(directoryPath, fsConstants.O_RDONLY | fsConstants.O_DIRECTORY | fsConstants.O_NOFOLLOW);
  await directory.sync();
  await directory.close();
  await verifyImmutableBytes(target, bytes, 'immutable.reopen');
}

async function verifyImmutableBytes(target, bytes, field) {
  const info = await lstat(target);
  assert(info.isFile() && !info.isSymbolicLink() && info.nlink === 1 && info.size === bytes.length && (info.mode & 0o777) === 0o600, field);
  const existing = await readFile(target);
  assert(Buffer.compare(existing, bytes) === 0, field);
}

function modelTuple(model, effort, field) {
  assert(MODELS.has(model) && EFFORTS.has(effort), `${field}.modelEffort`);
  assert((model === 'pro' && effort === 'standard') || (model === 'xhigh' && effort === 'high'), `${field}.modelEffortTuple`);
}

function exactObject(value, keys, field) {
  assert(value !== null && typeof value === 'object' && !Array.isArray(value), field);
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  assert(actual.length === expected.length && actual.every((key, index) => key === expected[index]), `${field}.fields`);
  return value;
}

function array(value, min, max, field) {
  assert(Array.isArray(value) && value.length >= min && value.length <= max, field);
  return value;
}

function integer(value, min, max, field) {
  assert(Number.isSafeInteger(value) && value >= min && value <= max, field);
}

function timestamp(value, field) {
  integer(value, 1, Number.MAX_SAFE_INTEGER, field);
}

function nullable(value, validate) {
  if (value !== null) validate(value);
}

function nonEmpty(value, field) {
  assert(typeof value === 'string' && Buffer.byteLength(value, 'utf8') >= 1 && Buffer.byteLength(value, 'utf8') <= 4096 && !value.includes('\0'), field);
}

function h256(value, field) {
  assert(typeof value === 'string' && H256.test(value), field);
}

function genericId(value, field) {
  assert(typeof value === 'string' && Buffer.byteLength(value, 'ascii') === value.length && GENERIC_ID.test(value), field);
}

function sessionId(value, field) {
  assert(typeof value === 'string' && Buffer.byteLength(value, 'ascii') === value.length && SESSION_ID.test(value), field);
}

function prefixed(value, kind, field) {
  assert(typeof value === 'string' && PREFIXED[kind]?.test(value), field);
}

function slot(value, field) {
  assert(/^slot-(?:0[1-9]|10)$/.test(value), field);
}

function conversationUrl(value, expectedSessionId, field) {
  assert(value === `https://chatgpt.com/c/${expectedSessionId}`, field);
}

function safeRelPath(value, field) {
  assert(typeof value === 'string' && Buffer.byteLength(value, 'utf8') >= 1 && Buffer.byteLength(value, 'utf8') <= 240, field);
  assert(!path.isAbsolute(value) && !value.includes('\\') && !/[\0-\x1f\x7f]/.test(value), field);
  const parts = value.split('/');
  assert(parts.every(part => part.length > 0 && part !== '.' && part !== '..'), field);
}

function oneOf(value, values, field) {
  assert(values.includes(value), field);
}

function validateJsonValue(value, field) {
  if (value === null || typeof value === 'boolean' || typeof value === 'string') return;
  if (typeof value === 'number') {
    assert(Number.isSafeInteger(value), field);
    return;
  }
  if (Array.isArray(value)) {
    value.forEach((item, index) => validateJsonValue(item, `${field}[${index}]`));
    return;
  }
  assert(typeof value === 'object', field);
  Object.entries(value).forEach(([key, item]) => {
    assert(!key.includes('\0'), `${field}.key`);
    validateJsonValue(item, `${field}.${key}`);
  });
}

function sortJson(value) {
  if (Array.isArray(value)) return value.map(sortJson);
  if (value !== null && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort().map(key => [key, sortJson(value[key])]));
  }
  return value;
}

function deepEqual(left, right) {
  return Buffer.compare(canonicalBytes(left), canonicalBytes(right)) === 0;
}

function pick(value, keys) {
  return Object.fromEntries(keys.map(key => [key, value[key]]));
}

function assert(condition, field) {
  if (!condition) throw new R13ContractError(field);
}

const PAGE_BINDING_KEYS = Object.freeze([
  'bindingId', 'bindingGeneration', 'slotId', 'cohort', 'leaseId', 'leaseGeneration',
  'runtimeOwnerId', 'runtimeOwnerGeneration', 'runtimeIncarnationId', 'browserContextId',
  'targetId', 'pageIncarnationId', 'rootBindingHash', 'domMutationGeneration',
]);

const REQUEST_KEYS = Object.freeze({
  status: ['expectedSlotId', 'probeAttempt'],
  'capture.root': ['requestedModel', 'requestedEffort', 'rediscoveryAttempt'],
  'ensure-model': ['pageBinding', 'requestedModel', 'requestedEffort', 'pickerOpenBudget', 'stabilizationMs'],
  'upload-only': ['pageBinding', 'attachmentSet', 'uploadAttemptId', 'retryIndex'],
  'clear-upload': ['pageBinding', 'uploadAttemptId', 'clearAttemptId', 'staleChips'],
  'send-click': ['pageBinding', 'sendAttemptId', 'uploadProof', 'promptInput', 'clickBudget'],
  'send-reconcile': ['pageBinding', 'sendAttemptId', 'preClickReceipt'],
  'session-rebind': ['operationKind', 'expectation', 'navigationAttemptLimit', 'hydrationDeadlineMs'],
  poll: ['expected', 'pollAttemptId', 'pollTimeoutSeconds', 'artifactExpectation'],
  'artifact-discover': ['expected', 'artifactClaimId', 'terminalAssistantTurnId', 'expectation'],
  'artifact-click-save': ['expected', 'artifactClaimId', 'terminalAssistantTurnId', 'control', 'baseline', 'controlIndex', 'hostSaveDirectory'],
});

const RESPONSE_KEYS = Object.freeze({
  status: {
    success: ['healthStatus', 'dockerStatus', 'retryAfterMs', 'modelLabel', 'composerReady'],
    failure: ['healthStatus', 'dockerStatus', 'retryAfterMs', 'modelLabel', 'composerReady'],
  },
  'capture.root': { success: ['rootBindingCandidate', 'failureProof'], failure: ['rootBindingCandidate', 'failureProof'] },
  'ensure-model': {
    success: ['modelProof', 'effortProof', 'failureProof', 'observedPageBinding'],
    failure: ['modelProof', 'effortProof', 'failureProof', 'observedPageBinding'],
  },
  'upload-only': { success: ['uploadProof', 'failureReason', 'observedPageBinding'], failure: ['uploadProof', 'failureReason', 'observedPageBinding'] },
  'clear-upload': {
    success: ['clearAttemptId', 'clearedChips', 'observedPageBinding'],
    failure: ['clearAttemptId', 'failureReason', 'attemptedChipKeys', 'clearedChips', 'observedPageBinding'],
  },
  'send-click': { success: ['preClickReceipt', 'terminalSendReceipt', 'observedPageBinding'], failure: ['preClickReceipt', 'terminalSendReceipt', 'observedPageBinding'] },
  'send-reconcile': { success: ['preClickReceipt', 'terminalSendReceipt', 'observedPageBinding'], failure: ['preClickReceipt', 'terminalSendReceipt', 'observedPageBinding'] },
  'session-rebind': {
    success: ['expectation', 'observedEcho', 'pageBindingGeneration', 'hydrationObservations', 'terminalAnswer', 'failureReason'],
    failure: ['expectation', 'observedEcho', 'pageBindingGeneration', 'hydrationObservations', 'failureReason'],
  },
  poll: {
    success: ['expected', 'observedEcho', 'pollState', 'answerSha256', 'answerSizeBytes', 'answerRelPath', 'terminalAssistantTurnId', 'bottomProof'],
    failure: ['expected', 'observedEcho', 'pollState', 'answerSha256', 'answerSizeBytes', 'answerRelPath', 'terminalAssistantTurnId', 'bottomProof'],
  },
  'artifact-discover': {
    success: ['controls', 'bottomProof', 'zeroControlProof', 'failureReason', 'observedEcho'],
    failure: ['controls', 'bottomProof', 'zeroControlProof', 'failureReason', 'observedEcho'],
  },
  'artifact-click-save': { success: ['downloadReceipt', 'failureReason', 'observedEcho'], failure: ['downloadReceipt', 'failureReason', 'observedEcho'] },
});
