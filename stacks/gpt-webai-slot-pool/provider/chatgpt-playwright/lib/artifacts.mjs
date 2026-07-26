import { mkdir } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';

import {
  DEFAULT_DOWNLOAD_TIMEOUT_MS,
  fileSize,
  filenameFromText,
  mimeFromName,
  sanitizeFilename,
  sha256File,
  sha256Text,
} from './common.mjs';
import {
  annotateSidecarRelationships,
  canonicalArtifactObject,
} from './artifact-objects.mjs';
import { scrollPrimaryConversationToBottom } from './bottom-scroll.mjs';
import {
  assistantTurnLocator,
  candidateDedupeKey,
  candidateInCurrentTurnScope,
  candidateSnapshot,
  filenameMatchesExpected,
  normalizeExpectedFilenames,
} from './artifact-scope.mjs';
import {
  sanitizeArtifactWarningMessage,
  sanitizedCandidateSnapshot,
  saveDownloadByClickingCandidate,
} from './artifact-download-save.mjs';

async function saveCandidate({
  page,
  item,
  sessionId,
  turnIndex,
  turnScope,
  buttonText,
  filename,
  candidateIndex,
  snapshot,
  downloadsDir,
  hostRoot,
  artifacts,
  artifactCandidates,
  warnings,
}) {
  const snapshotTurnText = snapshot?.turnText || '';
  const snapshotAssistantTurnText = snapshot?.assistantTurnText || '';
  const safeSnapshot = sanitizedCandidateSnapshot(snapshot);
  const safe = sanitizeFilename(filename, `artifact-${candidateIndex}.bin`);
  const savedName = `${String(candidateIndex).padStart(3, '0')}-${safe}`;
  const containerPath = path.join(downloadsDir, savedName);
  const hostPath = path.join(hostRoot, 'downloads', savedName);
  const timeout = Number.parseInt(process.env.GPT_WEBAI_DOWNLOAD_TIMEOUT_MS || '', 10) || DEFAULT_DOWNLOAD_TIMEOUT_MS;

  let phase = 'download.waitForEvent';
  let suggestedFilename = '';
  let saveAttempts = 0;
  let recoveredFrom = '';
  try {
    const result = await saveDownloadByClickingCandidate({ page, item, containerPath, timeout, directDownloadDir: downloadsDir });
    suggestedFilename = result.suggestedFilename;
    saveAttempts = result.saveAttempts;
    recoveredFrom = result.recoveredFrom || '';
    const clickedElement = {
      ...safeSnapshot,
      turnIndex,
      turnTextSha256: snapshotTurnText ? sha256Text(snapshotTurnText) : '',
      assistantTurnTextSha256: snapshotAssistantTurnText ? sha256Text(snapshotAssistantTurnText) : '',
    };
    const artifact = {
      status: 'saved',
      visibleFilename: filename,
      suggestedFilename,
      finalFilename: savedName,
      savedPath: hostPath,
      hostPath,
      containerSavedPath: containerPath,
      containerPath,
      size: await fileSize(containerPath),
      sha256: await sha256File(containerPath),
      mime: mimeFromName(savedName),
      fileType: path.extname(savedName).replace(/^\./, '') || 'file',
      type: path.extname(savedName).replace(/^\./, '') || 'file',
      saveAttempts,
      recoveredFrom: recoveredFrom || undefined,
    };
    const object = canonicalArtifactObject({
      sessionId,
      buttonText,
      turnIndex,
      turnScope,
      clickedElement,
      artifact,
    });
    artifacts.push(object);
    artifactCandidates.push(object);
  } catch (error) {
    phase = error instanceof Error && error.phase ? error.phase : phase;
    suggestedFilename = error instanceof Error && error.suggestedFilename ? error.suggestedFilename : suggestedFilename;
    saveAttempts = error instanceof Error && Number.isFinite(Number(error.saveAttempts)) ? Number(error.saveAttempts) : saveAttempts;
    const clickedElement = {
      ...safeSnapshot,
      turnIndex,
      turnTextSha256: snapshotTurnText ? sha256Text(snapshotTurnText) : '',
      assistantTurnTextSha256: snapshotAssistantTurnText ? sha256Text(snapshotAssistantTurnText) : '',
    };
    const warning = {
      reason: 'artifact.download_timeout',
      phase,
      candidateIndex,
      turnIndex,
      visibleFilename: filename,
      suggestedFilename,
      containerSavedPath: containerPath,
      savedPath: hostPath,
      saveAttempts,
      message: sanitizeArtifactWarningMessage(error, snapshot, filename),
    };
    warnings.push(warning);
    artifactCandidates.push(canonicalArtifactObject({
      sessionId,
      buttonText,
      turnIndex,
      turnScope,
      clickedElement,
      artifact: {
        status: 'failed',
        reason: warning.reason,
        phase,
        visibleFilename: filename,
        suggestedFilename,
        finalFilename: savedName,
        savedPath: hostPath,
        hostPath,
        containerSavedPath: containerPath,
        containerPath,
        saveAttempts,
      },
    }));
  }
}

async function configureBrowserDownloadPath(page, downloadsDir) {
  const context = typeof page.context === 'function' ? page.context() : null;
  if (!context || typeof context.newCDPSession !== 'function') return;
  let session = null;
  try {
    session = await context.newCDPSession(page);
    await session.send('Browser.setDownloadBehavior', {
      behavior: 'allow',
      downloadPath: downloadsDir,
      eventsEnabled: true,
    });
  } catch {
    // Playwright's download event path remains the primary path; this is a best-effort fallback.
  } finally {
    await session?.detach?.().catch(() => undefined);
  }
}

function genericDownloadFilename(buttonText, candidateIndex) {
  const text = String(buttonText || '').trim();
  if (!/\b(download|file|attachment)\b/i.test(text)) return '';
  if (/\b(?:sha-?256|checksum|sidecar)\b/i.test(text)) return `artifact-${candidateIndex}.zip.sha256`;
  if (/\b(?:zip|archive|bundle)\b/i.test(text)) return `artifact-${candidateIndex}.zip`;
  if (/\b(?:text|txt)\b/i.test(text)) return `artifact-${candidateIndex}.txt`;
  return `artifact-${candidateIndex}.bin`;
}

export async function downloadArtifacts(page, sessionId, options = {}) {
  const scopedTurnIndexes = Array.isArray(options.turnIndexes) ? new Set(options.turnIndexes.map(Number)) : null;
  const containerRoot = process.env.GPT_WEBAI_ARTIFACTS_DIR || '/broker-artifacts/manual';
  const hostRoot = process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR || containerRoot;
  const downloadsDir = path.join(containerRoot, 'downloads');
  await mkdir(downloadsDir, { recursive: true });
  const bottomScroll = await scrollPrimaryConversationToBottom(page, { label: 'artifact-discovery' });
  await configureBrowserDownloadPath(page, downloadsDir);
  const turns = await assistantTurnLocator(page);
  const turnCount = await turns.count().catch(() => 0);
  const artifacts = [];
  const artifactCandidates = [];
  const warnings = [];
  const expectedFilenames = normalizeExpectedFilenames(options.expectedFilenames);
  const attemptedCandidateKeys = new Set();
  const completedFilenames = new Set();
  const previewAttemptedFilenames = new Set();
  const scopedTurnTextHashes = new Set();
  const scopedTurnBounds = [];
  let candidateIndex = 0;

  function isDownloadControlText(value) {
    return /^\s*download(?:\s+(?:file|artifact))?\s*$/i.test(String(value || ''));
  }

  async function tryPreviewDownload(expectedFilename, turnIndex, turnScope) {
    const locator = page.locator('button.behavior-btn, button, [role="button"], a[download], [role="link"]');
    const count = await locator.count().catch(() => 0);
    for (let index = 0; index < count; index += 1) {
      const item = locator.nth(index);
      if (!await item.isVisible().catch(() => false)) continue;
      const snapshot = await candidateSnapshot(item, candidateIndex + 1).catch(() => null);
      const visibleText = String(snapshot?.visibleText || '').trim();
      const accessibleName = String(snapshot?.accessibleName || '').trim();
      if (!isDownloadControlText(visibleText) && !isDownloadControlText(accessibleName)) continue;
      const dedupeKey = candidateDedupeKey(snapshot);
      if (dedupeKey && attemptedCandidateKeys.has(dedupeKey)) continue;
      if (dedupeKey) attemptedCandidateKeys.add(dedupeKey);
      candidateIndex += 1;
      await saveCandidate({
        page,
        item,
        sessionId,
        turnIndex,
        turnScope,
        buttonText: visibleText || accessibleName,
        filename: expectedFilename,
        candidateIndex,
        snapshot,
        downloadsDir,
        hostRoot,
        artifacts,
        artifactCandidates,
        warnings,
      });
      if (artifacts.length > 0) completedFilenames.add(String(expectedFilename).toLowerCase());
      return true;
    }
    return false;
  }

  async function waitForPreviewDownload(expectedFilename, turnIndex, turnScope) {
    for (let attempt = 0; attempt < 12; attempt += 1) {
      if (await tryPreviewDownload(expectedFilename, turnIndex, turnScope)) return true;
      await new Promise(resolve => setTimeout(resolve, 250));
    }
    return false;
  }

  async function tryCandidate(item, turnIndex, turnScope, requireExpectedFilename = false) {
    if (!await item.isVisible().catch(() => false)) return false;
    const snapshot = await candidateSnapshot(item, candidateIndex + 1).catch(() => null);
    const buttonText = String(snapshot?.visibleText || '').trim();
    const filename = filenameFromText(buttonText) || genericDownloadFilename(buttonText, candidateIndex + 1);
    if (!buttonText || !filename) return false;
    const normalizedFilename = String(filename).toLowerCase();
    if (expectedFilenames.has(normalizedFilename)
      && (completedFilenames.has(normalizedFilename) || previewAttemptedFilenames.has(normalizedFilename))) return false;
    if (requireExpectedFilename) {
      if (!filenameMatchesExpected(filename, expectedFilenames)) return false;
      if (!candidateInCurrentTurnScope(snapshot, scopedTurnTextHashes, scopedTurnBounds)) return false;
    }
    const dedupeKey = candidateDedupeKey(snapshot);
    if (dedupeKey && attemptedCandidateKeys.has(dedupeKey)) return false;
    if (dedupeKey) attemptedCandidateKeys.add(dedupeKey);

    // In the current ChatGPT file-card UI, clicking the filename opens a
    // preview pane and exposes a second visible Download button. Treat that
    // filename control as an opener, then bind the actual download to the
    // expected filename rather than waiting 30s for a download event that the
    // opener will never emit.
    if (filenameMatchesExpected(filename, expectedFilenames) && !isDownloadControlText(buttonText) && snapshot?.fileCardOpener === true) {
      previewAttemptedFilenames.add(normalizedFilename);
      candidateIndex += 1;
      await item.click({ timeout: 10_000 }).catch(() => undefined);
      const downloaded = await waitForPreviewDownload(filename, turnIndex, 'current-answer-download-panel');
      if (!downloaded) {
        warnings.push({
          reason: 'artifact.download_timeout',
          phase: 'download.preview-control',
          candidateIndex,
          turnIndex,
          visibleFilename: filename,
          suggestedFilename: '',
          saveAttempts: 0,
          message: 'visible filename control opened no downloadable preview control',
        });
      } else {
        completedFilenames.add(normalizedFilename);
      }
      return true;
    }
    candidateIndex += 1;
    await saveCandidate({
      page,
      item,
      sessionId,
      turnIndex,
      turnScope,
      buttonText,
      filename,
      candidateIndex,
      snapshot,
      downloadsDir,
      hostRoot,
      artifacts,
      artifactCandidates,
      warnings,
    });
    return true;
  }

  for (let turnIndex = 0; turnIndex < turnCount; turnIndex += 1) {
    if (scopedTurnIndexes && !scopedTurnIndexes.has(turnIndex)) continue;
    const turn = turns.nth(turnIndex);
    if (!await turn.isVisible().catch(() => false)) continue;
    const turnText = await turn.evaluate(node => (node.innerText || node.textContent || '').trim()).catch(() => '');
    if (turnText) scopedTurnTextHashes.add(sha256Text(turnText));
    const turnBox = await turn.boundingBox().catch(() => null);
    if (turnBox) scopedTurnBounds.push(turnBox);
    // Some ChatGPT file cards render the filename as a generic clickable text
    // node (without href/role/button). When the answer declared an expected
    // filename, target that exact visible node before broad control scanning;
    // clicking it still goes through the Playwright download listener and
    // avoids treating the visible file card as absent.
    if (expectedFilenames.size > 0 && typeof turn.getByText === 'function') {
      for (const expectedFilename of expectedFilenames) {
        const exact = turn.getByText(expectedFilename, { exact: true });
        const exactCount = await exact.count().catch(() => 0);
        for (let index = 0; index < exactCount; index += 1) {
          await tryCandidate(exact.nth(index), turnIndex, scopedTurnIndexes ? 'current-assistant-turn' : 'all-assistant-turns', true);
        }
      }
    }
    // ChatGPT's file card has appeared as a native link, a behavior button,
    // and (in newer UI variants) a clickable element with role="button".
    // Keep the candidate set UI-first and let filenameFromText() reject
    // unrelated controls; omitting role=button turns a visible file card into
    // a false artifact.controls_absent result.
    const locator = turn.locator('button.behavior-btn, a[download], a[href], button, [role="button"], [role="link"]');
    const count = await locator.count().catch(() => 0);
    for (let index = 0; index < count; index += 1) {
      await tryCandidate(locator.nth(index), turnIndex, scopedTurnIndexes ? 'current-assistant-turn' : 'all-assistant-turns');
    }
  }

  if (artifacts.length === 0 && expectedFilenames.size > 0) {
    if (typeof page.getByText === 'function') {
      for (const expectedFilename of expectedFilenames) {
        const exact = page.getByText(expectedFilename, { exact: true });
        const exactCount = await exact.count().catch(() => 0);
        for (let index = 0; index < exactCount; index += 1) {
          await tryCandidate(exact.nth(index), null, 'current-answer-filename-fallback', true);
        }
      }
    }
    const locator = page.locator('button.behavior-btn, a[download], a[href], button, [role="button"], [role="link"]');
    const count = await locator.count().catch(() => 0);
    for (let index = 0; index < count; index += 1) {
      await tryCandidate(locator.nth(index), null, 'current-answer-filename-fallback', true);
    }
  }

  await annotateSidecarRelationships(artifacts);
  return { artifacts, artifactCandidates, warnings, downloadCandidateCount: candidateIndex, bottomScroll };
}
