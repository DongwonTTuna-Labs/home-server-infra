
import { hasProModelEvidence } from '../readiness.mjs';

export const MODEL_SURFACE_RE = /\b(?:GPT(?:[-\s]?[45])?|Pro(?:\s+Extended)?|Model|Thinking|Reason(?:ing)?|Extra\s+High|High|Auto|Standard|Fast)\b|모델|프로/i;
const REQUESTED_PRO_EXTENDED_RE = /\bPro\s+Extended\b/i;
const PROFILE_OR_ACCOUNT_RE = /account|profile|avatar|workspace|user-menu|accounts-profile/i;
const SIDEBAR_OR_HISTORY_RE = /sidebar|history|nav(?:igation)?|conversation-options/i;
const NON_MODEL_COMPOSER_CONTROL_RE = /\b(?:send|submit|stop|voice|microphone|mic|attach|attachment|file|upload|add|plus|follow up)\b|전송|중지|첨부/i;

function textDigestEvidence(value) {
  const text = String(value || '');
  return {
    length: text.length,
    sha256: text ? sha256Text(text) : '',
  };
}

export function controlSignal(control = {}) {
  return [control.text, control.label, control.title, control.testid, control.role]
    .map(value => String(value || ''))
    .filter(Boolean)
    .join(' ')
    .trim();
}

export function controlTextSignal(control = {}) {
  return [control.text, control.label, control.title]
    .map(value => String(value || ''))
    .filter(Boolean)
    .join(' ')
    .trim();
}

function controlIsProfileOrAccount(control = {}) {
  return Boolean(control.profileOrAccount || PROFILE_OR_ACCOUNT_RE.test(controlSignal(control)));
}

function controlIsSidebarOrHistory(control = {}) {
  return Boolean(control.sidebarOrNav || SIDEBAR_OR_HISTORY_RE.test(controlSignal(control)));
}

function controlMatchesModelSurface(control = {}) {
  return Boolean(control.matchesModelSurface || MODEL_SURFACE_RE.test(controlSignal(control)));
}

function modelControlScore(control = {}) {
  if (controlIsProfileOrAccount(control) || controlIsSidebarOrHistory(control)) return -10_000;
  if (!controlMatchesModelSurface(control)) return -1_000;
  if (NON_MODEL_COMPOSER_CONTROL_RE.test(controlSignal(control)) && !/\b(?:GPT|Pro|Model|Extra\s+High|High|Auto)\b/i.test(controlSignal(control))) {
    return -500;
  }
  let score = 0;
  if (control.inComposerRoot) score += 1_000;
  if (control.nearComposer) score += 800;
  if (control.bottomComposerBand) score += 250;
  if (/model/i.test(String(control.testid || ''))) score += 280;
  if (/model|GPT|Pro|Thinking|Reason|모델/i.test(String(control.label || ''))) score += 180;
  if (REQUESTED_PRO_EXTENDED_RE.test(controlTextSignal(control))) score += 260;
  if (/\bExtra\s+High\b/i.test(controlTextSignal(control))) score += 230;
  if (String(control.text || '').trim()) score += 90;
  const rect = control.rect || {};
  if (Number.isFinite(Number(rect.y)) && Number.isFinite(Number(control.viewportHeight)) && Number(control.viewportHeight) > 0) {
    score += Math.max(0, Math.round((Number(rect.y) / Number(control.viewportHeight)) * 80));
  }
  return score;
}

export function selectedModelControl(controls = []) {
  let best = null;
  let bestScore = -Infinity;
  for (const control of controls) {
    const score = modelControlScore(control);
    if (score > bestScore) {
      best = control;
      bestScore = score;
    }
  }
  if (!best || bestScore < 0) return { control: null, index: -1, score: bestScore };
  const index = controls.indexOf(best);
  return { control: best, index, score: bestScore };
}

export function requestedProExtended(model, effort) {
  return String(model || '').toLowerCase() === 'pro'
    && String(effort || '').toLowerCase() === 'extended';
}

export function selectedMatchesRequestedModel({ selectedText = '', model = '', effort = '' } = {}) {
  if (String(model || '').toLowerCase() === 'pro') {
    // `effort=extended` is an effort hint, not a distinct picker product in
    // the current ChatGPT UI. A visible Pro control is the requested model
    // proof; do not require a non-existent `Pro Extended` label.
    return hasProModelEvidence({ selectedText });
  }
  return true;
}
