import assert from 'node:assert/strict';
import test from 'node:test';

import { sanitizeDomDiagnostics } from '../lib/diagnostics/sanitize.mjs';
import { hasProviderLimitDiagnostics } from '../lib/provider-limit.mjs';
import { selectRootBindingCandidates } from '../lib/root-selector.mjs';

const SECRET = 'private prompt answer account@example.test Too many requests';
const h256 = character => `sha256:${character.repeat(64)}`;

test('R13 DOM evidence keeps only hashes, lengths, structure, and provider-state classification', () => {
  const sanitized = sanitizeDomDiagnostics({
    assistantTurns: [{ index: 0, tag: 'article', domId: 'assistant', text: SECRET, rect: {} }],
    bodyTextPreview: SECRET,
    controls: [{ index: 0, tag: 'button', role: 'button', text: SECRET, label: SECRET, title: SECRET, rect: {}, disabled: false }],
    dialogs: [{ index: 0, tag: 'div', role: 'dialog', className: SECRET, text: SECRET, rect: {} }],
    providerLimitSurfaces: [{
      index: 0, tag: 'div', role: 'alert', kind: 'alert', className: SECRET,
      text: SECRET, rect: {}, actionButtons: [{ index: 0, tag: 'button', role: 'button', text: SECRET, label: SECRET, rect: {} }],
    }],
    userTurns: [{ index: 0, tag: 'article', domId: 'user', text: SECRET, rect: {} }],
  }, {});
  const bytes = JSON.stringify(sanitized);
  assert.doesNotMatch(bytes, /private prompt|account@example|Too many requests/);
  assert.equal(sanitized.assistantTurns[0].textLength, SECRET.length);
  assert.match(sanitized.assistantTurns[0].textSha256, /^[0-9a-f]{64}$/);
  assert.equal(sanitized.providerLimitSurfaces[0].providerLimitMatched, true);
  assert.equal(hasProviderLimitDiagnostics([sanitized]), true);
});

test('R13 structural selector rejects content-bearing candidate fields', () => {
  const identity = (name, role = null) => ({
    ariaLabelHash: role === null ? null : h256('a'),
    boundingBox: { height: 10, width: 10, x: 0, y: 0 },
    domPath: [['html', 0], [name, 0]],
    role,
    tagName: role === null ? 'div' : 'button',
    testIdHash: null,
  });
  const candidate = {
    containsTurnList: 1,
    excludesSidebar: 1,
    hiddenPenalty: 0,
    identity: identity('conversation'),
    promptText: SECRET,
    roleMain: 1,
    viewportWidthCoverageBucket: 10,
    visible: 1,
  };
  assert.throws(() => selectRootBindingCandidates({
    composerRoots: [],
    conversationRoots: [candidate],
    domMutationGeneration: 0,
    effortControls: [],
    modelControls: [],
    normalizedUrl: 'https://chatgpt.com/',
  }), /provider.schema_drift/);
});
