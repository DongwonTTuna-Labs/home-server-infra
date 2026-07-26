import {
  jsonOut,
  valueAfter,
} from '../common.mjs';
import {
  selectPage,
  withBrowser,
} from '../browser.mjs';
import { captureRootState, RootSelectorError } from '../root-selector.mjs';
import { captureDiagnostics } from './shared.mjs';

export async function commandCapture(args) {
  const sid = valueAfter(args, '--session') || '';
  const label = valueAfter(args, '--label') || (sid ? 'session-capture' : 'root-capture');
  await withBrowser(async browser => {
    const page = await selectPage(browser, sid);
    const diagnostics = await captureDiagnostics(page, label, sid);
    jsonOut({
      ok: true,
      vendor: 'chatgpt',
      status: diagnostics.dom === 'saved' || diagnostics.screenshot === 'saved' ? 'captured' : 'capture_failed',
      sessionId: sid || undefined,
      conversationUrl: page.url(),
      diagnostics,
    });
  });
}

export async function handleCaptureRoot(context, overrides = {}) {
  const { request, page, evidenceRefs } = context;
  const dependencies = { captureRootState, ...overrides };
  try {
    const state = await dependencies.captureRootState(page);
    return {
      ok: true,
      status: 'done',
      providerReason: null,
      operationData: {
        failureProof: null,
        rootBindingCandidate: {
          browserContextId: state.browserContextId,
          capturedAtMs: Date.now(),
          composerRootId: state.composerRootId,
          conversationRootId: state.conversationRootId,
          domMutationGeneration: state.domMutationGeneration,
          effortControl: state.effortControl,
          evidenceRefs,
          modelControl: state.modelControl,
          normalizedUrl: state.normalizedUrl,
          operationId: request.identity.operationId,
          pageIncarnationId: state.pageIncarnationId,
          selectorMargin: state.selectorMargin,
          targetId: state.targetId,
        },
      },
    };
  } catch (error) {
    const ambiguous = error instanceof RootSelectorError && error.reason === 'capture.ambiguous';
    return {
      ok: false,
      status: 'failed',
      providerReason: ambiguous ? 'capture.ambiguous' : 'capture.timeout',
      operationData: {
        failureProof: ambiguous ? {
          controlIdentityStable: false,
          evidenceRefs,
          failedAtMs: Date.now(),
          pickerOpened: false,
          reason: 'capture.ambiguous',
          requestedEffortVisible: false,
          requestedModelVisible: false,
        } : null,
        rootBindingCandidate: null,
      },
    };
  }
}
