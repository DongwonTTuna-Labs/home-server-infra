
import { sha256Text } from '../common.mjs';
import { MODEL_SURFACE_RE, controlSignal } from './signals.mjs';

function textDigestEvidence(value) {
  const text = String(value || '');
  return {
    length: text.length,
    sha256: text ? sha256Text(text) : '',
  };
}

function sanitizeModelControlEvidence(control, index, selectedIndex, selectedScore) {
  const text = String(control?.text || '');
  const label = String(control?.label || '');
  const title = String(control?.title || '');
  const testid = String(control?.testid || '');
  const signal = controlSignal(control);
  return {
    index,
    selected: index === selectedIndex || undefined,
    selectionScore: index === selectedIndex && Number.isFinite(Number(selectedScore)) ? Number(selectedScore) : undefined,
    textLength: text.length,
    textSha256: text ? sha256Text(text) : '',
    labelLength: label.length,
    labelSha256: label ? sha256Text(label) : '',
    titleLength: title.length,
    titleSha256: title ? sha256Text(title) : '',
    testidLength: testid.length,
    testidSha256: testid ? sha256Text(testid) : '',
    matchesModel: MODEL_SURFACE_RE.test(signal),
    matchesUpgrade: /\bUpgrade\b|Get Plus|Go Plus|Free plan/i.test(signal),
    inComposerRoot: Boolean(control?.inComposerRoot) || undefined,
    nearComposer: Boolean(control?.nearComposer) || undefined,
    bottomComposerBand: Boolean(control?.bottomComposerBand) || undefined,
    sidebarOrNav: Boolean(control?.sidebarOrNav) || undefined,
    profileOrAccount: Boolean(control?.profileOrAccount) || undefined,
    rect: control?.rect,
  };
}

export function sanitizeModelEvidence({ bodyText, controls, selectedText, selectedIndex, selectedScore, error }) {
  const selectedDigest = textDigestEvidence(selectedText);
  return {
    bodyTextLength: bodyText.length,
    bodyTextSha256: bodyText ? sha256Text(bodyText) : '',
    selectedTextLength: selectedDigest.length,
    selectedTextSha256: selectedDigest.sha256,
    selectedIndex,
    selectedScore: Number.isFinite(Number(selectedScore)) ? Number(selectedScore) : undefined,
    controls: controls.map((control, index) => sanitizeModelControlEvidence(control, index, selectedIndex, selectedScore)),
    error,
  };
}
