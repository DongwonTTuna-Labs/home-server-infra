import { sha256Text } from '../common.mjs';

function digest(value) {
  const text = String(value || '');
  return {
    length: text.length,
    sha256: text ? sha256Text(text) : '',
  };
}

const PROVIDER_LIMIT_RE = /too many requests|request limit|rate limit|usage limit|temporarily limited|message cap|try again later|you(?:'|’| have)?ve? reached (?:the )?.{0,80}limit/i;

function sanitizeTurn(turn = {}) {
  const text = String(turn.text || '');
  const domId = String(turn.domId || '');
  return {
    index: turn.index,
    tag: turn.tag,
    domIdLength: domId.length,
    domIdSha256: domId ? sha256Text(domId) : '',
    textLength: text.length,
    textSha256: text ? sha256Text(text) : '',
    rect: turn.rect,
  };
}

function sanitizeBottomRectEvidence(value = null) {
  if (!value || typeof value !== 'object') return value || undefined;
  return {
    visible: value.visible,
    nearBottom: value.nearBottom,
    bottomGapPx: value.bottomGapPx,
    rect: value.rect,
    count: value.count,
    disabled: value.disabled,
  };
}

function sanitizeBottomReadinessEvidence(evidence = {}) {
  if (!evidence || typeof evidence !== 'object') return {};
  const newestText = String(evidence.newestTurn?.text || '');
  return {
    schema: evidence.schema,
    label: evidence.label,
    status: evidence.status,
    reason: evidence.reason,
    urlKind: evidence.urlKind,
    sessionIdPresent: evidence.sessionIdPresent,
    sessionUrlMatches: evidence.sessionUrlMatches,
    authenticatedComposerReadyAtBottom: evidence.authenticatedComposerReadyAtBottom,
    activeGenerationAtBottom: evidence.activeGenerationAtBottom,
    newestTurnAtBottom: evidence.newestTurnAtBottom,
    evidenceKinds: Array.isArray(evidence.evidenceKinds) ? evidence.evidenceKinds.slice(0, 8) : [],
    viewport: evidence.viewport,
    composer: sanitizeBottomRectEvidence(evidence.composer),
    activeGenerationControl: sanitizeBottomRectEvidence(evidence.activeGenerationControl),
    newestTurn: evidence.newestTurn ? {
      kind: evidence.newestTurn.kind,
      visible: evidence.newestTurn.visible,
      nearBottom: evidence.newestTurn.nearBottom,
      bottomGapPx: evidence.newestTurn.bottomGapPx,
      rect: evidence.newestTurn.rect,
      textLength: newestText.length || evidence.newestTurn.textLength || 0,
      textSha256: newestText ? sha256Text(newestText) : '',
    } : undefined,
  };
}

export function sanitizeDomDiagnostics(raw, bottomScroll) {
  return {
    schema: raw.schema,
    capturedAt: raw.capturedAt,
    label: raw.label,
    sessionId: raw.sessionId,
    url: raw.url,
    title: digest(raw.title || ''),
    bottomScroll,
    bodyText: digest(raw.bodyTextPreview || ''),
    readinessSignals: sanitizeReadinessSignals(raw.readinessSignals),
    selectorInventory: sanitizeSelectorInventory(raw.selectorInventory),
    controls: (raw.controls || []).map(control => ({
      index: control.index,
      tag: control.tag,
      role: control.role,
      type: control.type,
      testIdLength: String(control.testid || '').length,
      testIdSha256: control.testid ? sha256Text(control.testid) : '',
      textLength: String(control.text || '').length,
      textSha256: control.text ? sha256Text(control.text) : '',
      labelLength: String(control.label || '').length,
      labelSha256: control.label ? sha256Text(control.label) : '',
      titleLength: String(control.title || '').length,
      titleSha256: control.title ? sha256Text(control.title) : '',
      rect: control.rect,
      disabled: control.disabled,
    })),
    dialogs: (raw.dialogs || []).map(dialog => ({
      index: dialog.index,
      tag: dialog.tag,
      role: dialog.role,
      classNameLength: String(dialog.className || '').length,
      classNameSha256: dialog.className ? sha256Text(dialog.className) : '',
      textLength: String(dialog.text || '').length,
      textSha256: dialog.text ? sha256Text(dialog.text) : '',
      providerLimitMatched: PROVIDER_LIMIT_RE.test(String(dialog.text || '')),
      rect: dialog.rect,
    })),
    providerLimitSurfaces: (raw.providerLimitSurfaces || []).map(surface => ({
      index: surface.index,
      tag: surface.tag,
      role: surface.role,
      kind: surface.kind,
      classNameLength: String(surface.className || '').length,
      classNameSha256: surface.className ? sha256Text(surface.className) : '',
      textLength: String(surface.text || '').length,
      textSha256: surface.text ? sha256Text(surface.text) : '',
      providerLimitMatched: PROVIDER_LIMIT_RE.test(String(surface.text || '')),
      rect: surface.rect,
      actionButtons: (surface.actionButtons || []).map(button => ({
        index: button.index,
        tag: button.tag,
        role: button.role,
        textLength: String(button.text || '').length,
        textSha256: button.text ? sha256Text(button.text) : '',
        labelLength: String(button.label || '').length,
        labelSha256: button.label ? sha256Text(button.label) : '',
        rect: button.rect,
      })),
    })),
    assistantTurns: (raw.assistantTurns || []).map(turn => sanitizeTurn(turn)),
    userTurns: (raw.userTurns || []).map(turn => sanitizeTurn(turn)),
    bottomReadinessEvidence: sanitizeBottomReadinessEvidence(raw.bottomReadinessEvidence),
  };
}

function sanitizeReadinessSignals(value = {}) {
  return {
    login: value.login === true,
    limit: value.limit === true,
    upgrade: value.upgrade === true,
    pro: value.pro === true,
    composer: value.composer === true,
    stopControls: safeCount(value.stopControls),
    dialogs: safeCount(value.dialogs),
    providerLimitSurfaceCount: safeCount(value.providerLimitSurfaceCount),
    fileInputs: safeCount(value.fileInputs),
    textboxes: safeCount(value.textboxes),
  };
}

function sanitizeSelectorInventory(value = {}) {
  return {
    controls: safeCount(value.controls),
    dialogs: safeCount(value.dialogs),
    fileInputs: safeCount(value.fileInputs),
    textboxes: safeCount(value.textboxes),
    assistantTurns: safeCount(value.assistantTurns),
    userTurns: safeCount(value.userTurns),
  };
}

function safeCount(value) {
  return Number.isInteger(value) && value >= 0 ? value : 0;
}
