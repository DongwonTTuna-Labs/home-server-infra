import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

import { canonicalSha256 } from '../contracts/r13.mjs';
import { buildChipProofs, normalizeChipStem, observeUploadChips } from './upload-only.mjs';

const REMOVAL_FIXTURE = fileURLToPath(new URL(
  '../../../../contracts/ui-labels-r14/chip-removal-labels.tsv',
  import.meta.url,
));
const REMOVAL_FIXTURE_BYTES = 78;
const REMOVAL_FIXTURE_SHA256 = '5f72d20331679072012c7bfecf7e71dccd6df346c68a4fed3e3e9180782c4b03';

export async function handleClearUpload(context, overrides = {}) {
  const { request, page, evidenceRefs, observePageBinding } = context;
  const dependencies = {
    loadChipRemovalLabels,
    observeUploadChips,
    removeChipByObservation,
    ...overrides,
  };
  const labels = await dependencies.loadChipRemovalLabels();
  const attemptedChipKeys = [];
  const clearedChips = [];
  for (const stale of request.operationData.staleChips) {
    const before = await dependencies.observeUploadChips(page);
    const proofs = buildChipProofs(
      before,
      request.operationData.pageBinding.pageIncarnationId,
      evidenceRefs,
    );
    const indexes = proofs
      .map((proof, index) => (proof.chipStableKey === stale.chipStableKey ? index : -1))
      .filter(index => index >= 0);
    attemptedChipKeys.push(stale.chipStableKey);
    if (indexes.length !== 1) {
      return clearFailure(request, attemptedChipKeys, clearedChips, await observePageBinding());
    }
    const target = before[indexes[0]];
    const stem = normalizeChipStem(target.accessibleFilename);
    const dupOrdinal = before.slice(0, indexes[0])
      .filter(item => normalizeChipStem(item.accessibleFilename) === stem).length;
    const countBefore = before.filter(item => normalizeChipStem(item.accessibleFilename) === stem).length;
    const removed = await dependencies.removeChipByObservation(page, {
      ...target,
      dupOrdinal,
      normalizedStem: stem,
    }, labels, dependencies.observeUploadChips);
    if (!removed) {
      return clearFailure(request, attemptedChipKeys, clearedChips, await observePageBinding());
    }
    await page.waitForTimeout(250);
    const after = await dependencies.observeUploadChips(page);
    const countAfter = after.filter(item => normalizeChipStem(item.accessibleFilename) === stem).length;
    if (countAfter !== countBefore - 1) {
      return clearFailure(request, attemptedChipKeys, clearedChips, await observePageBinding());
    }
    clearedChips.push({ chipStableKey: stale.chipStableKey, digest: stale.digest, cleared: true });
  }
  const observedPageBinding = await observePageBinding();
  if (canonicalSha256(observedPageBinding)
      !== canonicalSha256(request.operationData.pageBinding)) {
    return clearFailure(request, attemptedChipKeys, clearedChips, observedPageBinding);
  }
  return {
    ok: true,
    status: 'done',
    providerReason: null,
    operationData: {
      clearAttemptId: request.operationData.clearAttemptId,
      clearedChips,
      observedPageBinding,
    },
  };
}

export async function loadChipRemovalLabels(pathname = REMOVAL_FIXTURE) {
  const bytes = await readFile(pathname);
  if (bytes.length !== REMOVAL_FIXTURE_BYTES
      || createHash('sha256').update(bytes).digest('hex') !== REMOVAL_FIXTURE_SHA256) {
    throw new Error('provider.schema_drift: chip removal fixture identity');
  }
  const lines = bytes.toString('utf8').split('\n');
  if (lines.pop() !== '' || lines.shift() !== 'kind\tkey\tlabel') {
    throw new Error('provider.schema_drift: chip removal fixture serialization');
  }
  const labels = new Set();
  let previous = '';
  for (const line of lines) {
    const [kind, key, label, ...extra] = line.split('\t');
    if (extra.length > 0 || kind !== 'chip_removal' || !key || !label) {
      throw new Error('provider.schema_drift: chip removal fixture row');
    }
    const sortKey = `${kind}\0${key}`;
    if (sortKey <= previous) {
      throw new Error('provider.schema_drift: chip removal fixture order');
    }
    previous = sortKey;
    labels.add(normalizeLabel(label));
  }
  if (labels.size === 0) throw new Error('provider.schema_drift: chip removal fixture empty');
  return labels;
}

export async function removeChipByObservation(
  page,
  observation,
  labels,
  observe = observeUploadChips,
) {
  if (!Number.isInteger(observation?.dupOrdinal)
      || observation.dupOrdinal < 0
      || typeof observation.normalizedStem !== 'string'
      || observation.normalizedStem.length === 0) return false;
  const current = await observe(page);
  const currentTarget = current
    .filter(item => normalizeChipStem(item.accessibleFilename) === observation.normalizedStem)
    .at(observation.dupOrdinal);
  if (!validChipPaths(currentTarget)) return false;
  const root = locatorForDomPath(page, currentTarget.rootDomPath);
  if (!await root.isVisible().catch(() => false)) return false;
  const filename = locatorForDomPath(page, currentTarget.filenameDomPath);
  if (!await filename.isVisible().catch(() => false)) return false;
  const accessibleFilename = await filename.evaluate(node => (
    node.getAttribute('aria-label') || node.getAttribute('title')
    || node.innerText || node.textContent || ''
  )).catch(() => '');
  if (normalizeChipStem(accessibleFilename) !== observation.normalizedStem) return false;
  const controls = root.locator('button,[role="button"]');
  const matches = [];
  const count = await controls.count().catch(() => 0);
  for (let index = 0; index < count; index += 1) {
    const control = controls.nth(index);
    if (!await control.isVisible().catch(() => false)) continue;
    const name = await control.evaluate(node => (
      node.getAttribute('aria-label') || node.innerText || node.textContent || ''
    )).catch(() => '');
    if (labels.has(normalizeLabel(name))) matches.push(control);
  }
  if (matches.length !== 1) return false;
  await matches[0].click({ timeout: 10_000 });
  return true;
}

function validChipPaths(observation) {
  return Array.isArray(observation?.rootDomPath)
    && Array.isArray(observation?.filenameDomPath)
    && observation.rootDomPath.every(item => Number.isInteger(item) && item >= 0)
    && observation.filenameDomPath.every(item => Number.isInteger(item) && item >= 0)
    && observation.rootDomPath.length <= observation.filenameDomPath.length
    && observation.rootDomPath.every(
      (item, index) => item === observation.filenameDomPath[index],
    );
}

function locatorForDomPath(page, domPath) {
  let locator = page.locator('html');
  for (const ordinal of domPath) {
    locator = locator.locator(`:scope > :nth-child(${ordinal + 1})`);
  }
  return locator;
}

function normalizeLabel(value) {
  return String(value).normalize('NFC').toLowerCase().replace(/\s+/gu, ' ').trim();
}

function clearFailure(request, attemptedChipKeys, clearedChips, observedPageBinding) {
  return {
    ok: false,
    status: 'failed',
    providerReason: 'upload.chip_removal_failed',
    operationData: {
      attemptedChipKeys,
      clearAttemptId: request.operationData.clearAttemptId,
      clearedChips,
      failureReason: 'upload.chip_removal_failed',
      observedPageBinding,
    },
  };
}
