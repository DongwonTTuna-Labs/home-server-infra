import {
  buildNoScrollableConversationOverflowProof,
  buildVisibleRightEdgeScrollbarProof,
  proofReason,
  scrollRootConsistencyReason,
  selectedScrollRoot,
} from './affordances.mjs';
import {
  summarizeBottomReadinessEvidence,
  summarizeBottomScrollObservation,
  summarizeIgnoredMoreContentAffordances,
  summarizeMoreContentAffordances,
  summarizeSelected,
} from './summaries.mjs';

export function buildScrollBottomProof({
  label = '',
  fullViewportScreenshot = {},
  rightEdgeScrollbarCrop = {},
  screenshotObservation = {},
  domObservation = {},
  visualScrollbarProof = {},
  bottomReadinessEvidence = {},
  options = {},
} = {}) {
  const bottomReadinessEvidenceSummary = summarizeBottomReadinessEvidence(bottomReadinessEvidence);
  const noScrollableConversationOverflowProof = buildNoScrollableConversationOverflowProof({
    screenshotObservation,
    domObservation,
    visualScrollbarProof,
    bottomReadinessEvidence: bottomReadinessEvidenceSummary,
    options,
  });
  const consistencyReason = noScrollableConversationOverflowProof.status === 'verified'
    ? ''
    : scrollRootConsistencyReason(screenshotObservation, domObservation, options);
  const visibleRightEdgeScrollbarProof = buildVisibleRightEdgeScrollbarProof({
    screenshotObservation,
    domObservation,
    visualScrollbarProof,
    consistencyReason,
  });
  const reason = proofReason({
    fullViewportScreenshot,
    rightEdgeScrollbarCrop,
    screenshotObservation,
    domObservation,
    visualScrollbarProof,
    visibleRightEdgeScrollbarProof,
    noScrollableConversationOverflowProof,
  });
  const verificationMode = !reason && visibleRightEdgeScrollbarProof.status === 'verified'
    ? 'strict_visible_right_edge_scrollbar'
    : !reason && noScrollableConversationOverflowProof.status === 'verified'
      ? 'strict_short_no_scrollbar'
      : undefined;
  return {
    schema: 'gpt-webai.scroll-bottom-proof.v1',
    label: label || undefined,
    status: reason ? 'unverified' : 'verified',
    reason: reason || undefined,
    verificationMode,
    fullViewportScreenshot,
    rightEdgeScrollbarCrop,
    screenshotObservation: summarizeBottomScrollObservation(screenshotObservation),
    domObservation: summarizeBottomScrollObservation(domObservation),
    visualScrollbarProof,
    visibleRightEdgeScrollbarProof,
    noScrollableConversationOverflowProof,
    bottomReadinessEvidence: bottomReadinessEvidenceSummary,
    moreContentAffordances: summarizeMoreContentAffordances(screenshotObservation, domObservation),
    ignoredMoreContentAffordances: summarizeIgnoredMoreContentAffordances(screenshotObservation, domObservation),
    consistency: {
      status: consistencyReason ? 'mismatch' : 'consistent',
      reason: consistencyReason || undefined,
      screenshotSelected: summarizeSelected(selectedScrollRoot(screenshotObservation)),
      domSelected: summarizeSelected(selectedScrollRoot(domObservation)),
    },
  };
}
