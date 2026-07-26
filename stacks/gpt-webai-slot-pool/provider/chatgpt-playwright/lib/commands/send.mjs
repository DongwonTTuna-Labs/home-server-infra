import { readFile } from 'node:fs/promises';

import {
  conversationIdFromUrl,
  jsonOut,
  valueAfter,
  valuesAfter,
} from '../common.mjs';
import {
  classifyReadiness,
  clickSend,
  fillPrompt,
  selectFreshPage,
  setFiles,
  prepareRequestedModel,
  waitForAttachmentEvidence,
  withBrowser,
} from '../browser.mjs';
import { waitForR13SendStartConfirmation } from '../send-confirmation.mjs';
import { r13TurnObservations } from '../turns.mjs';
import {
  captureDiagnostics,
  targetIdForUrl,
  writeModelSelectionEvidence,
  writeDurableSendStartEvidence,
} from './shared.mjs';

async function promptFromArgs(args) {
  const promptFile = valueAfter(args, '--prompt-file');
  if (promptFile) {
    return await readFile(promptFile, 'utf8');
  }
  return valueAfter(args, '--prompt');
}

export async function commandSend(args) {
  const prompt = await promptFromArgs(args);
  if (!prompt) {
    jsonOut({ ok: true, vendor: 'chatgpt', status: 'provider.schema_drift', reason: 'provider.schema_drift', message: 'missing prompt' });
    process.exit(2);
  }
  const model = valueAfter(args, '--model') || 'pro';
  const effort = valueAfter(args, '--effort') || 'extended';
  const files = valuesAfter(args, '--file');
  await withBrowser(async browser => {
    const page = await selectFreshPage(browser);
    const diagnostics = [];
    let readiness = await classifyReadiness(page);
    if (readiness.status !== 'ready' && readiness.status !== 'login_required' && readiness.status !== 'provider_limit' && readiness.status !== 'subscription_required') {
      // Late hydration frequently misclassifies a healthy page; retry once.
      await page.waitForTimeout(5000);
      readiness = await classifyReadiness(page);
    }
    if (readiness.status === 'login_required' || readiness.status === 'provider_limit' || readiness.status === 'subscription_required') {
      diagnostics.push(await captureDiagnostics(page, 'send-readiness-blocked', ''));
      jsonOut({ ok: true, vendor: 'chatgpt', status: readiness.status, reason: readiness.reason, url: page.url(), diagnostics });
      return;
    }
    if (readiness.status !== 'ready') {
      diagnostics.push(await captureDiagnostics(page, 'send-readiness-unreachable', ''));
      jsonOut({ ok: true, vendor: 'chatgpt', status: 'unreachable', reason: readiness.reason || 'provider.schema_drift', url: page.url(), diagnostics });
      process.exitCode = 70;
      return;
    }
    const modelEvidence = await prepareRequestedModel(page, model, effort);
    if (!modelEvidence.ok) {
      await writeModelSelectionEvidence(modelEvidence);
      diagnostics.push(await captureDiagnostics(page, 'send-model-blocked', ''));
      jsonOut({ ok: true, vendor: 'chatgpt', status: modelEvidence.status, reason: modelEvidence.reason, url: page.url(), model, effort, modelEvidence, diagnostics });
      return;
    }
    diagnostics.push(await captureDiagnostics(page, 'send-before-attachments', ''));
    const baseline = await r13TurnObservations(page);
    await setFiles(page, files);
    const attachmentEvidence = await waitForAttachmentEvidence(page, files);
    if (files.length > 0) {
      diagnostics.push(await captureDiagnostics(page, 'send-after-attachments-before-prompt', ''));
    }
    if (!attachmentEvidence.ok) {
      jsonOut({
        ok: true,
        vendor: 'chatgpt',
        status: 'attachment_unavailable',
        reason: 'provider.attachment_unavailable',
        url: page.url(),
        attachmentEvidence,
        diagnostics,
      });
      return;
    }
    await fillPrompt(page, prompt);
    await clickSend(page);
    const confirmation = await waitForR13SendStartConfirmation(page, baseline, 120_000);
    const conversationUrl = confirmation.conversationUrl;
    const sessionId = confirmation.sessionId || conversationIdFromUrl(conversationUrl);
    const startEvidence = {
      activeTurn: Boolean(confirmation.confirmed),
      assistantTurnId: confirmation.assistantTurnId,
      userTurnId: confirmation.userTurnId,
    };
    const targetId = targetIdForUrl(conversationUrl);
    const durableSendStart = await writeDurableSendStartEvidence({
      ok: Boolean(sessionId && confirmation.confirmed),
      status: sessionId && confirmation.confirmed ? 'sent' : 'session.start_unconfirmed',
      model,
      effort,
      sessionId,
      targetId,
      conversationUrl,
      turnEvidence: startEvidence,
    });
    diagnostics.push(await captureDiagnostics(page, 'send-after-start-confirmation', sessionId || ''));
    if (!sessionId || !confirmation.confirmed) {
      jsonOut({
        ok: true,
        vendor: 'chatgpt',
        status: 'session.start_unconfirmed',
        reason: 'session.start_unconfirmed',
        url: conversationUrl,
        baseline,
        turnEvidence: startEvidence,
        durableSendStart,
        diagnostics,
      });
      return;
    }
    jsonOut({
      ok: true,
      vendor: 'chatgpt',
      status: 'sent',
      model,
      effort,
      sessionId,
      targetId,
      conversationUrl,
      baseline: { capturedAt: new Date().toISOString(), turnCount: baseline.length },
      turnEvidence: startEvidence,
      attachmentEvidence,
      modelEvidence,
      durableSendStart,
      diagnostics,
    });
  });
}
