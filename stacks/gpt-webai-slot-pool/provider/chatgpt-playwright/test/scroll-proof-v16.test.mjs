import test from 'node:test';
import assert from 'node:assert/strict';

import {
  analyzeRightEdgeScrollbarPixels,
  buildScrollBottomProof,
  scrollBottomProofVerified,
} from '../lib/scroll-proof.mjs';

function decodedCrop({ height = 720, thumbStart = 288, thumbEnd = 684, bottomCapStart = 691 } = {}) {
  const width = 4;
  const rows = [];
  for (let y = 0; y < height; y += 1) {
    const dark = (y >= thumbStart && y <= thumbEnd) || (y >= bottomCapStart && y <= bottomCapStart + 4);
    rows.push(Buffer.alloc(width * 3, dark ? 160 : 245));
  }
  return { width, height, colorType: 2, rows };
}

function clippedContentCrop({ height = 703 } = {}) {
  const width = 24;
  const rows = [];
  for (let y = 0; y < height; y += 1) {
    const row = Buffer.alloc(width * 3, 245);
    if (y >= 18 && y <= 525) {
      for (const [start, end] of [[0, 8], [12, 20]]) {
        for (let x = start; x <= end; x += 1) {
          row[x * 3] = 160;
          row[x * 3 + 1] = 160;
          row[x * 3 + 2] = 160;
        }
      }
    }
    rows.push(row);
  }
  return { width, height, colorType: 2, rows };
}

function savedScreens() {
  return {
    fullViewportScreenshot: { status: 'saved', path: '/diagnostics/poll-terminal.png', width: 1020, height: 703 },
    rightEdgeScrollbarCrop: {
      status: 'saved',
      path: '/diagnostics/poll-terminal.right-edge-scrollbar.png',
      width: 24,
      height: 703,
    },
  };
}

function tinyRightEdgeObservation(label, overrides = {}) {
  const selectionKind = overrides.selectionKind || 'chatgpt_scroll_root_scrollbar';
  const selected = {
    tag: 'div',
    id: 'chatgpt-scroll-root',
    testid: '',
    selectorHint: '[class*="scroll-root"][class*="scrollbar"]',
    selectionKind,
    rootKind: overrides.rootKind,
    rect: overrides.rect || { x: 52, y: 0, left: 52, top: 0, right: 1020, bottom: 703, width: 968, height: 703 },
    scrollTop: 8,
    maxScrollTop: 8,
    scrollHeight: 711,
    clientHeight: 703,
    atBottom: true,
    visualScrollbarProof: {
      status: 'right_edge_scrollbar_at_bottom',
      method: 'dom_scroll_metrics_and_right_edge_scroll_root_scrollbar',
      selectionKind,
      rootKind: overrides.rootKind,
      rightEdgeDeltaPx: 0,
      requiredRightEdgeDeltaPx: 12,
      classContainsScrollRoot: true,
      classContainsScrollbar: true,
      scrollTop: 8,
      maxScrollTop: 8,
      atBottom: true,
    },
    ...overrides.selected,
  };
  return {
    schema: 'gpt-webai.bottom-scroll-gate.v1',
    label,
    status: 'at_bottom',
    attempts: 4,
    candidateCount: 1,
    viewport: { width: 1020, height: 703 },
    selected,
    visualScrollbarProof: selected.visualScrollbarProof,
    moreContentAffordances: overrides.moreContentAffordances || [],
    ignoredMoreContentAffordances: overrides.ignoredMoreContentAffordances || [],
  };
}

function terminalAnswerReadiness(overrides = {}) {
  return {
    schema: 'gpt-webai.bottom-readiness-evidence.v1',
    label: 'poll-terminal',
    status: 'verified',
    urlKind: 'conversation',
    sessionIdPresent: true,
    sessionUrlMatches: true,
    authenticatedComposerReadyAtBottom: false,
    activeGenerationAtBottom: false,
    newestTurnAtBottom: true,
    evidenceKinds: ['newest_turn_at_bottom'],
    viewport: { width: 1020, height: 703 },
    newestTurn: {
      kind: 'assistant',
      visible: true,
      nearBottom: true,
      bottomGapPx: 38,
      rect: { x: 230, y: 354, left: 230, top: 354, right: 872, bottom: 665, width: 642, height: 311 },
      textLength: 1880,
      textSha256: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
    },
    ...overrides,
  };
}

function noScrollbarVisualProof() {
  return {
    status: 'unavailable',
    reason: 'scrollbar_thumb_not_found_in_right_edge_crop',
    method: 'right_edge_crop_pixel_scan',
    crop: { width: 24, height: 703 },
    segments: [],
    alignment: { status: 'unavailable' },
  };
}

function noScrollableObservation(label, affordances = []) {
  return {
    schema: 'gpt-webai.bottom-scroll-gate.v1',
    label,
    status: 'scrollport_not_found',
    attempts: 4,
    candidateCount: 0,
    viewport: { width: 1020, height: 703 },
    selected: null,
    visualScrollbarProof: {
      status: 'unavailable',
      reason: 'scrollport_not_found',
      method: 'dom_scroll_metrics_and_right_edge_scroll_root_scrollbar',
    },
    moreContentAffordances: affordances,
  };
}

test('v16 verifies the exact tiny-range visible right-edge scrollbar bottom proof shape', () => {
  const proof = buildScrollBottomProof({
    label: 'poll-terminal',
    ...savedScreens(),
    screenshotObservation: tinyRightEdgeObservation('poll-terminal:screenshot'),
    domObservation: tinyRightEdgeObservation('poll-terminal:dom'),
    visualScrollbarProof: analyzeRightEdgeScrollbarPixels(decodedCrop({ thumbEnd: 684, bottomCapStart: 691 })),
    bottomReadinessEvidence: terminalAnswerReadiness(),
  });

  assert.equal(proof.status, 'verified');
  assert.equal(proof.reason, undefined);
  assert.equal(proof.verificationMode, 'strict_visible_right_edge_scrollbar');
  assert.equal(proof.visibleRightEdgeScrollbarProof.status, 'verified');
  assert.equal(proof.noScrollableConversationOverflowProof.status, 'unverified');
  assert.equal(proof.noScrollableConversationOverflowProof.reason, 'right_edge_scrollbar_absence_unverified');
  assert.equal(proof.consistency.status, 'consistent');
  assert.equal(proof.consistency.screenshotSelected.selectionKind, 'chatgpt_scroll_root_scrollbar');
  assert.equal(proof.consistency.screenshotSelected.maxScrollTop, 8);
  assert.equal(proof.consistency.screenshotSelected.scrollTop, 8);
  assert.equal(proof.consistency.screenshotSelected.atBottom, true);
  assert.equal(proof.moreContentAffordances.status, 'clear');
  assert.equal(proof.bottomReadinessEvidence.newestTurnAtBottom, true);
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), true);
});

test('v16 visible right-edge mode rejects a bottom-gap crop', () => {
  const proof = buildScrollBottomProof({
    label: 'poll-terminal',
    ...savedScreens(),
    screenshotObservation: tinyRightEdgeObservation('poll-terminal:screenshot'),
    domObservation: tinyRightEdgeObservation('poll-terminal:dom'),
    visualScrollbarProof: analyzeRightEdgeScrollbarPixels(decodedCrop({ thumbStart: 157, thumbEnd: 552, bottomCapStart: 691 })),
    bottomReadinessEvidence: terminalAnswerReadiness(),
  });

  assert.equal(proof.status, 'unverified');
  assert.equal(proof.reason, 'right_edge_scrollbar_thumb_bottom_gap');
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), false);
});

test('v16 visible right-edge mode rejects a missing crop even when DOM metrics are at bottom', () => {
  const proof = buildScrollBottomProof({
    label: 'poll-terminal',
    fullViewportScreenshot: savedScreens().fullViewportScreenshot,
    rightEdgeScrollbarCrop: { status: 'failed', path: '/diagnostics/missing.right-edge-scrollbar.png' },
    screenshotObservation: tinyRightEdgeObservation('poll-terminal:screenshot'),
    domObservation: tinyRightEdgeObservation('poll-terminal:dom'),
    visualScrollbarProof: { status: 'right_edge_scrollbar_at_bottom', alignment: { status: 'bottom_aligned' } },
    bottomReadinessEvidence: terminalAnswerReadiness(),
  });

  assert.equal(proof.status, 'unverified');
  assert.equal(proof.reason, 'right_edge_scrollbar_crop_missing');
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), false);
});

test('v16 visible right-edge mode rejects clipped-content crops as pixel proof', () => {
  const proof = buildScrollBottomProof({
    label: 'poll-terminal',
    ...savedScreens(),
    screenshotObservation: tinyRightEdgeObservation('poll-terminal:screenshot'),
    domObservation: tinyRightEdgeObservation('poll-terminal:dom'),
    visualScrollbarProof: analyzeRightEdgeScrollbarPixels(clippedContentCrop()),
    bottomReadinessEvidence: terminalAnswerReadiness(),
  });

  assert.equal(proof.status, 'unverified');
  assert.equal(proof.reason, 'right_edge_crop_contains_clipped_content');
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), false);
});

test('v16 visible right-edge mode rejects a visible conversation-scoped down-arrow affordance', () => {
  const conversationAffordance = {
    index: 0,
    tag: 'button',
    role: 'button',
    labelPreview: 'Scroll to bottom',
    rect: { x: 492, y: 558, width: 36, height: 36 },
    match: { labelMatch: true, centeredFloatingIcon: true },
  };
  const proof = buildScrollBottomProof({
    label: 'poll-terminal',
    ...savedScreens(),
    screenshotObservation: tinyRightEdgeObservation('poll-terminal:screenshot', {
      moreContentAffordances: [conversationAffordance],
    }),
    domObservation: tinyRightEdgeObservation('poll-terminal:dom'),
    visualScrollbarProof: analyzeRightEdgeScrollbarPixels(decodedCrop({ thumbEnd: 684, bottomCapStart: 691 })),
    bottomReadinessEvidence: terminalAnswerReadiness(),
  });

  assert.equal(proof.status, 'unverified');
  assert.equal(proof.reason, 'more_content_below_affordance_visible');
  assert.equal(proof.moreContentAffordances.status, 'visible');
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), false);
});

test('v16 visible right-edge mode rejects mismatched screenshot and DOM selected roots', () => {
  const proof = buildScrollBottomProof({
    label: 'poll-terminal',
    ...savedScreens(),
    screenshotObservation: tinyRightEdgeObservation('poll-terminal:screenshot'),
    domObservation: tinyRightEdgeObservation('poll-terminal:dom', {
      rect: { x: 52, y: 0, left: 52, top: 0, right: 900, bottom: 640, width: 848, height: 640 },
    }),
    visualScrollbarProof: analyzeRightEdgeScrollbarPixels(decodedCrop({ thumbEnd: 684, bottomCapStart: 691 })),
    bottomReadinessEvidence: terminalAnswerReadiness(),
  });

  assert.equal(proof.status, 'unverified');
  assert.equal(proof.reason, 'scroll_root_metrics_inconsistent');
  assert.equal(proof.consistency.status, 'mismatch');
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), false);
});

test('v16 visible right-edge mode rejects mismatched screenshot and DOM selection kinds', () => {
  const proof = buildScrollBottomProof({
    label: 'poll-terminal',
    ...savedScreens(),
    screenshotObservation: tinyRightEdgeObservation('poll-terminal:screenshot'),
    domObservation: tinyRightEdgeObservation('poll-terminal:dom', {
      selectionKind: 'browser_viewport_scrollbar',
    }),
    visualScrollbarProof: analyzeRightEdgeScrollbarPixels(decodedCrop({ thumbEnd: 684, bottomCapStart: 691 })),
    bottomReadinessEvidence: terminalAnswerReadiness(),
  });

  assert.equal(proof.status, 'unverified');
  assert.equal(proof.reason, 'scroll_root_selection_kind_inconsistent');
  assert.equal(proof.consistency.status, 'mismatch');
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), false);
});

test('v16 short no-scrollbar mode rejects root or stale URL bottom-readiness evidence', () => {
  const proof = buildScrollBottomProof({
    label: 'pre-poll-wait-gate',
    ...savedScreens(),
    screenshotObservation: noScrollableObservation('pre-poll-wait-gate:screenshot'),
    domObservation: noScrollableObservation('pre-poll-wait-gate:dom'),
    visualScrollbarProof: noScrollbarVisualProof(),
    bottomReadinessEvidence: terminalAnswerReadiness({
      status: 'unverified',
      reason: 'session_url_mismatch',
      urlKind: 'root',
      sessionUrlMatches: false,
    }),
  });

  assert.equal(proof.status, 'unverified');
  assert.equal(proof.reason, 'session_url_mismatch');
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), false);
});

test('v16 short no-scrollbar mode rejects missing bottom-readiness evidence', () => {
  const proof = buildScrollBottomProof({
    label: 'pre-poll-wait-gate',
    ...savedScreens(),
    screenshotObservation: noScrollableObservation('pre-poll-wait-gate:screenshot'),
    domObservation: noScrollableObservation('pre-poll-wait-gate:dom'),
    visualScrollbarProof: noScrollbarVisualProof(),
    bottomReadinessEvidence: terminalAnswerReadiness({
      status: 'unverified',
      reason: 'bottom_readiness_evidence_missing',
      authenticatedComposerReadyAtBottom: false,
      activeGenerationAtBottom: false,
      newestTurnAtBottom: false,
      evidenceKinds: [],
    }),
  });

  assert.equal(proof.status, 'unverified');
  assert.equal(proof.reason, 'bottom_readiness_evidence_missing');
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), false);
});
