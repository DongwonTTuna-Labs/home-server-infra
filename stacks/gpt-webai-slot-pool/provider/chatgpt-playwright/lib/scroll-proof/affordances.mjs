import { DEFAULT_SCROLL_METRIC_EPSILON_PX, clampNumber } from './util.mjs';

const LEFT_SIDEBAR_AFFORDANCE_MAX_RIGHT_PX = 360;
const LEFT_SIDEBAR_AFFORDANCE_MAX_WIDTH_PX = 360;

export function selectedScrollRoot(observation = {}) {
  return observation?.selected || null;
}

function numericRectValue(rect = {}, keys = []) {
  for (const key of keys) {
    const value = Number(rect?.[key]);
    if (Number.isFinite(value)) return value;
  }
  return null;
}

function rectLeft(rect = {}) {
  return numericRectValue(rect, ['left', 'x']);
}

function rectWidth(rect = {}) {
  return numericRectValue(rect, ['width']);
}

function rectRight(rect = {}) {
  const right = numericRectValue(rect, ['right']);
  if (right !== null) return right;
  const left = rectLeft(rect);
  const width = rectWidth(rect);
  if (left !== null && width !== null) return left + width;
  return null;
}

function textFieldsForAffordance(affordance = {}) {
  return [
    affordance.textPreview,
    affordance.labelPreview,
    affordance.titlePreview,
    affordance.testid,
    affordance.selector,
    affordance.role,
    affordance.scope,
    affordance.ignoredReason,
  ]
    .filter(value => typeof value === 'string' && value.trim())
    .join(' ');
}

function isCenteredFloatingIcon(affordance = {}) {
  return affordance?.match?.centeredFloatingIcon === true;
}

function textIdentifiesSidebarOrNavigationAffordance(text = '', affordance = {}) {
  if (/history-item|sidebar|side-bar|navigation|nav-|conversation-options|open conversation options/i.test(text)) return true;
  if (/^(pin|open conversation options|archive|delete|rename)\b/i.test(String(affordance.labelPreview || ''))) return true;
  if (/^(pin|open conversation options|archive|delete|rename)\b/i.test(String(affordance.titlePreview || ''))) return true;
  return false;
}

function leftSidebarGeometry(affordance = {}, observation = {}) {
  if (isCenteredFloatingIcon(affordance)) return false;
  const rect = affordance.rect || {};
  const left = rectLeft(rect);
  const right = rectRight(rect);
  const width = rectWidth(rect);
  if (left === null || right === null) return false;

  const selectedRect = selectedScrollRoot(observation)?.rect || observation?.selectedRect || {};
  const selectedLeft = rectLeft(selectedRect);
  if (Number.isFinite(selectedLeft) && selectedLeft > 0 && right <= selectedLeft + 8) {
    return true;
  }

  const viewportWidth = numericRectValue(observation?.viewport || {}, ['width']);
  const maxRight = viewportWidth !== null && viewportWidth > 0 && viewportWidth < 700
    ? 96
    : LEFT_SIDEBAR_AFFORDANCE_MAX_RIGHT_PX;
  const narrowEnough = width === null || width <= LEFT_SIDEBAR_AFFORDANCE_MAX_WIDTH_PX;

  return narrowEnough && (
    (left <= 24 && right <= maxRight)
    || (left < 0 && right <= 96)
  );
}

function affordanceIgnoredByScope(affordance = {}, observation = {}) {
  if (!affordance) return true;
  if (affordance.ignoredReason) return true;
  if (affordance.scope && affordance.scope !== 'conversation') return true;
  if (affordance.inSidebar === true || affordance.inNavigation === true) return true;
  const text = textFieldsForAffordance(affordance);
  if (textIdentifiesSidebarOrNavigationAffordance(text, affordance) && !isCenteredFloatingIcon(affordance)) return true;
  if (leftSidebarGeometry(affordance, observation)) return true;

  const selectedRect = selectedScrollRoot(observation)?.rect || observation?.selectedRect || {};
  const selectedLeft = rectLeft(selectedRect);
  const selectedRight = rectRight(selectedRect);
  const selectedWidth = rectWidth(selectedRect);
  const rect = affordance.rect || {};
  const affRight = rectRight(rect);
  const affX = rectLeft(rect);
  if (Number.isFinite(selectedLeft) && selectedLeft > 0 && affRight !== null
    && affRight <= selectedLeft + 8 && !isCenteredFloatingIcon(affordance)) {
    return true;
  }
  if (Number.isFinite(selectedLeft) && selectedLeft > 0 && affX !== null
    && affX <= selectedLeft + 8 && !isCenteredFloatingIcon(affordance)) {
    return true;
  }
  if (selectedRight !== null && selectedWidth !== null && selectedWidth > 360 && affX !== null
    && affX < selectedRight - selectedWidth + 16 && !isCenteredFloatingIcon(affordance)) {
    return true;
  }
  return false;
}

function allMoreContentAffordances(observation = {}) {
  if (Array.isArray(observation.moreContentAffordances)) {
    return observation.moreContentAffordances.filter(Boolean);
  }
  if (Array.isArray(observation.moreContentAffordances?.samples)) {
    return observation.moreContentAffordances.samples.filter(Boolean);
  }
  return [];
}

export function visibleMoreContentAffordances(observation = {}) {
  return allMoreContentAffordances(observation)
    .filter(affordance => !affordanceIgnoredByScope(affordance, observation));
}

export function ignoredMoreContentAffordances(observation = {}) {
  const fromPrimary = allMoreContentAffordances(observation)
    .filter(affordance => affordanceIgnoredByScope(affordance, observation));
  const explicit = (Array.isArray(observation.ignoredMoreContentAffordances)
    ? observation.ignoredMoreContentAffordances
    : [])
    .filter(Boolean);
  return [...fromPrimary, ...explicit];
}

export function hasVisibleMoreContentAffordance(observation = {}) {
  return visibleMoreContentAffordances(observation).length > 0
    || (observation.status === 'more_content_affordance_visible'
      && allMoreContentAffordances(observation).some(affordance => !affordanceIgnoredByScope(affordance, observation)));
}

function bottomStatusAcceptable(observation = {}) {
  return observation.status === 'at_bottom'
    || (observation.status === 'more_content_affordance_visible'
      && visibleMoreContentAffordances(observation).length === 0);
}

const RIGHT_EDGE_SCROLLBAR_SELECTION_KINDS = new Set([
  'browser_viewport_scrollbar',
  'chatgpt_scroll_root_scrollbar',
]);

function selectionKindFor(selected = {}) {
  return selected?.selectionKind || selected?.visualScrollbarProof?.selectionKind || '';
}

export function selectedUsesRightEdgeScrollbar(selected = {}) {
  return RIGHT_EDGE_SCROLLBAR_SELECTION_KINDS.has(selectionKindFor(selected));
}

function selectedVisualScrollbarProof(selected = {}, observation = {}) {
  return observation.visualScrollbarProof || selected.visualScrollbarProof || {};
}

function rightEdgeDomScrollbarAtBottom(observation = {}) {
  const selected = selectedScrollRoot(observation);
  if (!selected || hasVisibleMoreContentAffordance(observation)) return false;
  const visual = selectedVisualScrollbarProof(selected, observation);
  return bottomStatusAcceptable(observation)
    && selected.atBottom === true
    && selectedUsesRightEdgeScrollbar(selected)
    && visual.status === 'right_edge_scrollbar_at_bottom';
}

function rightEdgeDomScrollbarReason(observation = {}, prefix = 'screenshot_time') {
  const selected = selectedScrollRoot(observation);
  if (!selected) return `${prefix}_scroll_root_missing`;
  if (hasVisibleMoreContentAffordance(observation)) return 'more_content_below_affordance_visible';
  if (!bottomStatusAcceptable(observation)) return `${prefix}_bottom_scroll_unverified`;
  if (selected.atBottom !== true) return `${prefix}_bottom_scroll_unverified`;
  if (!selectedUsesRightEdgeScrollbar(selected)) return `${prefix}_right_edge_scrollbar_selection_unverified`;
  const visual = selectedVisualScrollbarProof(selected, observation);
  if (visual.status !== 'right_edge_scrollbar_at_bottom') {
    return visual.reason || `${prefix}_right_edge_scrollbar_dom_unverified`;
  }
  return '';
}

export function visualScrollbarProofBottomAligned(visualScrollbarProof = {}) {
  return visualScrollbarProof?.status === 'right_edge_scrollbar_at_bottom'
    && visualScrollbarProof?.alignment?.status === 'bottom_aligned';
}

function sameText(leftValue, rightValue) {
  return String(leftValue || '') === String(rightValue || '');
}

export function scrollRootConsistencyReason(left = {}, right = {}, options = {}) {
  const epsilon = clampNumber(options.metricEpsilonPx, DEFAULT_SCROLL_METRIC_EPSILON_PX);
  const leftSelected = selectedScrollRoot(left);
  const rightSelected = selectedScrollRoot(right);
  if (!leftSelected || !rightSelected) return 'scroll_root_missing';
  if (!sameText(selectionKindFor(leftSelected), selectionKindFor(rightSelected))) {
    return 'scroll_root_selection_kind_inconsistent';
  }
  if (!sameText(leftSelected.rootKind, rightSelected.rootKind)) {
    return 'scroll_root_kind_inconsistent';
  }
  const leftRect = leftSelected.rect || {};
  const rightRect = rightSelected.rect || {};
  const sameRight = Math.abs(clampNumber(rectRight(leftRect)) - clampNumber(rectRight(rightRect))) <= epsilon;
  const sameWidth = Math.abs(clampNumber(rectWidth(leftRect)) - clampNumber(rectWidth(rightRect))) <= epsilon;
  const sameHeight = Math.abs(clampNumber(leftRect.height) - clampNumber(rightRect.height)) <= epsilon;
  const sameMax = Math.abs(clampNumber(leftSelected.maxScrollTop) - clampNumber(rightSelected.maxScrollTop)) <= Math.max(epsilon, 24);
  return sameRight && sameWidth && sameHeight && sameMax ? '' : 'scroll_root_metrics_inconsistent';
}

export function scrollRootConsistent(left = {}, right = {}, options = {}) {
  return scrollRootConsistencyReason(left, right, options) === '';
}

function selectedHasOverflowingConversationScrollbar(observation = {}, options = {}) {
  const epsilon = clampNumber(options.metricEpsilonPx, DEFAULT_SCROLL_METRIC_EPSILON_PX);
  const selected = selectedScrollRoot(observation);
  if (!selected) return false;
  return clampNumber(selected.maxScrollTop) > epsilon;
}

function noScrollableConversationOverflow(observation = {}, options = {}) {
  if (hasVisibleMoreContentAffordance(observation)) return false;
  const epsilon = clampNumber(options.metricEpsilonPx, DEFAULT_SCROLL_METRIC_EPSILON_PX);
  const selected = selectedScrollRoot(observation);
  if (!selected) {
    return ['scrollport_not_found', 'no_scrollable_conversation_overflow', 'at_bottom', 'more_content_affordance_visible']
      .includes(observation.status)
      && clampNumber(observation.candidateCount) === 0;
  }
  return clampNumber(selected.maxScrollTop) <= epsilon && selected.atBottom !== false;
}

function rightEdgeCropHasNoVisibleScrollbar(visualScrollbarProof = {}) {
  return visualScrollbarProof?.status === 'unavailable'
    && visualScrollbarProof?.reason === 'scrollbar_thumb_not_found_in_right_edge_crop';
}

function bottomReadinessEvidenceVerified(evidence = {}) {
  if (evidence?.status !== 'verified') return false;
  if (evidence.sessionUrlMatches === false) return false;
  return evidence.authenticatedComposerReadyAtBottom === true
    || evidence.activeGenerationAtBottom === true
    || evidence.newestTurnAtBottom === true;
}

function bottomReadinessEvidenceReason(evidence = {}) {
  if (evidence?.status !== 'verified') return evidence?.reason || 'bottom_readiness_evidence_missing';
  if (evidence.sessionUrlMatches === false) return 'bottom_readiness_session_url_mismatch';
  return 'bottom_readiness_evidence_missing';
}

export function buildNoScrollableConversationOverflowProof({
  screenshotObservation = {},
  domObservation = {},
  visualScrollbarProof = {},
  bottomReadinessEvidence = {},
  options = {},
} = {}) {
  const screenshotNoOverflow = noScrollableConversationOverflow(screenshotObservation, options);
  const domNoOverflow = noScrollableConversationOverflow(domObservation, options);
  const screenshotHasOverflow = selectedHasOverflowingConversationScrollbar(screenshotObservation, options);
  const domHasOverflow = selectedHasOverflowingConversationScrollbar(domObservation, options);
  const noVisibleRightEdgeScrollbar = rightEdgeCropHasNoVisibleScrollbar(visualScrollbarProof);
  const readinessVerified = bottomReadinessEvidenceVerified(bottomReadinessEvidence);
  let reason = '';
  if (screenshotHasOverflow || domHasOverflow) reason = 'conversation_scrollbar_overflow_requires_right_edge_pixel_proof';
  else if (!screenshotNoOverflow) reason = 'screenshot_time_no_overflow_unverified';
  else if (!domNoOverflow) reason = 'dom_time_no_overflow_unverified';
  else if (!noVisibleRightEdgeScrollbar) reason = 'right_edge_scrollbar_absence_unverified';
  else if (!readinessVerified) reason = bottomReadinessEvidenceReason(bottomReadinessEvidence);

  return {
    status: reason ? 'unverified' : 'verified',
    reason: reason || undefined,
    method: 'dom_short_conversation_no_scrollbar',
    observations: {
      screenshot: screenshotNoOverflow ? 'no_scrollable_overflow' : 'unverified',
      dom: domNoOverflow ? 'no_scrollable_overflow' : 'unverified',
      rightEdgeScrollbar: noVisibleRightEdgeScrollbar ? 'no_visible_scrollbar' : 'unverified',
    },
    bottomReadinessEvidence,
  };
}


export function buildVisibleRightEdgeScrollbarProof({
  screenshotObservation = {},
  domObservation = {},
  visualScrollbarProof = {},
  consistencyReason = '',
} = {}) {
  const screenshotVerified = rightEdgeDomScrollbarAtBottom(screenshotObservation);
  const domVerified = rightEdgeDomScrollbarAtBottom(domObservation);
  const pixelVerified = visualScrollbarProofBottomAligned(visualScrollbarProof);
  let reason = '';
  if (!pixelVerified) reason = visualScrollbarProof?.reason || 'right_edge_scrollbar_pixel_unverified';
  else if (!screenshotVerified) reason = rightEdgeDomScrollbarReason(screenshotObservation, 'screenshot_time');
  else if (!domVerified) reason = rightEdgeDomScrollbarReason(domObservation, 'dom_time');
  else if (consistencyReason) reason = consistencyReason;

  return {
    status: reason ? 'unverified' : 'verified',
    reason: reason || undefined,
    method: 'strict_visible_right_edge_scrollbar',
    observations: {
      screenshot: screenshotVerified ? 'right_edge_scrollbar_at_bottom' : 'unverified',
      dom: domVerified ? 'right_edge_scrollbar_at_bottom' : 'unverified',
      pixel: pixelVerified ? 'right_edge_scrollbar_at_bottom' : 'unverified',
    },
  };
}

function shortNoScrollbarModeCandidate(noScrollableConversationOverflowProof = {}, visualScrollbarProof = {}) {
  return (noScrollableConversationOverflowProof?.observations?.screenshot === 'no_scrollable_overflow'
    && noScrollableConversationOverflowProof?.observations?.dom === 'no_scrollable_overflow'
    && rightEdgeCropHasNoVisibleScrollbar(visualScrollbarProof));
}

function unavailableRightEdgeCropReason(visualScrollbarProof = {}) {
  if (visualScrollbarProof?.status !== 'unavailable') return '';
  if (!visualScrollbarProof?.reason) return '';
  return rightEdgeCropHasNoVisibleScrollbar(visualScrollbarProof) ? '' : visualScrollbarProof.reason;
}

export function proofReason({
  fullViewportScreenshot,
  rightEdgeScrollbarCrop,
  screenshotObservation,
  domObservation,
  visualScrollbarProof,
  visibleRightEdgeScrollbarProof,
  noScrollableConversationOverflowProof,
}) {
  if (fullViewportScreenshot?.status !== 'saved') return 'full_viewport_screenshot_missing';
  if (rightEdgeScrollbarCrop?.status !== 'saved') return 'right_edge_scrollbar_crop_missing';
  if (hasVisibleMoreContentAffordance(screenshotObservation) || hasVisibleMoreContentAffordance(domObservation)) {
    return 'more_content_below_affordance_visible';
  }
  if (visibleRightEdgeScrollbarProof?.status === 'verified') return '';
  if (noScrollableConversationOverflowProof?.status === 'verified') return '';
  if (visualScrollbarProofBottomAligned(visualScrollbarProof)) {
    return visibleRightEdgeScrollbarProof?.reason || 'visible_right_edge_scrollbar_unverified';
  }
  const cropReason = unavailableRightEdgeCropReason(visualScrollbarProof);
  if (cropReason) return cropReason;
  if (shortNoScrollbarModeCandidate(noScrollableConversationOverflowProof, visualScrollbarProof)) {
    return noScrollableConversationOverflowProof.reason || 'short_conversation_no_overflow_unverified';
  }
  if (visibleRightEdgeScrollbarProof?.reason) return visibleRightEdgeScrollbarProof.reason;
  if (visualScrollbarProof?.status !== 'right_edge_scrollbar_at_bottom') {
    return visualScrollbarProof?.reason || 'right_edge_scrollbar_pixel_unverified';
  }
  if (visualScrollbarProof?.alignment?.status !== 'bottom_aligned') {
    return 'right_edge_scrollbar_thumb_not_bottom_aligned';
  }
  return noScrollableConversationOverflowProof?.reason || 'scroll.bottom_unverified';
}
