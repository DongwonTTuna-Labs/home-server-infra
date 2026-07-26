import assert from 'node:assert/strict';
import test from 'node:test';

import {
  handleClearUpload,
  loadChipRemovalLabels,
  removeChipByObservation,
} from '../lib/commands/clear-upload.mjs';
import {
  buildChipProofs,
  handleUploadOnly,
  normalizeChipStem,
  selectCanonicalChipRootObservations,
} from '../lib/commands/upload-only.mjs';

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

const evidenceRefs = [{
  path: 'dom.sanitized.json', sha256: h256('a'), sizeBytes: 1, mediaType: 'application/json',
}];

function observation(name, complete = true, x = 0) {
  return {
    accessibleFilename: name,
    boundingBox: { x, y: 1, width: 100, height: 20 },
    complete,
    visibleSizeBytes: null,
  };
}

function uploadRequest(retryIndex = 0) {
  return {
    deadlineMs: 30_000,
    operationData: {
      attachmentSet: {
        count: 1,
        records: [{
          containerRelPath: 'run-1/001-aaaaaaaaaaaaaaaa.txt', mediaType: 'text/plain',
          ordinal: 0, sizeBytes: 7, sourceSha256: h256('a'),
        }],
        setSha256: h256('e'),
      },
      pageBinding: binding(), retryIndex, uploadAttemptId: 'upload-1',
    },
  };
}

test('chip identity strips one extension and duplicate marker and preserves DOM ordinal', () => {
  assert.equal(normalizeChipStem(' Report   Final (2).TXT '), 'report final');
  const proofs = buildChipProofs([
    observation('report.txt', true, 0), observation('report (2).txt', true, 20),
  ], binding().pageIncarnationId, evidenceRefs);
  assert.notEqual(proofs[0].chipStableKey, proofs[1].chipStableKey);
  assert.equal(proofs[0].digest, null);
  assert.equal(proofs[1].complete, true);
});

test('chip observation keeps the canonical smallest root and rejects ambiguous root labels', () => {
  const candidate = (accessibleFilename, rootDomPath, filenameDomPath) => ({
    accessibleFilename,
    boundingBox: { x: 0, y: 0, width: 10, height: 10 },
    complete: true,
    filenameDomPath,
    rootDomPath,
    visibleSizeBytes: null,
  });
  const selected = selectCanonicalChipRootObservations([
    candidate('old.pdf', [1], [1, 0]),
    candidate('old.pdf', [1, 2], [1, 2, 0]),
    candidate('other.pdf', [3], [3, 0]),
    candidate('conflict.pdf', [3], [3, 1]),
  ]);
  assert.deepEqual(selected.map(item => item.rootDomPath), [[1, 2]]);
  assert.equal(selected[0].accessibleFilename, 'old.pdf');
});

test('upload-only attaches staged paths once and returns a complete current proof without sending', async () => {
  const req = uploadRequest();
  let attached = null;
  const result = await handleUploadOnly({
    request: req, page: {}, evidenceRefs,
    observePageBinding: async () => binding(),
  }, {
    observeUploadChips: async () => [],
    setFiles: async (_page, files) => { attached = files; },
    waitForUploadChips: async () => [observation('001-aaaaaaaaaaaaaaaa.txt')],
  });
  assert.deepEqual(attached, ['/broker-attachments/run-1/001-aaaaaaaaaaaaaaaa.txt']);
  assert.equal(result.ok, true);
  assert.equal(result.operationData.uploadProof.allExpectedComplete, true);
  assert.equal(result.operationData.uploadProof.visibleCurrentChips[0].digest, h256('a'));
});

test('upload-only preserves a changed binding in a valid failure observation', async () => {
  const changed = binding();
  changed.bindingGeneration = 2;
  const result = await handleUploadOnly({
    request: uploadRequest(), page: {}, evidenceRefs,
    observePageBinding: async () => changed,
  }, {
    observeUploadChips: async () => [],
    setFiles: async () => undefined,
    waitForUploadChips: async () => [observation('001-aaaaaaaaaaaaaaaa.txt')],
  });
  assert.equal(result.ok, false);
  assert.equal(result.providerReason, 'upload.incomplete');
  assert.equal(result.operationData.observedPageBinding.bindingGeneration, 2);
});

test('retry zero returns concrete stale proof while retry one fails closed as uncleared', async () => {
  const run = async retryIndex => handleUploadOnly({
    request: uploadRequest(retryIndex), page: {}, evidenceRefs,
    observePageBinding: async () => binding(),
  }, {
    observeUploadChips: async () => [observation('old.pdf')],
    setFiles: async () => undefined,
    waitForUploadChips: async () => [
      observation('old.pdf'), observation('001-aaaaaaaaaaaaaaaa.txt', true, 20),
    ],
  });
  const first = await run(0);
  assert.equal(first.providerReason, 'upload.stale_chip_mismatch');
  assert.equal(first.operationData.uploadProof.staleChips.length, 1);
  const second = await run(1);
  assert.equal(second.providerReason, 'upload.stale_chip_uncleared');
  assert.equal(second.operationData.uploadProof, null);
});

test('same-stem upload keeps the pre-existing ordinal stale and accepts only the new duplicate', async () => {
  const result = await handleUploadOnly({
    request: uploadRequest(), page: {}, evidenceRefs,
    observePageBinding: async () => binding(),
  }, {
    observeUploadChips: async () => [observation('001-aaaaaaaaaaaaaaaa.txt')],
    setFiles: async () => undefined,
    waitForUploadChips: async () => [
      observation('001-aaaaaaaaaaaaaaaa.txt'),
      observation('001-aaaaaaaaaaaaaaaa (2).txt', true, 20),
    ],
  });
  assert.equal(result.providerReason, 'upload.stale_chip_mismatch');
  assert.equal(result.operationData.uploadProof.staleChips.length, 1);
  assert.equal(result.operationData.uploadProof.visibleCurrentChips.length, 1);
  assert.notEqual(
    result.operationData.uploadProof.staleChips[0].chipStableKey,
    result.operationData.uploadProof.visibleCurrentChips[0].chipStableKey,
  );
});

test('clear-upload removes only the unique label-associated requested chip and verifies count drop', async () => {
  const builtProof = buildChipProofs(
    [observation('old.pdf')], binding().pageIncarnationId, evidenceRefs,
  )[0];
  const staleProof = (({ normalizedStem: _ignored, ...value }) => value)(builtProof);
  let chips = [observation('old.pdf')];
  let removalCalls = 0;
  const request = {
    operationData: {
      clearAttemptId: 'clear-1', pageBinding: binding(), staleChips: [staleProof],
      uploadAttemptId: 'upload-1',
    },
  };
  const result = await handleClearUpload({
    request,
    page: { waitForTimeout: async () => undefined },
    evidenceRefs,
    observePageBinding: async () => binding(),
  }, {
    loadChipRemovalLabels: async () => new Set(['remove file', 'delete']),
    observeUploadChips: async () => chips,
    removeChipByObservation: async (_page, target, labels) => {
      removalCalls += 1;
      assert.equal(target.normalizedStem, 'old');
      assert.equal(target.dupOrdinal, 0);
      assert(labels.has('remove file'));
      chips = [];
      return true;
    },
  });
  assert.equal(removalCalls, 1);
  assert.equal(result.ok, true);
  assert.deepEqual(result.operationData.clearedChips, [{
    chipStableKey: staleProof.chipStableKey, digest: null, cleared: true,
  }]);
});

test('clear-upload fails instead of guessing when removal association is not unique', async () => {
  const builtProof = buildChipProofs(
    [observation('old.pdf')], binding().pageIncarnationId, evidenceRefs,
  )[0];
  const staleProof = (({ normalizedStem: _ignored, ...value }) => value)(builtProof);
  const request = {
    operationData: {
      clearAttemptId: 'clear-1', pageBinding: binding(), staleChips: [staleProof],
      uploadAttemptId: 'upload-1',
    },
  };
  const result = await handleClearUpload({
    request, page: { waitForTimeout: async () => undefined }, evidenceRefs,
    observePageBinding: async () => binding(),
  }, {
    loadChipRemovalLabels: async () => new Set(['remove file']),
    observeUploadChips: async () => [observation('old.pdf')],
    removeChipByObservation: async () => false,
  });
  assert.equal(result.ok, false);
  assert.equal(result.providerReason, 'upload.chip_removal_failed');
  assert.deepEqual(result.operationData.attemptedChipKeys, [staleProof.chipStableKey]);
});

test('clear-upload preserves a post-clear binding change as a closed failure', async () => {
  const builtProof = buildChipProofs(
    [observation('old.pdf')], binding().pageIncarnationId, evidenceRefs,
  )[0];
  const staleProof = (({ normalizedStem: _ignored, ...value }) => value)(builtProof);
  let chips = [observation('old.pdf')];
  const changed = binding();
  changed.bindingGeneration = 2;
  const result = await handleClearUpload({
    request: {
      operationData: {
        clearAttemptId: 'clear-1', pageBinding: binding(), staleChips: [staleProof],
        uploadAttemptId: 'upload-1',
      },
    },
    page: { waitForTimeout: async () => undefined },
    evidenceRefs,
    observePageBinding: async () => changed,
  }, {
    loadChipRemovalLabels: async () => new Set(['remove file']),
    observeUploadChips: async () => chips,
    removeChipByObservation: async () => { chips = []; return true; },
  });
  assert.equal(result.ok, false);
  assert.equal(result.providerReason, 'upload.chip_removal_failed');
  assert.equal(result.operationData.observedPageBinding.bindingGeneration, 2);
});

test('clear-upload re-evaluates duplicate ordinal and never clicks a stale DOM path', async () => {
  const clicked = [];
  const nodes = new Map([
    ['0', { filename: 'unrelated.pdf', controls: ['delete'] }],
    ['0.0', { filename: 'unrelated.pdf' }],
    ['2', { filename: 'old (2).pdf', controls: ['remove file'] }],
    ['2.0', { filename: 'old (2).pdf' }],
  ]);
  const locatorFor = domPath => ({
    locator(selector) {
      const match = /^:scope > :nth-child\((\d+)\)$/u.exec(selector);
      if (match) return locatorFor([...domPath, Number(match[1]) - 1]);
      assert.equal(selector, 'button,[role="button"]');
      const controls = nodes.get(domPath.join('.'))?.controls ?? [];
      return {
        count: async () => controls.length,
        nth: index => ({
          click: async () => { clicked.push(domPath.join('.')); },
          evaluate: async () => controls[index],
          isVisible: async () => true,
        }),
      };
    },
    evaluate: async () => nodes.get(domPath.join('.'))?.filename ?? '',
    isVisible: async () => nodes.has(domPath.join('.')),
  });
  const page = {
    locator: selector => {
      assert.equal(selector, 'html');
      return locatorFor([]);
    },
  };
  const current = [
    { accessibleFilename: 'other.txt', rootDomPath: [0], filenameDomPath: [0, 0] },
    { accessibleFilename: 'old (2).pdf', rootDomPath: [2], filenameDomPath: [2, 0] },
  ];
  const removed = await removeChipByObservation(page, {
    accessibleFilename: 'old.pdf',
    dupOrdinal: 0,
    filenameDomPath: [0, 0],
    normalizedStem: 'old',
    rootDomPath: [0],
  }, new Set(['remove file', 'delete']), async () => current);
  assert.equal(removed, true);
  assert.deepEqual(clicked, ['2']);
});

test('production chip-removal fixture is identity-pinned and loads its exact labels', async () => {
  assert.deepEqual(
    [...await loadChipRemovalLabels()],
    ['remove file', 'delete'],
  );
});
