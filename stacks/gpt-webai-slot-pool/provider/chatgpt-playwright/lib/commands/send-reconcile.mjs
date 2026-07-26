import { canonicalSha256, writeOperationReceipt } from '../contracts/r13.mjs';
import { reconcileR13TurnStart } from '../send-confirmation.mjs';

const SEND_CONFIRMATION_MS = 30_000;

export async function handleSendReconcile(context, overrides = {}) {
  const { request, evidenceRoot, page, evidenceRefs, observePageBinding } = context;
  const dependencies = { reconcileR13TurnStart, writeOperationReceipt, ...overrides };
  const { pageBinding, preClickReceipt, sendAttemptId } = request.operationData;
  const observedPageBinding = await observePageBinding();
  if (canonicalSha256(observedPageBinding) !== canonicalSha256(pageBinding)) {
    return {
      ok: false,
      status: 'failed',
      providerReason: 'send.turn_not_proven',
      operationData: {
        preClickReceipt,
        terminalSendReceipt: null,
        observedPageBinding,
      },
    };
  }
  const confirmation = await dependencies.reconcileR13TurnStart(
    page,
    preClickReceipt.promptSha256,
    Math.min(request.deadlineMs, SEND_CONFIRMATION_MS),
  );
  if (!confirmation.confirmed) {
    return {
      ok: false,
      status: 'failed',
      providerReason: 'send.turn_not_proven',
      operationData: {
        preClickReceipt,
        terminalSendReceipt: null,
        observedPageBinding,
      },
    };
  }
  const terminalSendReceipt = {
    assistantTurnId: confirmation.assistantTurnId,
    capturedAtMs: Date.now(),
    conversationUrl: confirmation.conversationUrl,
    evidenceRefs,
    kind: 'reconciled_turn_start',
    pageBinding,
    physicalClickCount: 0,
    promptSha256: preClickReceipt.promptSha256,
    sendAttemptId,
    sessionId: confirmation.sessionId,
    userTurnId: confirmation.userTurnId,
  };
  await dependencies.writeOperationReceipt({
    request,
    evidenceRoot,
    relPath: request.evidence.receiptRelPaths.reconcile,
    operation: 'send.reconcile',
    payload: terminalSendReceipt,
  });
  return {
    ok: true,
    status: 'done',
    providerReason: null,
    operationData: { preClickReceipt, terminalSendReceipt, observedPageBinding },
  };
}
