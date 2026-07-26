#!/usr/bin/env node
import process from 'node:process';

import {
  completeProviderResponse,
  derivePageBindingId,
  readCanonicalRequest,
  writeCanonicalStdout,
} from './lib/contracts/r13.mjs';
import {
  selectExistingPage,
  withBrowserR13,
} from './lib/browser-session.mjs';
import { writeR13OperationEvidence } from './lib/diagnostics/files.mjs';
import {
  captureRootState,
  observeBoundPage,
} from './lib/root-selector.mjs';
import {
  handleArtifactClickSave,
  handleArtifactDiscover,
} from './lib/artifact-download-r13.mjs';
import { handleCaptureRoot } from './lib/commands/capture.mjs';
import { handleClearUpload } from './lib/commands/clear-upload.mjs';
import { handleEnsureModel } from './lib/commands/ensure-model.mjs';
import { handlePoll } from './lib/commands/poll.mjs';
import { handleSendClick } from './lib/commands/send-click.mjs';
import { handleSendReconcile } from './lib/commands/send-reconcile.mjs';
import { handleStatus } from './lib/commands/status.mjs';
import { handleUploadOnly } from './lib/commands/upload-only.mjs';
import {
  handleSessionRebind,
  observeR13Session,
} from './lib/session-rebind.mjs';

const HANDLERS = Object.freeze({
  'artifact-click-save': handleArtifactClickSave,
  'artifact-discover': handleArtifactDiscover,
  'capture.root': handleCaptureRoot,
  'clear-upload': handleClearUpload,
  'ensure-model': handleEnsureModel,
  poll: handlePoll,
  'send-click': handleSendClick,
  'send-reconcile': handleSendReconcile,
  'session-rebind': handleSessionRebind,
  status: handleStatus,
  'upload-only': handleUploadOnly,
});

export async function dispatchR13Request(loaded, overrides = {}) {
  const { request, evidenceRoot } = loaded;
  const dependencies = {
    selectExistingPage,
    withBrowserR13,
    writeR13OperationEvidence,
    ...overrides,
  };
  return dependencies.withBrowserR13(async browser => {
    const sessionId = request.identity.sessionId
      ?? request.operationData.expected?.sessionId
      ?? request.operationData.expectation?.sessionId
      ?? '';
    const page = await dependencies.selectExistingPage(browser, sessionId);
    let evidenceCaptureIndex = 0;
    const captureEvidence = async () => {
      const refs = await dependencies.writeR13OperationEvidence(page, {
        captureIndex: evidenceCaptureIndex,
        evidenceRoot,
        request,
      });
      evidenceCaptureIndex += 1;
      return refs;
    };
    const evidenceRefs = await captureEvidence();
    const artifactsRoot = process.env.GPT_WEBAI_ARTIFACTS_DIR || evidenceRoot;
    const expectedSession = request.operationData.expected
      ?? request.operationData.expectation
      ?? null;
    const pageGeneration = request.operation === 'session-rebind'
      ? request.operationData.expectation.lastKnownPageBindingGeneration + 1
      : expectedSession?.pageBindingGeneration;
    const context = {
      artifactsRoot,
      browser,
      evidenceRefs,
      evidenceRoot,
      page,
      request,
      captureModelState: async () => {
        const state = await captureRootState(page);
        const expected = request.operationData.pageBinding;
        return {
          effortControl: state.effortControl,
          effortLabel: state.effortLabel,
          modelControl: state.modelControl,
          modelLabel: state.modelLabel,
          pageBinding: pageBindingFromState(expected, state),
        };
      },
      captureEvidence,
      observePageBinding: () => observeBoundPage(
        page,
        request.operationData.pageBinding,
      ),
      observeSession: (observedSessionId = sessionId) => observeR13Session(page, {
        expected: expectedSession,
        pageBindingGeneration: pageGeneration,
        sessionId: observedSessionId,
      }),
    };
    const handler = (overrides.handlers ?? HANDLERS)[request.operation];
    const result = await handler(context, overrides.handlerOverrides?.[request.operation]);
    return completeProviderResponse({
      request,
      evidenceRoot,
      ok: result.ok,
      status: result.status,
      providerReason: result.providerReason,
      operationData: result.operationData,
    });
  });
}

export async function runR13Cli(argv = process.argv.slice(2), overrides = {}) {
  if (argv.length !== 2 || argv[0] !== '--request-file' || !argv[1]) {
    throw new ProviderUsageError(
      'usage: cli.mjs --request-file <container-safe-absolute-path>',
    );
  }
  const loaded = await (overrides.readCanonicalRequest ?? readCanonicalRequest)(argv[1]);
  const response = await dispatchR13Request(loaded, overrides);
  (overrides.writeCanonicalStdout ?? writeCanonicalStdout)(response);
  return response;
}

export class ProviderUsageError extends Error {
  constructor(message) {
    super(message);
    this.name = 'ProviderUsageError';
  }
}

function pageBindingFromState(expected, state) {
  return {
    ...expected,
    bindingId: derivePageBindingId(state.pageIncarnationId, state.rootBindingHash),
    browserContextId: state.browserContextId,
    domMutationGeneration: state.domMutationGeneration,
    pageIncarnationId: state.pageIncarnationId,
    rootBindingHash: state.rootBindingHash,
    targetId: state.targetId,
  };
}

function exitAfterStdoutFlush(code) {
  process.stdout.write('', () => process.exit(code));
}

if (import.meta.url === `file://${process.argv[1]}`) {
  runR13Cli()
    .then(() => exitAfterStdoutFlush(0))
    .catch(error => {
      process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
      exitAfterStdoutFlush(error instanceof ProviderUsageError ? 2 : 70);
    });
}
