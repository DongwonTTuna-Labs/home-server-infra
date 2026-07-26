import test from 'node:test';
import assert from 'node:assert/strict';

import { PROVIDER_SCHEMA, validateProviderEnvelope } from '../lib/schemas.mjs';

test('provider envelope schema accepts structured artifact identity', () => {
  const errors = validateProviderEnvelope({
    schema: PROVIDER_SCHEMA,
    ok: true,
    vendor: 'chatgpt',
    status: 'done',
    sessionId: 'sid',
    conversationUrl: 'https://chatgpt.com/c/sid',
    answerText: 'VERDICT: LGTM_NO_BLOCKING',
    artifacts: [{
      sessionId: 'sid',
      buttonText: 'pr72-review.zip',
      buttonTextSha256: 'a'.repeat(64),
      turnScope: 'current-assistant-turn',
      clickedElement: { role: 'button', tag: 'button' },
      artifact: { status: 'saved', hostPath: '/tmp/pr72-review.zip', sha256: 'b'.repeat(64), size: 123 },
    }],
    artifactCandidates: [],
  });

  assert.deepEqual(errors, []);
});

test('provider envelope schema rejects root-url sent success', () => {
  const errors = validateProviderEnvelope({
    schema: PROVIDER_SCHEMA,
    ok: true,
    vendor: 'chatgpt',
    status: 'sent',
    sessionId: 'sid',
    conversationUrl: 'https://chatgpt.com/',
  });

  assert.match(errors.join('\n'), /non-root/);
});

test('provider envelope schema rejects artifact candidates without visible button text', () => {
  const errors = validateProviderEnvelope({
    schema: PROVIDER_SCHEMA,
    ok: true,
    vendor: 'chatgpt',
    status: 'artifact.download_timeout',
    sessionId: 'sid',
    conversationUrl: 'https://chatgpt.com/c/sid',
    artifactCandidates: [{
      buttonText: '',
      buttonTextSha256: 'a'.repeat(64),
      clickedElement: {},
      artifact: { status: 'failed', reason: 'artifact.download_timeout' },
    }],
  });

  assert.match(errors.join('\n'), /buttonText/);
});

test('provider envelope schema validates artifact expectation taxonomy', () => {
  for (const artifactExpectation of ['none', 'optional', 'required', 'claimed']) {
    assert.deepEqual(validateProviderEnvelope({
      schema: PROVIDER_SCHEMA,
      ok: true,
      vendor: 'chatgpt',
      status: 'done',
      sessionId: 'sid',
      conversationUrl: 'https://chatgpt.com/c/sid',
      answerText: 'final answer',
      artifactExpectation,
    }), []);
  }

  assert.match(validateProviderEnvelope({
    schema: PROVIDER_SCHEMA,
    ok: true,
    vendor: 'chatgpt',
    status: 'done',
    sessionId: 'sid',
    conversationUrl: 'https://chatgpt.com/c/sid',
    answerText: 'final answer',
    artifactExpectation: 'sometimes',
  }).join('\n'), /artifactExpectation/);
});

test('provider envelope schema accepts bottom-scroll unverified statuses with session evidence', () => {
  for (const status of ['scroll.bottom_unverified', 'session.running_unverified']) {
    assert.deepEqual(validateProviderEnvelope({
      schema: PROVIDER_SCHEMA,
      ok: true,
      vendor: 'chatgpt',
      status,
      reason: 'scroll.bottom_unverified',
      sessionId: 'sid',
      conversationUrl: 'https://chatgpt.com/c/sid',
      diagnostics: [{ scrollBottomProof: { status: 'unverified' } }],
    }), []);
  }
});
