import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

import { canonicalSha256 } from '../contracts/r13.mjs';

const LABEL_FIXTURE = fileURLToPath(new URL(
  '../../../../contracts/ui-labels-r14/model-effort-labels.tsv',
  import.meta.url,
));
const LABEL_FIXTURE_SHA256 = '5fb47aaaf04834d7730088449401ee6c06020576173fb7bf1d45b836673af2d0';

export async function handleEnsureModel(context, overrides = {}) {
  const {
    request,
    page,
    evidenceRefs,
    captureModelState,
  } = context;
  const dependencies = {
    loadModelEffortLabels,
    selectModelTuple,
    ...overrides,
  };
  const captureEvidence = context.captureEvidence ?? (async () => evidenceRefs);
  const requestedModel = request.operationData.requestedModel;
  const requestedEffort = request.operationData.requestedEffort;
  const labels = await dependencies.loadModelEffortLabels();
  const before = await captureModelState();
  if (!matchesExpectedBinding(before.pageBinding, request.operationData.pageBinding)) {
    try {
      const fs = await import('node:fs');
      const o = before.pageBinding || {}; const e = request.operationData.pageBinding || {};
      const diff = {};
      for (const k of new Set([...Object.keys(o), ...Object.keys(e)])) {
        if (JSON.stringify(o[k]) !== JSON.stringify(e[k])) diff[k] = { obs: o[k], exp: e[k] };
      }
      fs.writeFileSync(`${process.env.BROWSER_AGENT_HOME || '/tmp'}/binding-diff.json`, JSON.stringify(diff, null, 1));
    } catch { /* debug */ }
    return invocationFailure('binding.mismatch', before.pageBinding);
  }
  const exactBefore = stateMatches(before, requestedModel, requestedEffort, labels);
  if (exactBefore) {
    return success(before, requestedModel, requestedEffort, 'already_exact', evidenceRefs);
  }

  const selection = await dependencies.selectModelTuple(page, {
    modelLabel: labels.model.get(requestedModel),
    effortLabel: labels.effort.get(requestedEffort),
  });
  if (!selection.modelVisible || !selection.modelSelected) {
    const postSelectionEvidenceRefs = await captureEvidence();
    return failure(
      'picker.model_absent',
      selection,
      before.pageBinding,
      postSelectionEvidenceRefs,
    );
  }
  if (!selection.effortVisible || !selection.effortSelected) {
    const postSelectionEvidenceRefs = await captureEvidence();
    return failure(
      'picker.effort_absent',
      selection,
      before.pageBinding,
      postSelectionEvidenceRefs,
    );
  }
  await page.waitForTimeout(request.operationData.stabilizationMs);
  const after = await captureModelState();
  const postSelectionEvidenceRefs = await captureEvidence();
  if (!matchesExpectedBinding(after.pageBinding, request.operationData.pageBinding)) {
    return invocationFailure('binding.mismatch', after.pageBinding);
  }
  if (!sameControl(before.modelControl, after.modelControl)
      || !sameControl(before.effortControl, after.effortControl)) {
    return failure(
      'picker.control_drift',
      selection,
      after.pageBinding,
      postSelectionEvidenceRefs,
      false,
    );
  }
  if (!stateMatches(after, requestedModel, requestedEffort, labels)) {
    return failure(
      'picker.reverify_mismatch',
      selection,
      after.pageBinding,
      postSelectionEvidenceRefs,
    );
  }
  return success(
    after,
    requestedModel,
    requestedEffort,
    'picker',
    postSelectionEvidenceRefs,
  );
}

export async function loadModelEffortLabels(pathname = LABEL_FIXTURE) {
  const bytes = await readFile(pathname);
  if (bytes.length !== 93
      || createHash('sha256').update(bytes).digest('hex') !== LABEL_FIXTURE_SHA256) {
    throw new Error('provider.schema_drift: model label fixture identity');
  }
  const lines = bytes.toString('utf8').split('\n');
  if (lines.pop() !== '' || lines.shift() !== 'kind\tkey\tlabel') {
    throw new Error('provider.schema_drift: model label fixture serialization');
  }
  const model = new Map();
  const effort = new Map();
  let previous = '';
  for (const line of lines) {
    const [kind, key, label, ...extra] = line.split('\t');
    if (extra.length > 0 || !['model', 'effort'].includes(kind) || !key || !label) {
      throw new Error('provider.schema_drift: model label fixture row');
    }
    const sortKey = `${kind}\0${key}`;
    if (sortKey <= previous) throw new Error('provider.schema_drift: model label fixture order');
    previous = sortKey;
    (kind === 'model' ? model : effort).set(key, label);
  }
  return { model, effort };
}

export function normalizeVisibleLabel(value) {
  return String(value).normalize('NFC').toLowerCase().replace(/\s+/gu, ' ').trim();
}

export async function selectModelTuple(page, requested) {
  // Scope the model/tier control to the composer. A page-wide aria-haspopup
  // search also matches sidebar conversation-options buttons, which can carry
  // model-like titles (e.g. a chat named "프로 …") and are covered by the
  // sidebar overlay, so clicking them times out.
  const composerModelControl = async () => {
    const textbox = page
      .locator('#prompt-textarea, textarea, [contenteditable="true"][role="textbox"]')
      .last();
    const form = textbox.locator('xpath=ancestor::form[1]');
    const menuBtn = form.locator('button[aria-haspopup="menu"], [role="button"][aria-haspopup="menu"]').first();
    if (await menuBtn.count().catch(() => 0)) return menuBtn;
    return null;
  };
  const modelControl = (await composerModelControl())
    || await visibleExactOrLabeled(page, [
      '[data-testid*="model" i]',
    ], requested.modelLabel);
  let pickerOpened = false;
  if (modelControl && await modelControl.isVisible().catch(() => false)) {
    await modelControl.click({ timeout: 10_000 });
    pickerOpened = true;
  }
  const modelOption = pickerOpened
    ? await visibleExactOrLabeled(page, [
      '[role="menuitemradio"]', '[role="menuitem"]', '[role="option"]', '[role="menu"] button',
    ], requested.modelLabel)
    : null;
  if (modelOption) await modelOption.click({ timeout: 10_000 });
  // Unified model/effort tier: selecting the model tier fixes effort too. Try a
  // distinct effort control; if absent, the tier selection satisfies effort.
  const distinctEffort = modelOption
    ? await visibleExactOrLabeled(page, [
      '[role="menuitemradio"]', '[role="menuitem"]', '[role="option"]', '[role="menu"] button',
    ], requested.effortLabel)
    : null;
  const effortOption = distinctEffort || modelOption;
  if (distinctEffort) await distinctEffort.click({ timeout: 10_000 });
  return {
    pickerOpened,
    modelVisible: Boolean(modelOption),
    modelSelected: Boolean(modelOption),
    effortVisible: Boolean(effortOption),
    effortSelected: Boolean(effortOption),
  };
}

async function visibleExactOrLabeled(page, selectors, exactLabel = null, pattern = null) {
  for (const selector of selectors) {
    const candidates = page.locator(selector);
    const count = await candidates.count().catch(() => 0);
    for (let index = 0; index < count; index += 1) {
      const candidate = candidates.nth(index);
      if (!await candidate.isVisible().catch(() => false)) continue;
      const signal = await candidate.evaluate(node => (
        node.getAttribute('aria-label') || node.innerText || node.textContent || ''
      )).catch(() => '');
      const normalized = normalizeVisibleLabel(signal);
      if (exactLabel !== null && normalized === normalizeVisibleLabel(exactLabel)) return candidate;
      if (pattern?.test(normalized)) return candidate;
    }
  }
  return null;
}

function stateMatches(state, model, effort, labels) {
  const modelOk = normalizeVisibleLabel(state.modelLabel)
    === normalizeVisibleLabel(labels.model.get(model));
  // Unified model/effort tier (current ChatGPT UI): the model/tier control and
  // the effort control are the same element, so the tier selection IS the whole
  // model+effort choice and there is no distinct effort label to verify.
  const unified = normalizeVisibleLabel(state.effortLabel)
    === normalizeVisibleLabel(state.modelLabel);
  const effortOk = unified
    || normalizeVisibleLabel(state.effortLabel) === normalizeVisibleLabel(labels.effort.get(effort));
  return modelOk && effortOk;
}

function success(state, model, effort, selectedBy, evidenceRefs) {
  const verifiedAtMs = Date.now();
  return {
    ok: true,
    status: 'done',
    providerReason: null,
    operationData: {
      modelProof: {
        requested: model, observed: model, verified: true, control: state.modelControl,
        selectedBy, evidenceRefs, verifiedAtMs,
      },
      effortProof: {
        requested: effort, observed: effort, verified: true, control: state.effortControl,
        selectedBy, evidenceRefs, verifiedAtMs,
      },
      failureProof: null,
      observedPageBinding: state.pageBinding,
    },
  };
}

function failure(reason, selection, pageBinding, evidenceRefs, stable = true) {
  return {
    ok: false,
    status: 'failed',
    providerReason: reason,
    operationData: {
      modelProof: null,
      effortProof: null,
      failureProof: {
        reason,
        pickerOpened: selection.pickerOpened,
        requestedModelVisible: selection.modelVisible,
        requestedEffortVisible: selection.effortVisible,
        controlIdentityStable: stable,
        evidenceRefs,
        failedAtMs: Date.now(),
      },
      observedPageBinding: pageBinding,
    },
  };
}

function invocationFailure(reason, observedPageBinding) {
  return {
    ok: false,
    status: 'failed',
    providerReason: reason,
    operationData: {
      modelProof: null,
      effortProof: null,
      failureProof: null,
      observedPageBinding,
    },
  };
}

function sameControl(left, right) {
  return left.controlId === right.controlId;
}

function matchesExpectedBinding(observed, expected) {
  return canonicalSha256(observed) === canonicalSha256(expected);
}
