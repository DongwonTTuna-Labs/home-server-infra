import test from 'node:test';
import assert from 'node:assert/strict';

import {
  classifyReadinessSignals,
  hasProModelEvidence,
  modelBlockedByUpgrade,
} from '../lib/readiness.mjs';
import { statusPayloadFromSnapshot } from '../lib/commands/status.mjs';

test('ready Pro composer is not blocked by unrelated upgrade text', () => {
  const state = classifyReadinessSignals({
    url: 'https://chatgpt.com/',
    login: false,
    challenge: false,
    providerLimit: false,
    upgrade: true,
    pro: true,
    composer: true,
    send: false,
  });

  assert.equal(state.status, 'ready');
  assert.equal(state.reason, '');
});

test('non-Pro composer remains subscription_required', () => {
  const state = classifyReadinessSignals({
    url: 'https://chatgpt.com/',
    login: false,
    challenge: false,
    providerLimit: false,
    upgrade: true,
    pro: false,
    composer: true,
    send: false,
  });

  assert.equal(state.status, 'subscription_required');
  assert.equal(state.reason, 'auth.needs_pro');
});

test('model verification allows Upgrade text when selected control shows Pro Extended', () => {
  const proEvidence = hasProModelEvidence({
    selectedText: 'Pro Extended',
    haystack: 'Upgrade Team workspace',
  });

  assert.equal(proEvidence, true);
  assert.equal(modelBlockedByUpgrade({ upgrade: true, proEvidence }), false);
});

test('model verification blocks Upgrade text without Pro evidence', () => {
  const proEvidence = hasProModelEvidence({
    selectedText: 'Free',
    haystack: 'Upgrade to continue',
  });

  assert.equal(proEvidence, false);
  assert.equal(modelBlockedByUpgrade({ upgrade: true, proEvidence }), true);
});

test('status payload trusts captured diagnostics snapshot over stale fallback readiness', () => {
  const diagnostics = {
    label: 'status',
    dom: 'saved',
    screenshot: 'saved',
    url: 'https://chatgpt.com/',
    title: 'ChatGPT',
    readinessSignals: {
      login: false,
      limit: false,
      upgrade: true,
      pro: true,
      composer: true,
      stopControls: 0,
      dialogs: 0,
      fileInputs: 3,
      textboxes: 1,
    },
  };

  const payload = statusPayloadFromSnapshot({
    diagnostics,
    fallbackState: {
      status: 'subscription_required',
      reason: 'auth.needs_pro',
      composer: false,
      send: false,
      pro: false,
      upgrade: true,
    },
    pageUrl: 'https://chatgpt.com/',
  });

  assert.equal(payload.status, 'ready');
  assert.equal(payload.reason, undefined);
  assert.equal(payload.composer, true);
  assert.equal(payload.pro, true);
});

test('model evidence ignores body/menu haystack without selected composer Pro evidence', () => {
  const proEvidence = hasProModelEvidence({
    selectedText: '',
    haystack: 'Pro Extended',
  });

  assert.equal(proEvidence, false);
});
