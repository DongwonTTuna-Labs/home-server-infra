import { createHash } from 'node:crypto';
import path from 'node:path';

import { setFiles } from '../browser.mjs';
import { canonicalSha256, deriveChipStableKey } from '../contracts/r13.mjs';

const ATTACHMENT_ROOT = '/broker-attachments';

export async function handleUploadOnly(context, overrides = {}) {
  const { request, page, evidenceRefs, observePageBinding } = context;
  const dependencies = {
    observeUploadChips,
    setFiles,
    waitForUploadChips,
    ...overrides,
  };
  const { attachmentSet, retryIndex, uploadAttemptId } = request.operationData;
  const before = await dependencies.observeUploadChips(page);
  const files = attachmentSet.records.map(record => {
    const absolute = path.resolve(ATTACHMENT_ROOT, record.containerRelPath);
    if (!absolute.startsWith(`${ATTACHMENT_ROOT}/`)) {
      throw new Error('contract.invalid_provider_envelope: attachment path');
    }
    return absolute;
  });
  await dependencies.setFiles(page, files);
  const observed = await dependencies.waitForUploadChips(
    page,
    attachmentSet.count,
    Math.min(request.deadlineMs, 30_000),
  );
  const observedPageBinding = await observePageBinding();
  if (canonicalSha256(observedPageBinding) !== canonicalSha256(request.operationData.pageBinding)) {
    return uploadFailure('upload.incomplete', observedPageBinding);
  }
  const proof = uploadProof({
    attachmentSet,
    before,
    evidenceRefs,
    observed,
    pageIncarnationId: observedPageBinding.pageIncarnationId,
    retryIndex,
    uploadAttemptId,
  });
  if (proof.staleChips.length > 0) {
    if (retryIndex === 1) {
      return uploadFailure('upload.stale_chip_uncleared', observedPageBinding);
    }
    return {
      ok: false,
      status: 'failed',
      providerReason: 'upload.stale_chip_mismatch',
      operationData: {
        uploadProof: proof,
        failureReason: 'upload.stale_chip_mismatch',
        observedPageBinding,
      },
    };
  }
  if (!proof.allExpectedComplete) return uploadFailure('upload.incomplete', observedPageBinding);
  return {
    ok: true,
    status: 'done',
    providerReason: null,
    operationData: { uploadProof: proof, failureReason: null, observedPageBinding },
  };
}

export function normalizeChipStem(value) {
  let normalized = String(value).normalize('NFC').toLowerCase().replace(/\s+/gu, ' ').trim();
  normalized = normalized.replace(/\.[a-z0-9]{1,8}$/u, '');
  normalized = normalized.replace(/ \(([1-9]|[1-9][0-9])\)$/u, '');
  return normalized;
}

export function buildChipProofs(observations, pageIncarnationId, evidenceRefs, records = []) {
  const recordByStem = new Map(records.map(record => [
    normalizeChipStem(path.posix.basename(record.containerRelPath)),
    record,
  ]));
  const ordinals = new Map();
  return observations.map(observation => {
    const normalizedStem = normalizeChipStem(observation.accessibleFilename);
    const ordinal = ordinals.get(normalizedStem) ?? 0;
    ordinals.set(normalizedStem, ordinal + 1);
    const record = recordByStem.get(normalizedStem);
    const box = observation.boundingBox;
    return {
      boundingBoxHash: `sha256:${canonicalSha256([
        Math.round(box.x), Math.round(box.y), Math.round(box.width), Math.round(box.height),
      ])}`,
      chipStableKey: deriveChipStableKey(pageIncarnationId, normalizedStem, ordinal),
      complete: observation.complete === true,
      digest: record?.sourceSha256 ?? null,
      evidenceRefs,
      labelHash: `sha256:${createHash('sha256').update(
        Buffer.from(normalizeVisibleLabel(observation.accessibleFilename), 'utf8'),
      ).digest('hex')}`,
      visibleSizeBytes: Number.isInteger(observation.visibleSizeBytes)
        ? observation.visibleSizeBytes
        : null,
      normalizedStem,
    };
  });
}

export async function observeUploadChips(page) {
  const candidates = await page.evaluate(() => {
    const visible = node => {
      const rect = node?.getBoundingClientRect?.();
      if (!rect || rect.width <= 0 || rect.height <= 0) return false;
      const style = getComputedStyle(node);
      return style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0';
    };
    const domPath = node => {
      const result = [];
      let current = node;
      while (current && current !== document.documentElement) {
        const parent = current.parentElement;
        if (!parent) return null;
        result.unshift(Array.prototype.indexOf.call(parent.children, current));
        current = parent;
      }
      return current === document.documentElement ? result : null;
    };
    const seeds = Array.from(document.querySelectorAll(
      '[data-testid*="attachment" i],[class*="attachment" i],[data-testid*="file" i]',
    )).filter(visible);
    return seeds.map(seed => {
      const accessibleFilename = (
        seed.getAttribute('aria-label') || seed.getAttribute('title')
        || seed.innerText || seed.textContent || ''
      ).trim();
      let root = seed;
      while (root) {
        const controls = [
          ...(root.matches?.('button,[role="button"]') ? [root] : []),
          ...root.querySelectorAll('button,[role="button"]'),
        ].filter(visible);
        if (controls.length > 0 && visible(root)) break;
        root = root.parentElement;
      }
      if (!root || !accessibleFilename) return null;
      const rect = root.getBoundingClientRect();
      const busy = root.getAttribute('aria-busy') === 'true'
        || Boolean(root.querySelector('[role="progressbar"],[aria-busy="true"]'));
      return {
        accessibleFilename,
        boundingBox: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
        complete: !busy,
        filenameDomPath: domPath(seed),
        rootDomPath: domPath(root),
        visibleSizeBytes: null,
      };
    }).filter(Boolean);
  }).catch(() => []);
  return selectCanonicalChipRootObservations(candidates);
}

export function selectCanonicalChipRootObservations(candidates) {
  const groups = new Map();
  for (const candidate of candidates) {
    if (!validDomPath(candidate.rootDomPath) || !validDomPath(candidate.filenameDomPath)
        || !isPathPrefix(candidate.rootDomPath, candidate.filenameDomPath)) continue;
    const key = candidate.rootDomPath.join('.');
    const group = groups.get(key) ?? [];
    group.push(candidate);
    groups.set(key, group);
  }
  const roots = [];
  for (const group of groups.values()) {
    const labels = new Set(group.map(item => normalizeVisibleLabel(item.accessibleFilename)));
    if (labels.size !== 1 || labels.has('')) continue;
    const ordered = [...group].sort((left, right) => (
      right.filenameDomPath.length - left.filenameDomPath.length
      || compareDomPaths(left.filenameDomPath, right.filenameDomPath)
    ));
    roots.push(ordered[0]);
  }
  return roots
    .filter(candidate => !roots.some(other => (
      other !== candidate
      && candidate.rootDomPath.length < other.rootDomPath.length
      && isPathPrefix(candidate.rootDomPath, other.rootDomPath)
    )))
    .sort((left, right) => compareDomPaths(left.rootDomPath, right.rootDomPath));
}

export async function waitForUploadChips(page, expectedCount, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let observations = await observeUploadChips(page);
  while (Date.now() < deadline) {
    observations = await observeUploadChips(page);
    if (observations.length >= expectedCount && observations.every(item => item.complete)) {
      return observations;
    }
    await page.waitForTimeout(250);
  }
  return observations;
}

function uploadProof({
  attachmentSet,
  before,
  evidenceRefs,
  observed,
  pageIncarnationId,
  retryIndex,
  uploadAttemptId,
}) {
  const proofs = buildChipProofs(
    observed,
    pageIncarnationId,
    evidenceRefs,
    attachmentSet.records,
  );
  const expectedStems = new Set(attachmentSet.records.map(record => (
    normalizeChipStem(path.posix.basename(record.containerRelPath))
  )));
  const beforeKeys = new Set(buildChipProofs(
    before,
    pageIncarnationId,
    evidenceRefs,
  ).map(proof => proof.chipStableKey));
  const staleChips = proofs.filter(proof => (
    beforeKeys.has(proof.chipStableKey) || !expectedStems.has(proof.normalizedStem)
  ));
  const visibleCurrentChips = proofs.filter(proof => !staleChips.includes(proof));
  const strip = ({ normalizedStem: _normalizedStem, ...proof }) => proof;
  return {
    allExpectedComplete: staleChips.length === 0
      && visibleCurrentChips.length === attachmentSet.count
      && visibleCurrentChips.every(item => item.complete),
    capturedAtMs: Date.now(),
    expectedSetSha256: attachmentSet.setSha256,
    retryIndex,
    staleChips: staleChips.map(strip),
    uploadAttemptId,
    visibleCurrentChips: visibleCurrentChips.map(strip),
  };
}

function normalizeVisibleLabel(value) {
  return String(value).normalize('NFC').toLowerCase().replace(/\s+/gu, ' ').trim();
}

function validDomPath(value) {
  return Array.isArray(value) && value.every(item => Number.isInteger(item) && item >= 0);
}

function isPathPrefix(prefix, value) {
  return prefix.length <= value.length
    && prefix.every((item, index) => item === value[index]);
}

function compareDomPaths(left, right) {
  const length = Math.min(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    if (left[index] !== right[index]) return left[index] - right[index];
  }
  return left.length - right.length;
}

function uploadFailure(providerReason, observedPageBinding) {
  return {
    ok: false,
    status: 'failed',
    providerReason,
    operationData: { uploadProof: null, failureReason: providerReason, observedPageBinding },
  };
}
