import { clampNumber } from './util.mjs';
import {
  ignoredMoreContentAffordances,
  selectedScrollRoot,
  selectedUsesRightEdgeScrollbar,
  visibleMoreContentAffordances,
  visualScrollbarProofBottomAligned,
} from './affordances.mjs';

export function summarizeSelected(selected = null) {
  if (!selected) return null;
  return {
    tag: selected.tag,
    id: selected.id,
    testid: selected.testid,
    selectorHint: selected.selectorHint,
    selectionKind: selected.selectionKind,
    rootKind: selected.rootKind,
    rect: selected.rect,
    scrollTop: selected.scrollTop,
    maxScrollTop: selected.maxScrollTop,
    scrollHeight: selected.scrollHeight,
    clientHeight: selected.clientHeight,
    atBottom: selected.atBottom,
    visualScrollbarProof: selected.visualScrollbarProof,
  };
}

function summarizeAffordance(affordance = {}) {
  return {
    index: affordance.index,
    tag: affordance.tag,
    role: affordance.role,
    testid: affordance.testid,
    scope: affordance.scope,
    ignoredReason: affordance.ignoredReason,
    textPreview: affordance.textPreview,
    labelPreview: affordance.labelPreview,
    titlePreview: affordance.titlePreview,
    rect: affordance.rect,
    match: affordance.match,
  };
}

function summarizeRectEvidence(value = null) {
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

export function summarizeBottomReadinessEvidence(evidence = {}) {
  if (!evidence || typeof evidence !== 'object' || Object.keys(evidence).length === 0) return {};
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
    evidenceKinds: Array.isArray(evidence.evidenceKinds) ? evidence.evidenceKinds.slice(0, 8) : undefined,
    viewport: evidence.viewport,
    composer: summarizeRectEvidence(evidence.composer),
    activeGenerationControl: summarizeRectEvidence(evidence.activeGenerationControl),
    newestTurn: evidence.newestTurn ? {
      kind: evidence.newestTurn.kind,
      visible: evidence.newestTurn.visible,
      nearBottom: evidence.newestTurn.nearBottom,
      bottomGapPx: evidence.newestTurn.bottomGapPx,
      rect: evidence.newestTurn.rect,
      textLength: evidence.newestTurn.textLength,
      textSha256: evidence.newestTurn.textSha256,
    } : undefined,
  };
}

export function summarizeMoreContentAffordances(...observations) {
  const samples = observations
    .flatMap(observation => visibleMoreContentAffordances(observation).map(summarizeAffordance))
    .slice(0, 8);
  return {
    status: samples.length > 0 ? 'visible' : 'clear',
    count: samples.length,
    samples,
  };
}

export function summarizeIgnoredMoreContentAffordances(...observations) {
  const samples = observations
    .flatMap(observation => ignoredMoreContentAffordances(observation).map(summarizeAffordance))
    .slice(0, 8);
  return {
    status: samples.length > 0 ? 'ignored' : 'clear',
    count: samples.length,
    samples,
  };
}

export function summarizeBottomScrollObservation(observation = {}) {
  const affordances = visibleMoreContentAffordances(observation).map(summarizeAffordance);
  const ignoredAffordances = ignoredMoreContentAffordances(observation).map(summarizeAffordance);
  return {
    schema: observation.schema,
    label: observation.label,
    status: observation.status,
    attempts: observation.attempts,
    candidateCount: observation.candidateCount,
    viewport: observation.viewport,
    selected: summarizeSelected(selectedScrollRoot(observation)),
    visualScrollbarProof: observation.visualScrollbarProof || selectedScrollRoot(observation)?.visualScrollbarProof,
    moreContentAffordances: {
      status: affordances.length > 0 ? 'visible' : 'clear',
      count: affordances.length,
      samples: affordances.slice(0, 8),
    },
    ignoredMoreContentAffordances: {
      status: ignoredAffordances.length > 0 ? 'ignored' : 'clear',
      count: ignoredAffordances.length,
      samples: ignoredAffordances.slice(0, 8),
    },
  };
}

function proofHasVisibleMoreContentAffordance(proof = {}) {
  const affordances = proof?.moreContentAffordances || {};
  return affordances.status === 'visible'
    || clampNumber(affordances.count) > 0
    || (Array.isArray(affordances.samples) && affordances.samples.length > 0);
}


function visibleRightEdgeScrollbarProofVerified(proof = {}) {
  const visible = proof?.visibleRightEdgeScrollbarProof || {};
  if (proof?.verificationMode !== 'strict_visible_right_edge_scrollbar') return false;
  if (visible.status !== 'verified') return false;
  if (visible.method !== 'strict_visible_right_edge_scrollbar') return false;
  if (visible.observations?.screenshot !== 'right_edge_scrollbar_at_bottom') return false;
  if (visible.observations?.dom !== 'right_edge_scrollbar_at_bottom') return false;
  if (visible.observations?.pixel !== 'right_edge_scrollbar_at_bottom') return false;
  if (!visualScrollbarProofBottomAligned(proof?.visualScrollbarProof)) return false;
  if (proof?.consistency?.status !== 'consistent') return false;
  if (!selectedUsesRightEdgeScrollbar(proof?.consistency?.screenshotSelected)) return false;
  if (!selectedUsesRightEdgeScrollbar(proof?.consistency?.domSelected)) return false;
  return true;
}

function noScrollableConversationOverflowProofVerified(proof = {}) {
  const noOverflow = proof?.noScrollableConversationOverflowProof || {};
  const readiness = noOverflow.bottomReadinessEvidence || proof?.bottomReadinessEvidence || {};
  if (proof?.verificationMode !== 'strict_short_no_scrollbar') return false;
  if (noOverflow.status !== 'verified') return false;
  if (noOverflow.method !== 'dom_short_conversation_no_scrollbar') return false;
  if (noOverflow.observations?.screenshot !== 'no_scrollable_overflow') return false;
  if (noOverflow.observations?.dom !== 'no_scrollable_overflow') return false;
  if (noOverflow.observations?.rightEdgeScrollbar !== 'no_visible_scrollbar') return false;
  if (proof?.visualScrollbarProof?.status !== 'unavailable') return false;
  if (proof?.visualScrollbarProof?.reason !== 'scrollbar_thumb_not_found_in_right_edge_crop') return false;
  if (readiness.status !== 'verified') return false;
  if (readiness.sessionUrlMatches === false) return false;
  return readiness.authenticatedComposerReadyAtBottom === true
    || readiness.activeGenerationAtBottom === true
    || readiness.newestTurnAtBottom === true;
}

export function scrollBottomProofVerified(diagnostic = {}) {
  const proof = diagnostic?.scrollBottomProof || {};
  if (proof.status !== 'verified') return false;
  if (proof.fullViewportScreenshot?.status !== 'saved') return false;
  if (proof.rightEdgeScrollbarCrop?.status !== 'saved') return false;
  if (proofHasVisibleMoreContentAffordance(proof)) return false;
  if (visibleRightEdgeScrollbarProofVerified(proof)) return true;
  return noScrollableConversationOverflowProofVerified(proof);
}

export function scrollBottomProofReason(diagnostic = {}) {
  const proof = diagnostic?.scrollBottomProof || {};
  if (proof.status === 'verified') return '';
  return proof.reason || 'scroll.bottom_unverified';
}
