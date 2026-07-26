import assert from 'node:assert/strict';
import test from 'node:test';

import {
  handleEnsureModel,
  loadModelEffortLabels,
  normalizeVisibleLabel,
} from '../lib/commands/ensure-model.mjs';

const typed = (prefix, character) => `${prefix}_${character.repeat(64)}`;
const h256 = character => `sha256:${character.repeat(64)}`;

function binding() {
  return {
    bindingId: typed('binding', '1'), bindingGeneration: 1,
    browserContextId: typed('ctx', '2'), cohort: 'cohort-a', domMutationGeneration: 0,
    leaseGeneration: 1, leaseId: typed('lease', '3'),
    pageIncarnationId: typed('page', '4'), rootBindingHash: h256('5'),
    runtimeIncarnationId: typed('runtime', '6'), runtimeOwnerGeneration: 1,
    runtimeOwnerId: typed('owner', '7'), slotId: 'slot-01', targetId: typed('target', '8'),
  };
}

function control(character) {
  return {
    boundingBoxHash: h256(character), controlId: typed('control', character), disabled: false,
    domPathHash: h256(character), labelHash: h256(character), role: 'button',
    testIdHash: null, visible: true,
  };
}

function request() {
  return {
    operationData: {
      pageBinding: binding(), pickerOpenBudget: 1, requestedEffort: 'standard',
      requestedModel: 'pro', stabilizationMs: 500,
    },
  };
}

function state(modelLabel, effortLabel) {
  return {
    pageBinding: binding(), modelLabel, effortLabel,
    modelControl: control('a'), effortControl: control('b'),
  };
}

const evidenceRefs = [{
  path: 'dom.sanitized.json', sha256: h256('c'), sizeBytes: 1, mediaType: 'application/json',
}];

test('the checked-in 93-byte label fixture drives exact normalized tuple matching', async () => {
  const labels = await loadModelEffortLabels();
  assert.equal(labels.model.get('pro'), 'Pro');
  assert.equal(labels.model.get('xhigh'), 'Thinking');
  assert.equal(labels.effort.get('standard'), 'Standard');
  assert.equal(labels.effort.get('high'), 'Heavy');
  assert.equal(normalizeVisibleLabel('  PRO\u00a0'), 'pro');
});

test('already exact Pro and Standard returns two proofs without opening the picker', async () => {
  let pickerCalls = 0;
  const result = await handleEnsureModel({
    request: request(), page: {}, evidenceRefs, captureModelState: async () => state('Pro', 'Standard'),
  }, {
    selectModelTuple: async () => { pickerCalls += 1; },
  });
  assert.equal(pickerCalls, 0);
  assert.equal(result.ok, true);
  assert.equal(result.operationData.modelProof.selectedBy, 'already_exact');
  assert.equal(result.operationData.effortProof.observed, 'standard');
});

test('picker correction is bounded to one attempt and requires post-selection exact proof', async () => {
  const observations = [state('Instant', 'Standard'), state('Pro', 'Standard')];
  let pickerCalls = 0;
  let waitMs = 0;
  let evidenceCaptures = 0;
  const result = await handleEnsureModel({
    request: request(), page: { waitForTimeout: async value => { waitMs = value; } }, evidenceRefs,
    captureModelState: async () => observations.shift(),
    captureEvidence: async () => {
      evidenceCaptures += 1;
      return [{ ...evidenceRefs[0], path: `post-selection-${evidenceCaptures}.json` }];
    },
  }, {
    selectModelTuple: async (_page, tuple) => {
      pickerCalls += 1;
      assert.deepEqual(tuple, { modelLabel: 'Pro', effortLabel: 'Standard' });
      return {
        pickerOpened: true, modelVisible: true, modelSelected: true,
        effortVisible: true, effortSelected: true,
      };
    },
  });
  assert.equal(pickerCalls, 1);
  assert.equal(waitMs, 500);
  assert.equal(evidenceCaptures, 1);
  assert.equal(result.ok, true);
  assert.equal(result.operationData.modelProof.selectedBy, 'picker');
  assert.equal(
    result.operationData.modelProof.evidenceRefs[0].path,
    'post-selection-1.json',
  );
});

test('an opened picker without the requested model fails with concrete absence proof', async () => {
  let evidenceCaptures = 0;
  const result = await handleEnsureModel({
    request: request(), page: {}, evidenceRefs,
    captureModelState: async () => state('Instant', 'Standard'),
    captureEvidence: async () => {
      evidenceCaptures += 1;
      return [{ ...evidenceRefs[0], path: 'picker-absence.json' }];
    },
  }, {
    selectModelTuple: async () => ({
      pickerOpened: true, modelVisible: false, modelSelected: false,
      effortVisible: true, effortSelected: false,
    }),
  });
  assert.equal(result.ok, false);
  assert.equal(result.providerReason, 'picker.model_absent');
  assert.equal(evidenceCaptures, 1);
  assert.equal(result.operationData.failureProof.pickerOpened, true);
  assert.equal(result.operationData.failureProof.requestedModelVisible, false);
  assert.equal(result.operationData.failureProof.evidenceRefs[0].path, 'picker-absence.json');
});

test('a changed page binding returns the closed invocation failure instead of throwing', async () => {
  const changed = state('Pro', 'Standard');
  changed.pageBinding.bindingGeneration = 2;
  const result = await handleEnsureModel({
    request: request(), page: {}, evidenceRefs, captureModelState: async () => changed,
  });
  assert.equal(result.ok, false);
  assert.equal(result.providerReason, 'binding.mismatch');
  assert.equal(result.operationData.modelProof, null);
  assert.equal(result.operationData.failureProof, null);
  assert.equal(result.operationData.observedPageBinding.bindingGeneration, 2);
});
