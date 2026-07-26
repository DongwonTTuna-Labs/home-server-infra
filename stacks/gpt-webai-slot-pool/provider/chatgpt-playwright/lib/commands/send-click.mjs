import { constants as fsConstants } from 'node:fs';
import { open } from 'node:fs/promises';
import path from 'node:path';

import { clickSend, fillPrompt, readPromptComposer } from '../browser.mjs';
import {
  canonicalSha256,
  writeOperationReceipt,
} from '../contracts/r13.mjs';
import { waitForR13SendStartConfirmation } from '../send-confirmation.mjs';
import { r13TurnObservations } from '../turns.mjs';
import { hitFailpoint, sha256Text } from '../common.mjs';

const PROMPT_ROOT = '/broker-prompts';
const SEND_CONFIRMATION_MS = 30_000;

export async function handleSendClick(context, overrides = {}) {
  const { request, evidenceRoot, page, evidenceRefs, observePageBinding } = context;
  const dependencies = {
    clickSend,
    fillPrompt,
    readPromptComposer,
    readPromptInput,
    r13TurnObservations,
    waitForR13SendStartConfirmation,
    writeOperationReceipt,
    ...overrides,
  };
  const captureEvidence = context.captureEvidence ?? (async () => evidenceRefs);
  const { pageBinding, promptInput, sendAttemptId } = request.operationData;
  const prompt = await dependencies.readPromptInput(promptInput);
  const baseline = await dependencies.r13TurnObservations(page);
  await dependencies.fillPrompt(page, prompt.toString('utf8'));
  const composerText = await dependencies.readPromptComposer(page);
  if (`sha256:${sha256Text(composerText)}` !== promptInput.sha256) {
    throw new Error('contract.invalid_provider_envelope: composer digest mismatch');
  }

  const preClickReceipt = sendReceipt({
    kind: 'pre_click',
    pageBinding,
    promptSha256: promptInput.sha256,
    sendAttemptId,
    evidenceRefs,
  });
  await dependencies.writeOperationReceipt({
    request,
    evidenceRoot,
    relPath: request.evidence.receiptRelPaths.preClick,
    operation: 'send.pre_click',
    payload: preClickReceipt,
  });

  const observedPageBinding = await observePageBinding();
  if (canonicalSha256(observedPageBinding) !== canonicalSha256(pageBinding)) {
    return sendFailure('send.turn_not_proven', preClickReceipt, observedPageBinding);
  }
  try {
    await dependencies.clickSend(page);
    hitFailpoint('after-physical-send-click-before-provider-stdout');
  } catch {
    await captureEvidence();
    return sendFailure('send.click_timeout', preClickReceipt, observedPageBinding);
  }

  const confirmation = await dependencies.waitForR13SendStartConfirmation(
    page,
    baseline,
    Math.min(request.deadlineMs, SEND_CONFIRMATION_MS),
  );
  const postClickEvidenceRefs = await captureEvidence();
  if (!confirmation.confirmed) {
    return sendFailure('send.turn_not_proven', preClickReceipt, observedPageBinding);
  }
  const terminalSendReceipt = sendReceipt({
    kind: 'post_click',
    pageBinding,
    promptSha256: promptInput.sha256,
    sendAttemptId,
    evidenceRefs: postClickEvidenceRefs,
    confirmation,
  });
  await dependencies.writeOperationReceipt({
    request,
    evidenceRoot,
    relPath: request.evidence.receiptRelPaths.postClick,
    operation: 'send.post_click',
    payload: terminalSendReceipt,
  });
  return {
    ok: true,
    status: 'done',
    providerReason: null,
    operationData: { preClickReceipt, terminalSendReceipt, observedPageBinding },
  };
}

export async function readPromptInput(input, promptRoot = PROMPT_ROOT) {
  const root = path.resolve(promptRoot);
  const target = path.resolve(root, input.containerRelPath);
  if (target === root || !target.startsWith(`${root}${path.sep}`)) {
    throw new Error('contract.invalid_provider_envelope: prompt path');
  }
  const handle = await open(
    target,
    fsConstants.O_RDONLY | fsConstants.O_CLOEXEC | fsConstants.O_NOFOLLOW,
  );
  try {
    const info = await handle.stat();
    if (!info.isFile() || info.nlink !== 1 || info.size !== input.sizeBytes) {
      throw new Error('contract.invalid_provider_envelope: prompt metadata');
    }
    const bytes = await handle.readFile();
    if (`sha256:${sha256Text(bytes)}` !== input.sha256) {
      throw new Error('contract.invalid_provider_envelope: prompt digest');
    }
    return bytes;
  } finally {
    await handle.close();
  }
}

function sendReceipt({
  kind,
  pageBinding,
  promptSha256,
  sendAttemptId,
  evidenceRefs,
  confirmation = null,
}) {
  return {
    assistantTurnId: confirmation?.assistantTurnId ?? null,
    capturedAtMs: Date.now(),
    conversationUrl: confirmation?.conversationUrl ?? null,
    evidenceRefs,
    kind,
    pageBinding,
    physicalClickCount: kind === 'post_click' ? 1 : 0,
    promptSha256,
    sendAttemptId,
    sessionId: confirmation?.sessionId ?? null,
    userTurnId: confirmation?.userTurnId ?? null,
  };
}

function sendFailure(providerReason, preClickReceipt, observedPageBinding) {
  return {
    ok: false,
    status: 'failed',
    providerReason,
    operationData: {
      preClickReceipt,
      terminalSendReceipt: null,
      observedPageBinding,
    },
  };
}
