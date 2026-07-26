import { createHash } from 'node:crypto';
import { constants as fsConstants } from 'node:fs';
import {
  link,
  lstat,
  open,
  readFile,
  unlink,
} from 'node:fs/promises';

import {
  artifactHostSavedRelPath,
  canonicalSha256,
  deriveArtifactId,
  deriveDownloadEventId,
  deriveTurnId,
  resolveEvidencePath,
  sessionEchoMatchesExpected,
} from './contracts/r13.mjs';
import { scrollPrimaryConversationToBottom } from './bottom-scroll.mjs';
import { hitFailpoint, sha256Text } from './common.mjs';

export async function handleArtifactDiscover(context, overrides = {}) {
  const { request, page, evidenceRefs } = context;
  const dependencies = {
    discoverArtifactControls,
    proveBottom,
    ...overrides,
  };
  const expected = request.operationData.expected;
  const observed = await context.observeSession(expected.sessionId, expected.conversationUrl);
  if (!sessionEchoMatchesExpected(observed.observedEcho, expected)
      || observed.observedEcho.visibleAssistantTurnId
      !== request.operationData.terminalAssistantTurnId) {
    return discoverFailure('artifact.controls_ambiguous', observed.observedEcho);
  }
  const bottomProof = await dependencies.proveBottom(page, evidenceRefs);
  if (bottomProof === null) {
    return discoverFailure('artifact.bottom_unverified', observed.observedEcho);
  }
  const controls = await dependencies.discoverArtifactControls(
    page,
    request.operationData.terminalAssistantTurnId,
    expected.sessionId,
  );
  if (controls === null || controls.length > 64
      || new Set(controls.map(control => control.controlId)).size !== controls.length) {
    return discoverFailure('artifact.controls_ambiguous', observed.observedEcho);
  }
  const zeroControlProof = controls.length === 0 ? {
    artifactClaimId: request.operationData.artifactClaimId,
    bottomProof,
    capturedAtMs: Date.now(),
    controlCount: 0,
    evidenceRefs,
    terminalAssistantTurnId: request.operationData.terminalAssistantTurnId,
  } : null;
  return {
    ok: true,
    status: 'done',
    providerReason: null,
    operationData: {
      bottomProof,
      controls,
      failureReason: null,
      observedEcho: observed.observedEcho,
      zeroControlProof,
    },
  };
}

export async function handleArtifactClickSave(context, overrides = {}) {
  const { request } = context;
  const dependencies = { clickAndSaveArtifact, ...overrides };
  const expected = request.operationData.expected;
  const captureEvidence = context.captureEvidence ?? (async () => context.evidenceRefs);
  await captureEvidence();
  const observed = await context.observeSession(expected.sessionId, expected.conversationUrl);
  if (!sessionEchoMatchesExpected(observed.observedEcho, expected)
      || observed.observedEcho.visibleAssistantTurnId
        !== request.operationData.terminalAssistantTurnId) {
    return artifactSaveFailure('artifact.integrity_failed', observed.observedEcho);
  }
  try {
    const downloadReceipt = await dependencies.clickAndSaveArtifact(context);
    return {
      ok: true,
      status: 'done',
      providerReason: null,
      operationData: {
        downloadReceipt,
        failureReason: null,
        observedEcho: observed.observedEcho,
      },
    };
  } catch (error) {
    const reason = artifactFailureReason(error);
    return artifactSaveFailure(reason, observed.observedEcho);
  }
}

export async function proveBottom(page, evidenceRefs) {
  await scrollPrimaryConversationToBottom(page, { label: 'r13-artifact-discover' });
  const atBottom = await page.evaluate(() => {
    const candidates = [document.scrollingElement, ...document.querySelectorAll(
      'main,[data-testid*="conversation" i],[class*="conversation" i],[class*="scroll" i]',
    )].filter(Boolean);
    return candidates.some(node => {
      const remaining = Number(node.scrollHeight || 0)
        - Number(node.scrollTop || 0)
        - Number(node.clientHeight || 0);
      return remaining <= 2;
    });
  }).catch(() => false);
  if (!atBottom) return null;
  return {
    atBottom: true,
    capturedAtMs: Date.now(),
    evidenceRefs,
    method: 'dom_terminal_anchor',
  };
}

export async function discoverArtifactControls(page, terminalAssistantTurnId, sessionId = '') {
  const turn = await resolveTerminalAssistantTurn(page, sessionId, terminalAssistantTurnId);
  if (turn === null) return null;
  const entries = await collectArtifactControlEntries(turn.root);
  return entries.map(({ candidate }) => buildArtifactControl(candidate, terminalAssistantTurnId));
}

export function selectTerminalAssistantIndex(observations, sessionId, terminalAssistantTurnId) {
  const matches = observations
    .map((item, index) => ({ ...item, index }))
    .filter(item => item.visible && item.dataMessageId)
    .filter(item => deriveTurnId(sessionId, 'assistant', item.dataMessageId)
      === terminalAssistantTurnId);
  return matches.length === 1 ? matches[0].index : null;
}

export function buildArtifactControl(candidate, terminalAssistantTurnId) {
  const domPathHash = `sha256:${canonicalSha256(candidate.domPath.map(([tag, index]) => [
    String(tag).toLowerCase(), index,
  ]))}`;
  const boundingBoxHash = `sha256:${canonicalSha256([
    Math.round(candidate.boundingBox.x),
    Math.round(candidate.boundingBox.y),
    Math.round(candidate.boundingBox.width),
    Math.round(candidate.boundingBox.height),
  ])}`;
  const testIdHash = candidate.testId ? `sha256:${sha256Text(candidate.testId)}` : null;
  const ariaLabelHash = candidate.ariaLabel
    ? `sha256:${sha256Text(candidate.ariaLabel)}`
    : null;
  const controlId = `control_${canonicalSha256([
    candidate.tagName.toLowerCase(),
    candidate.role || '',
    testIdHash ?? '',
    ariaLabelHash ?? '',
    domPathHash,
    boundingBoxHash,
  ])}`;
  return {
    boundingBoxHash,
    controlId,
    currentTurnId: terminalAssistantTurnId,
    disabled: candidate.disabled === true,
    domPathHash,
    role: candidate.role === 'link' ? 'link' : 'button',
    visible: true,
    visibleTextHash: `sha256:${sha256Text(candidate.visibleText || '')}`,
  };
}

export async function clickAndSaveArtifact(context) {
  const {
    request,
    page,
    browser,
    artifactsRoot,
    observeSession,
  } = context;
  const captureEvidence = context.captureEvidence ?? (async () => context.evidenceRefs);
  await captureEvidence();
  const expected = request.operationData.expected;
  const observed = await observeSession(expected.sessionId, expected.conversationUrl);
  if (!sessionEchoMatchesExpected(observed.observedEcho, expected)
      || observed.observedEcho.visibleAssistantTurnId
        !== request.operationData.terminalAssistantTurnId) {
    throw new Error('artifact.integrity_failed');
  }
  const discovered = await discoverArtifactControls(
    page,
    request.operationData.terminalAssistantTurnId,
    request.operationData.expected.sessionId,
  );
  if (discovered === null) throw new Error('artifact.integrity_failed');
  const matches = discovered
    .map((control, index) => ({ control, index }))
    .filter(item => item.control.controlId === request.operationData.control.controlId);
  if (matches.length !== 1) throw new Error('artifact.integrity_failed');

  const turn = await resolveTerminalAssistantTurn(
    page,
    request.operationData.expected.sessionId,
    request.operationData.terminalAssistantTurnId,
  );
  if (turn === null) throw new Error('artifact.integrity_failed');
  const entries = await collectArtifactControlEntries(turn.root);
  const matchingLocators = [];
  for (const { candidate, locator } of entries) {
    const identity = buildArtifactControl(
      candidate,
      request.operationData.terminalAssistantTurnId,
    );
    if (identity.controlId === request.operationData.control.controlId) matchingLocators.push(locator);
  }
  if (matchingLocators.length !== 1) throw new Error('artifact.integrity_failed');

  if (typeof browser.newBrowserCDPSession !== 'function') {
    throw new Error('artifact.event_unrecoverable');
  }
  const cdp = await browser.newBrowserCDPSession();
  const pageCdp = await page.context().newCDPSession(page);
  const starts = [];
  const progress = new Map();
  cdp.on('Browser.downloadWillBegin', event => starts.push(event));
  cdp.on('Browser.downloadProgress', event => progress.set(event.guid, event));
  try {
    await cdp.send('Browser.setDownloadBehavior', {
    behavior: 'allow',
    downloadPath: artifactsRoot,
    eventsEnabled: true,
  }).catch(() => undefined);

  let listenerArmedAtMs;
  let clickedAtMs;
  let download;
  let start;
  try {
    const { frameTree } = await pageCdp.send('Page.getFrameTree');
    const mainFrameId = frameTree?.frame?.id;
    if (!mainFrameId) throw new Error('artifact.event_unrecoverable');
    listenerArmedAtMs = Date.now();
    const playwrightDownload = page.waitForEvent('download', {
      timeout: Math.min(request.deadlineMs, 120_000),
    });
    hitFailpoint('after-artifact-listener-arm');
    while (Date.now() <= listenerArmedAtMs) await new Promise(resolve => setTimeout(resolve, 1));
    clickedAtMs = Date.now();
    await matchingLocators[0].click({ timeout: Math.min(request.deadlineMs, 30_000) });
    hitFailpoint('after-artifact-click');
    download = await playwrightDownload;
    await captureEvidence();
    start = await waitForCorrelatedDownload({
      starts,
      progress,
      frameId: mainFrameId,
      url: download.url(),
      suggestedFilename: download.suggestedFilename(),
      timeoutMs: request.deadlineMs,
    });
  } finally {
    await pageCdp.detach().catch(() => undefined);
  }
  const suggestedFilename = start.suggestedFilename;
  const downloadEventId = deriveDownloadEventId(
    request.operationData.expected.pageIncarnationId,
    start.guid,
    suggestedFilename,
  );
  const artifactId = deriveArtifactId(
    request.operationData.artifactClaimId,
    request.operationData.control.controlId,
    downloadEventId,
  );
  const requestKey = request.identity.requestId
    ? `r-${request.identity.requestId}`
    : `s-${request.identity.sessionId}`;
  const hostSavedRelPath = artifactHostSavedRelPath(
    requestKey,
    request.operationData.artifactClaimId,
    artifactId,
  );
  if (hostSavedRelPath.split('/').slice(0, -1).join('/')
      !== request.operationData.hostSaveDirectory) {
    throw new Error('artifact.path_unsafe');
  }
  const target = await resolveArtifactDownloadTarget({
    artifactsRoot,
    hostSavedRelPath,
    requestKey,
  });
  const temporary = `${target}.${request.identity.operationId}.tmp`;
  await download.saveAs(temporary);
  const tempInfo = await lstat(temporary);
  if (!tempInfo.isFile() || tempInfo.isSymbolicLink() || tempInfo.nlink !== 1
      || tempInfo.size < 1) throw new Error('artifact.integrity_failed');
  try {
    await link(temporary, target);
  } catch (error) {
    if (error?.code !== 'EEXIST') throw error;
    throw new Error('artifact.path_unsafe');
  }
  await unlink(temporary);
  const directory = await open(
    target.split('/').slice(0, -1).join('/'),
    fsConstants.O_RDONLY | fsConstants.O_DIRECTORY | fsConstants.O_NOFOLLOW,
  );
  await directory.sync();
  await directory.close();
  const info = await lstat(target);
  const bytes = await readFile(target);
  if (!info.isFile() || info.isSymbolicLink() || info.nlink !== 1
      || info.size !== bytes.length || bytes.length < 1) {
    throw new Error('artifact.integrity_failed');
  }
  hitFailpoint('after-playwright-host-save-before-receipt');
  const receivedAtMs = Math.max(clickedAtMs, Date.now());
  return {
    artifactClaimId: request.operationData.artifactClaimId,
    artifactId,
    browserContextId: expected.browserContextId,
    clickedAtMs,
    control: request.operationData.control,
    conversationUrl: expected.conversationUrl,
    downloadEventId,
    hostSavedRelPath,
    listenerArmedAtMs,
    mediaType: 'application/octet-stream',
    pageIncarnationId: expected.pageIncarnationId,
    receivedAtMs,
    sessionId: expected.sessionId,
    sha256: `sha256:${createHash('sha256').update(bytes).digest('hex')}`,
    sizeBytes: bytes.length,
    slotId: expected.slotId,
    targetId: expected.targetId,
    terminalAssistantTurnId: request.operationData.terminalAssistantTurnId,
  };
  } finally {
    await cdp.detach().catch(() => undefined);
  }
}

export async function resolveArtifactDownloadTarget({
  artifactsRoot,
  hostSavedRelPath,
  requestKey,
}) {
  const prefix = `artifacts/${requestKey}/`;
  if (!hostSavedRelPath.startsWith(prefix)) throw new Error('artifact.path_unsafe');
  const mountedRelPath = hostSavedRelPath.slice(prefix.length);
  if (!mountedRelPath) throw new Error('artifact.path_unsafe');
  return resolveEvidencePath(artifactsRoot, mountedRelPath);
}

export function selectCorrelatedDownloadStart(starts, {
  frameId,
  url,
  suggestedFilename,
}) {
  const matches = starts.filter(event => event?.guid
    && event.frameId === frameId
    && event.url === url
    && event.suggestedFilename === suggestedFilename);
  if (matches.length > 1) throw new Error('artifact.event_unrecoverable');
  return matches[0] ?? null;
}

async function waitForCorrelatedDownload({
  starts,
  progress,
  frameId,
  url,
  suggestedFilename,
  timeoutMs,
}) {
  const deadline = Date.now() + Math.min(timeoutMs, 120_000);
  do {
    const start = selectCorrelatedDownloadStart(starts, {
      frameId,
      url,
      suggestedFilename,
    });
    if (start !== null) {
      const state = progress.get(start.guid)?.state;
      if (state === 'completed') return start;
      if (state === 'canceled') throw new Error('artifact.event_unrecoverable');
    }
    await new Promise(resolve => setTimeout(resolve, 10));
  } while (Date.now() < deadline);
  throw new Error('artifact.event_unrecoverable');
}

export async function resolveTerminalAssistantTurn(page, sessionId, terminalAssistantTurnId) {
  const messages = page.locator('[data-message-author-role="assistant"]');
  const observations = [];
  const count = await messages.count().catch(() => 0);
  for (let index = 0; index < count; index += 1) {
    const locator = messages.nth(index);
    observations.push({
      dataMessageId: await locator.getAttribute('data-message-id').catch(() => null),
      visible: await locator.isVisible().catch(() => false),
    });
  }
  const index = selectTerminalAssistantIndex(
    observations,
    sessionId,
    terminalAssistantTurnId,
  );
  if (index === null) return null;
  const message = messages.nth(index);
  const article = message.locator(
    'xpath=ancestor-or-self::*[starts-with(@data-testid, "conversation-turn")][1]',
  );
  if (await article.count().catch(() => 0) !== 1) return null;
  return { message, root: article };
}

async function collectArtifactControlEntries(root) {
  const controls = root.locator('button,[role="button"],a[download]');
  const entries = [];
  const count = await controls.count().catch(() => 0);
  for (let index = 0; index < count; index += 1) {
    const locator = controls.nth(index);
    if (!await locator.isVisible().catch(() => false)) continue;
    const candidate = await locator.evaluate(node => {
      const domPath = value => {
        const result = [];
        let current = value;
        while (current?.nodeType === Node.ELEMENT_NODE) {
          let ordinal = 0;
          let sibling = current.previousElementSibling;
          while (sibling) {
            ordinal += 1;
            sibling = sibling.previousElementSibling;
          }
          result.unshift([String(current.tagName || '').toLowerCase(), ordinal]);
          current = current.parentElement;
        }
        return result;
      };
      const rect = node.getBoundingClientRect();
      const signal = `${node.getAttribute('aria-label') || ''} ${node.getAttribute('title') || ''} ${node.innerText || node.textContent || ''}`;
      return {
        artifactSignal: node.hasAttribute('download')
          || /download|save file|artifact|\.zip\b|\.csv\b|\.tsv\b|\.json\b|\.md\b|다운로드|저장/i.test(signal),
        ariaLabel: node.getAttribute('aria-label') || '',
        boundingBox: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
        disabled: Boolean(node.disabled || node.getAttribute('aria-disabled') === 'true'),
        domPath: domPath(node),
        role: node.getAttribute('role') || (node.tagName === 'A' ? 'link' : 'button'),
        tagName: String(node.tagName || '').toLowerCase(),
        testId: node.getAttribute('data-testid') || '',
        visibleText: (node.innerText || node.textContent || '').trim(),
      };
    });
    if (candidate.artifactSignal !== true || candidate.disabled === true) continue;
    const { artifactSignal: _artifactSignal, ...identityCandidate } = candidate;
    entries.push({ candidate: identityCandidate, locator });
  }
  return entries;
}

function discoverFailure(providerReason, observedEcho) {
  return {
    ok: false,
    status: 'failed',
    providerReason,
    operationData: {
      bottomProof: null,
      controls: [],
      failureReason: providerReason,
      observedEcho,
      zeroControlProof: null,
    },
  };
}

function artifactSaveFailure(providerReason, observedEcho) {
  return {
    ok: false,
    status: 'failed',
    providerReason,
    operationData: {
      downloadReceipt: null,
      failureReason: providerReason,
      observedEcho,
    },
  };
}

function artifactFailureReason(error) {
  const message = error instanceof Error ? error.message : String(error);
  for (const reason of [
    'artifact.download_timeout',
    'artifact.event_unrecoverable',
    'artifact.integrity_failed',
    'artifact.path_unsafe',
  ]) if (message.includes(reason)) return reason;
  return /timeout/i.test(message) ? 'artifact.download_timeout' : 'artifact.integrity_failed';
}
