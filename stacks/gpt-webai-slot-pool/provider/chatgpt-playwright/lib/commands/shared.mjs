import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';

import { downloadArtifacts } from '../artifacts.mjs';
import {
  artifactDownloadFailureStatus,
  artifactExpectationRequiresControls,
  artifactFilenamesFromText,
  sha256Text,
} from '../common.mjs';
import { hostPathFor, writePageDiagnostics } from '../diagnostics.mjs';


export async function writeDurableSendStartEvidence(payload = {}) {
  try {
    const root = process.env.GPT_WEBAI_ARTIFACTS_DIR || process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR || '/broker-artifacts/manual';
    const diagnosticsDir = path.join(root, 'diagnostics');
    await mkdir(diagnosticsDir, { recursive: true });
    const evidencePath = path.join(diagnosticsDir, 'send-start-confirmation.json');
    const evidence = {
      schema: 'gpt-webai.send-start-confirmation.v1',
      capturedAt: new Date().toISOString(),
      ...payload,
    };
    await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, 'utf8');
    return { status: 'saved', path: hostPathFor(evidencePath) };
  } catch (error) {
    return {
      status: 'failed',
      message: error instanceof Error ? error.message : String(error),
    };
  }
}

export async function writeModelSelectionEvidence(payload = {}) {
  try {
    const root = process.env.GPT_WEBAI_ARTIFACTS_DIR || process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR || '/broker-artifacts/manual';
    const diagnosticsDir = path.join(root, 'diagnostics');
    await mkdir(diagnosticsDir, { recursive: true });
    const evidencePath = path.join(diagnosticsDir, 'model-selection.json');
    await writeFile(evidencePath, `${JSON.stringify({
      schema: 'gpt-webai.model-selection-evidence.v1',
      capturedAt: new Date().toISOString(),
      ...payload,
    }, null, 2)}\n`, 'utf8');
    return { status: 'saved', path: hostPathFor(evidencePath) };
  } catch (error) {
    return { status: 'failed', message: error instanceof Error ? error.message : String(error) };
  }
}

export function targetIdForUrl(url) {
  return `target-${sha256Text(url || '').slice(0, 16)}`;
}

export async function captureDiagnostics(page, label, sessionId = '') {
  try {
    return await writePageDiagnostics(page, { label, sessionId });
  } catch (error) {
    return {
      label,
      status: 'failed',
      message: error instanceof Error ? error.message : String(error),
    };
  }
}

export async function attachExpectedArtifactsForTerminalAnswer(page, sessionId, payload, answerText, assistantTurn, artifactExpectation = 'optional') {
  const artifactExpected = artifactExpectationRequiresControls(artifactExpectation, answerText);
  payload.artifactExpectation = artifactExpectation;
  if (!artifactExpected) return payload;
  const expectedFilenames = artifactFilenamesFromText(answerText);
  const turnIndexes = assistantTurn && Number.isFinite(Number(assistantTurn.turnIndex)) ? [assistantTurn.turnIndex] : [];
  const { artifacts, artifactCandidates, warnings, downloadCandidateCount, bottomScroll } = await downloadArtifacts(page, sessionId, { turnIndexes, expectedFilenames });
  payload.artifacts = artifacts;
  payload.artifactCandidates = artifactCandidates;
  payload.warnings = warnings;
  payload.downloadCandidateCount = downloadCandidateCount;
  payload.artifactDiscoveryBottomScroll = bottomScroll;
  const failureStatus = artifactDownloadFailureStatus({ artifactExpected, artifacts, warnings, downloadCandidateCount });
  if (failureStatus) {
    payload.status = failureStatus;
    payload.reason = failureStatus;
  }
  return payload;
}
