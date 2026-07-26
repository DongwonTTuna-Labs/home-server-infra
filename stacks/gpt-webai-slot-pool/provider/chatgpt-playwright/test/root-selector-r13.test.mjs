import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import test from 'node:test';

import {
  RootSelectorError,
  loadRootSelectorLabelFixtureSets,
  rootBindingHash,
  selectRootBindingCandidates,
  structuralIdentity,
} from '../lib/root-selector.mjs';

const h256 = value => `sha256:${createHash('sha256').update(value).digest('hex')}`;

function identity(name, role = null, x = 0) {
  return {
    ariaLabelHash: role === null ? null : h256(name),
    boundingBox: { height: 40, width: 100, x, y: 100 },
    domPath: [['html', 0], ['body', 0], [name, x]],
    role,
    tagName: role === null ? 'div' : 'button',
    testIdHash: null,
  };
}

function conversation(name, overrides = {}) {
  return {
    containsTurnList: 1,
    excludesSidebar: 1,
    hiddenPenalty: 0,
    identity: identity(name),
    roleMain: 1,
    viewportWidthCoverageBucket: 10,
    visible: 1,
    ...overrides,
  };
}

function composer(name, overrides = {}) {
  return {
    containsTextareaOrContenteditable: 1,
    fixedBottomOrForm: 1,
    historySidebarAncestorPenalty: 0,
    identity: identity(name),
    uploadControlNearby: 1,
    visible: 1,
    ...overrides,
  };
}

function model(name, x = 0, overrides = {}) {
  return {
    ariaHasPopupOrButton: 1,
    disabledPenalty: 0,
    identity: identity(name, 'button', x),
    insideComposerOrHeader: 1,
    labelHashMatchesModelControl: 1,
    visible: 1,
    ...overrides,
  };
}

function effort(name, x = 0, overrides = {}) {
  return {
    disabledPenalty: 0,
    identity: identity(name, 'button', x),
    labelHashMatchesEffortOrStandard: 1,
    modelMenuAssociation: 1,
    visible: 1,
    ...overrides,
  };
}

function input() {
  return {
    composerRoots: [composer('composer'), composer('composer-runner', { uploadControlNearby: 0 })],
    conversationRoots: [conversation('conversation'), conversation('conversation-runner', { containsTurnList: 0 })],
    domMutationGeneration: 7,
    effortControls: [effort('effort'), effort('effort-runner', 300, { modelMenuAssociation: 0 })],
    modelControls: [model('model'), model('model-runner', 300, { labelHashMatchesModelControl: 0 })],
    normalizedUrl: 'https://chatgpt.com/',
  };
}

test('R13 selector applies all four exact score formulas and the minimum real margin', () => {
  const selected = selectRootBindingCandidates(input());
  assert.match(selected.conversationRootId, /^root_[0-9a-f]{64}$/);
  assert.match(selected.composerRootId, /^root_[0-9a-f]{64}$/);
  assert.match(selected.modelControl.controlId, /^control_[0-9a-f]{64}$/);
  assert.match(selected.effortControl.controlId, /^control_[0-9a-f]{64}$/);
  assert.equal(selected.selectorMargin, 100);
  assert.equal(selected.modelControl.labelHash, h256('model'));
  assert.match(selected.rootBindingHash, /^sha256:[0-9a-f]{64}$/);
});

test('identity and binding hashes use only the canonical structural tuples', () => {
  const value = identity('model', 'button');
  const first = structuralIdentity(value, 'control');
  const second = structuralIdentity(structuredClone(value), 'control');
  assert.deepEqual(second, first);
  assert.equal(rootBindingHash({
    composerRootId: `root_${'1'.repeat(64)}`,
    conversationRootId: `root_${'2'.repeat(64)}`,
    domMutationGeneration: 1,
    effortControlId: `control_${'3'.repeat(64)}`,
    modelControlId: `control_${'4'.repeat(64)}`,
    normalizedUrl: 'https://chatgpt.com/',
  }), `sha256:1abda54ce183612b3451c8103fc2904f7070160f4104d16f3cd63bc3dcdb7a5d`);
});

test('low margin, duplicate sanitized identity, and content fields fail closed', () => {
  const lowMargin = input();
  lowMargin.modelControls[1].labelHashMatchesModelControl = 1;
  assert.throws(
    () => selectRootBindingCandidates(lowMargin),
    error => error instanceof RootSelectorError && error.reason === 'capture.ambiguous',
  );

  const duplicate = input();
  duplicate.effortControls[1].identity.domPath = structuredClone(duplicate.effortControls[0].identity.domPath);
  assert.throws(() => selectRootBindingCandidates(duplicate), /duplicateDomPathHash/);

  const privateContent = input();
  privateContent.conversationRoots[0].promptText = 'forbidden';
  assert.throws(
    () => selectRootBindingCandidates(privateContent),
    error => error instanceof RootSelectorError && error.reason === 'provider.schema_drift',
  );
});

test('model and effort tie breaks use floored viewport distance before dom path hash', () => {
  const value = input();
  value.modelControls = [
    model('far', 700),
    model('near', 10),
  ];
  value.modelControls[0].insideComposerOrHeader = 0;
  value.modelControls[1].insideComposerOrHeader = 1;
  const selected = selectRootBindingCandidates(value);
  assert.equal(selected.modelControl.labelHash, h256('near'));
});

test('production root scoring consumes only the pinned normalized model and effort labels', async () => {
  const labels = await loadRootSelectorLabelFixtureSets();
  assert.deepEqual(labels.model, ['pro', 'thinking']);
  assert.deepEqual(labels.effort, ['heavy', 'standard']);
  assert.equal(labels.model.includes('pro extended'), false);
});
