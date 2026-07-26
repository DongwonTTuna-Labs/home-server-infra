
import {
  hasProModelEvidence,
  modelBlockedByUpgrade,
} from './readiness.mjs';
import { collectModelSurfaceEvidence } from './model-evidence/browser.mjs';
import {
  controlSignal,
  controlTextSignal,
  requestedProExtended,
  selectedMatchesRequestedModel,
  selectedModelControl,
} from './model-evidence/signals.mjs';
import { sanitizeModelEvidence } from './model-evidence/sanitize.mjs';

export { selectedModelControl } from './model-evidence/signals.mjs';

const MODEL_LABEL_RE = /^(?:Pro(?:\s+Extended)?|Instant|Extra\s+High|High|Auto|Thinking|Standard|Fast)$/i;

async function visibleModelPicker(page) {
  const candidates = page.locator('button,[role="button"],[aria-haspopup="menu"],[data-testid*="model" i],[aria-label*="model" i],div,span');
  const count = await candidates.count().catch(() => 0);
  const viewport = await page.evaluate(() => ({ width: innerWidth, height: innerHeight })).catch(() => ({ width: 0, height: 0 }));
  for (let index = 0; index < count; index += 1) {
    const candidate = candidates.nth(index);
    if (!await candidate.isVisible().catch(() => false)) continue;
    const box = await candidate.boundingBox().catch(() => null);
    if (!box || (viewport.height > 0 && box.y < viewport.height * 0.52)) continue;
    const details = await candidate.evaluate(node => ({
      text: (node.innerText || node.textContent || '').replace(/\s+/g, ' ').trim(),
      label: node.getAttribute('aria-label') || '',
      title: node.getAttribute('title') || '',
      testid: node.getAttribute('data-testid') || '',
      tag: node.tagName || '',
    })).catch(() => null);
    if (!details) continue;
    const signal = `${details.text} ${details.label} ${details.title} ${details.testid}`;
    if (/account|profile|avatar|workspace|user-menu|accounts-profile/i.test(signal)) continue;
    if (MODEL_LABEL_RE.test(details.text) || /model|switcher|모델|프로/i.test(signal)) {
      return { locator: candidate, details, box };
    }
  }
  return null;
}

async function visibleModelOption(page, model, effort) {
  // Current ChatGPT 5.6 exposes the selectable product as `Pro` even when
  // the wrapper carries an `extended` effort hint. Older builds may say
  // `Pro Extended`; accept both without treating effort as a product name.
  const expected = String(model || '').toLowerCase() === 'pro'
    ? /^Pro(?:\s+Extended)?$/i
    : /^Pro$/i;
  const options = page.locator('[role="menuitem"],[role="option"],[role="menu"] button,[role="menu"] [role="button"],button,div,span');
  const count = await options.count().catch(() => 0);
  for (let index = 0; index < count; index += 1) {
    const option = options.nth(index);
    if (!await option.isVisible().catch(() => false)) continue;
    const text = await option.innerText().catch(() => '');
    if (expected.test(String(text).replace(/\s+/g, ' ').trim())) return option;
  }
  return null;
}

/**
 * Model-picker-first gate.  The selected composer label is never silently
 * downgraded: open the visible picker, choose the requested Pro option when it
 * is present, then recapture model evidence.  If the picker is unavailable
 * but the composer already proves Pro, preserve that positive evidence; emit a
 * mismatch only when Pro is absent or post-selection verification fails.
 */
export async function prepareRequestedModel(page, model, effort) {
  const before = await verifyRequestedModel(page, model, effort);
  const picker = await visibleModelPicker(page);
  if (!picker) return { ...before, picker: { opened: false, option: false, reason: 'picker_control_not_found' } };
  let opened = false;
  let optionSelected = false;
  try {
    await picker.locator.click({ timeout: 10_000 });
    opened = true;
    await page.waitForTimeout(250);
    const option = await visibleModelOption(page, model, effort);
    if (option) {
      await option.click({ timeout: 10_000 });
      optionSelected = true;
      await page.waitForTimeout(250);
    }
  } catch {
    // Re-read the visible composer below; transient picker DOM drift is not
    // evidence that the requested model is unavailable.
  }
  const after = await verifyRequestedModel(page, model, effort);
  return {
    ...after,
    picker: { opened, option: optionSelected, beforeOk: before.ok },
  };
}

export async function verifyRequestedModel(page, model, effort) {
  const rawEvidence = await collectModelSurfaceEvidence(page);
  const bodyText = rawEvidence.bodyText || '';
  const controls = rawEvidence.controls || [];
  const haystack = `${bodyText}\n${controls.map(item => controlSignal(item)).join('\n')}`;
  const selected = selectedModelControl(controls);
  const selectedText = selected.control ? controlTextSignal(selected.control) : '';
  const sanitizedEvidence = sanitizeModelEvidence({
    bodyText,
    controls,
    selectedText,
    selectedIndex: selected.index,
    selectedScore: selected.score,
    error: rawEvidence.error,
  });

  const upgrade = /\bUpgrade\b|Get Plus|Go Plus|Free plan/i.test(haystack);
  const proEvidence = hasProModelEvidence({ selectedText });
  if (modelBlockedByUpgrade({ upgrade, proEvidence })) {
    return { ok: false, status: 'subscription_required', reason: 'auth.needs_pro', evidence: sanitizedEvidence };
  }
  if (!selected.control) {
    return {
      ok: false,
      status: 'model.selection_mismatch',
      reason: 'model.selection_unverified',
      model,
      effort,
      expectedLabel: requestedProExtended(model, effort) ? 'Pro' : undefined,
      evidence: sanitizedEvidence,
    };
  }
  if (!selectedMatchesRequestedModel({ selectedText, model, effort })) {
    return {
      ok: false,
      status: 'model.selection_mismatch',
      reason: 'model.selection_mismatch',
      model,
      effort,
      expectedLabel: requestedProExtended(model, effort) ? 'Pro' : undefined,
      selectedTextLength: sanitizedEvidence.selectedTextLength,
      selectedTextSha256: sanitizedEvidence.selectedTextSha256,
      evidence: sanitizedEvidence,
    };
  }
  return {
    ok: true,
    model,
    effort,
    selectedTextLength: sanitizedEvidence.selectedTextLength,
    selectedTextSha256: sanitizedEvidence.selectedTextSha256,
    evidence: sanitizedEvidence,
  };
}
