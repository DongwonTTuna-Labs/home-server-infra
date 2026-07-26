import assert from 'node:assert/strict';
import test from 'node:test';

import { terminalProviderLimitPayload } from '../lib/commands/poll.mjs';
import { downloadPayload } from '../lib/commands/download.mjs';
import { sessionPayload, terminalSessionSuccess } from '../lib/commands/session.mjs';
import { applyProviderLimitStatus, hasProviderLimitDiagnostics } from '../lib/provider-limit.mjs';

test('detects provider-limit modal evidence from terminal diagnostics', () => {
  const diagnostics = [{
    label: 'poll-terminal-before-artifacts',
    readinessSignals: {
      limit: true,
      pro: true,
      composer: true,
    },
    dialogs: [{
      textPreview: 'Too many requests. Please wait a few minutes before trying again.',
    }],
  }];

  assert.equal(hasProviderLimitDiagnostics(diagnostics), true);
});

test('ignores provider-limit words in body text when no blocking surface is visible', () => {
  const diagnostics = [{
    label: 'poll-terminal-before-artifacts',
    readinessSignals: {
      limit: true,
      pro: true,
      composer: true,
      dialogs: 0,
    },
    bodyTextPreview: 'The assistant is discussing a provider limit bug and too many requests text.',
    dialogs: [],
    providerLimitSurfaces: [],
  }];

  assert.equal(hasProviderLimitDiagnostics(diagnostics), false);
});

test('does not classify a clean terminal answer as provider-limited', () => {
  const diagnostics = [{
    label: 'poll-terminal-before-artifacts',
    readinessSignals: {
      limit: false,
      pro: true,
      composer: true,
    },
    bodyTextPreview: 'Final answer text',
  }];

  assert.equal(hasProviderLimitDiagnostics(diagnostics), false);
});

test('terminal poll payload preserves answer evidence without reporting done when limited', () => {
  const payload = terminalProviderLimitPayload({
    sessionId: 'sid-limit',
    url: 'https://chatgpt.com/c/sid-limit',
    state: {
      answerText: 'visible answer text',
      assistantTurn: {
        textSha256: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
      },
      turnEvidence: {
        activeTurn: false,
      },
    },
    diagnostics: [{
      label: 'poll-terminal-before-artifacts',
      readinessSignals: {
        limit: true,
      },
      dialogs: [{
        textPreview: 'Too many requests. Please wait a few minutes before trying again.',
      }],
    }],
  });

  assert.equal(payload.status, 'provider_limit');
  assert.equal(payload.reason, 'provider.limit');
  assert.equal(payload.answerText, 'visible answer text');
});

test('ignores provider-limit phrases outside scoped blocking surfaces', () => {
  const diagnostics = [{
    label: 'poll-terminal-before-artifacts',
    bodyTextPreview: 'Prompt says too many requests and rate limit; assistant explains the false positive.',
    assistantTurns: [{ textPreview: 'A provider limit phrase in the final answer is not a modal.' }],
    sidebarHistoryPreview: 'History item: too many requests issue',
    composerPreview: 'composer text: request limit',
    attachmentFilenames: ['too-many-requests-notes.txt'],
    dialogs: [],
    providerLimitSurfaces: [],
  }];

  assert.equal(hasProviderLimitDiagnostics(diagnostics), false);
});

test('provider-limit diagnostics require scoped surface text rather than readiness body flags', () => {
  const diagnostics = [{
    label: 'poll-terminal-before-artifacts',
    readinessSignals: { limit: true, providerLimitSurfaceCount: 0 },
    bodyTextPreview: 'Too many requests appears in broad body text only.',
    dialogs: [],
    providerLimitSurfaces: [],
  }];

  assert.equal(terminalProviderLimitPayload({
    sessionId: 'sid-clean',
    url: 'https://chatgpt.com/c/sid-clean',
    state: { answerText: 'Complete terminal answer' },
    diagnostics,
  }), null);
});


test('session terminal success is rewritten to provider-limit when scoped blocking surface is visible', () => {
  const diagnostics = [{
    label: 'sessions-resume-before-terminal',
    providerLimitSurfaces: [{ textPreview: 'You have reached the message cap. Try again later.' }],
  }];
  const payload = sessionPayload({
    action: 'resume',
    sid: 'sid-limit',
    url: 'https://chatgpt.com/c/sid-limit',
    validSessionUrl: true,
    contentUnavailable: false,
    hasFinalVisibleAnswer: true,
    hasRunningAnswer: false,
    state: {
      answerText: 'visible final answer evidence',
      assistantTurn: { turnIndex: 2, textSha256: 'b'.repeat(64) },
      turnEvidence: { activeTurn: false },
    },
    diagnostics,
  });

  assert.equal(payload.status, 'done');
  assert.equal(terminalSessionSuccess(payload.status), true);
  assert.equal(applyProviderLimitStatus(payload, diagnostics), true);
  assert.equal(payload.status, 'provider_limit');
  assert.equal(payload.reason, 'provider.limit');
  assert.equal(payload.answerText, 'visible final answer evidence');
});

test('session show success is rewritten to provider-limit when scoped blocking surface is visible', () => {
  const diagnostics = [{
    label: 'sessions-show-before-terminal',
    providerLimitSurfaces: [{ textPreview: 'Too many requests. Please wait before trying again.' }],
  }];
  const payload = sessionPayload({
    action: 'show',
    sid: 'sid-show-limit',
    url: 'https://chatgpt.com/c/sid-show-limit',
    validSessionUrl: true,
    contentUnavailable: false,
    hasFinalVisibleAnswer: false,
    hasRunningAnswer: false,
    state: { reason: undefined },
    diagnostics,
  });

  assert.equal(payload.status, 'show');
  assert.equal(terminalSessionSuccess(payload.status), true);
  assert.equal(applyProviderLimitStatus(payload, diagnostics), true);
  assert.equal(payload.status, 'provider_limit');
  assert.equal(payload.reason, 'provider.limit');
});

test('download terminal done is rewritten to provider-limit while preserving artifact evidence', () => {
  const artifacts = [{
    sessionId: 'sid-download-limit',
    buttonText: 'source.zip',
    buttonTextSha256: 'c'.repeat(64),
    clickedElement: { tag: 'button' },
    artifact: { status: 'saved', savedPath: '/safe/source.zip' },
  }];
  const diagnostics = [{
    label: 'download-after-click',
    providerLimitSurfaces: [{ textPreview: 'Rate limit reached. Try again later.' }],
  }];
  const payload = downloadPayload({
    status: 'done',
    reason: undefined,
    sessionId: 'sid-download-limit',
    conversationUrl: 'https://chatgpt.com/c/sid-download-limit',
    state: { assistantTurn: { turnIndex: 3 }, turnEvidence: { activeTurn: false } },
    artifactExpectation: 'required',
    artifacts,
    artifactCandidates: artifacts,
    warnings: [],
    downloadCandidateCount: 1,
    bottomScroll: { status: 'at_bottom' },
    diagnostics,
  });

  assert.equal(applyProviderLimitStatus(payload, diagnostics), true);
  assert.equal(payload.status, 'provider_limit');
  assert.equal(payload.reason, 'provider.limit');
  assert.equal(payload.artifacts, artifacts);
  assert.equal(payload.diagnostics, diagnostics);
});
