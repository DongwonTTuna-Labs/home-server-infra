import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  analyzeRightEdgeScrollbarPixels,
  buildScrollBottomProof,
  decodePng,
  encodeRightEdgeCropPng,
  scrollBottomProofVerified,
} from '../lib/scroll-proof.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const fixturePath = name => path.join(__dirname, 'fixtures', name);

function decodedCrop({ height = 720, thumbStart = 288, thumbEnd = 684, bottomCapStart = 691 } = {}) {
  const width = 4;
  const rows = [];
  for (let y = 0; y < height; y += 1) {
    const dark = (y >= thumbStart && y <= thumbEnd) || (y >= bottomCapStart && y <= bottomCapStart + 4);
    const row = Buffer.alloc(width * 3, dark ? 160 : 245);
    rows.push(row);
  }
  return { width, height, colorType: 2, rows };
}


function clippedContentCrop({ height = 703 } = {}) {
  const width = 24;
  const rows = [];
  for (let y = 0; y < height; y += 1) {
    const row = Buffer.alloc(width * 3, 245);
    const inTallRegion = y >= 18 && y <= 525;
    if (inTallRegion) {
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

function bottomObservation(label = 'unit') {
  return {
    schema: 'gpt-webai.bottom-scroll-gate.v1',
    label,
    status: 'at_bottom',
    attempts: 1,
    selected: {
      tag: 'div',
      id: 'conversation-scrollport',
      selectionKind: 'chatgpt_scroll_root_scrollbar',
      rect: { x: 0, y: 0, right: 1280, width: 1280, height: 720 },
      scrollTop: 1200,
      maxScrollTop: 1200,
      scrollHeight: 1920,
      clientHeight: 720,
      atBottom: true,
      visualScrollbarProof: {
        status: 'right_edge_scrollbar_at_bottom',
        selectionKind: 'chatgpt_scroll_root_scrollbar',
      },
    },
    visualScrollbarProof: {
      status: 'right_edge_scrollbar_at_bottom',
      selectionKind: 'chatgpt_scroll_root_scrollbar',
    },
  };
}

test('right-edge scrollbar pixel proof accepts thumb aligned near track bottom', () => {
  const proof = analyzeRightEdgeScrollbarPixels(decodedCrop({ thumbEnd: 684, bottomCapStart: 691 }));

  assert.equal(proof.status, 'right_edge_scrollbar_at_bottom');
  assert.equal(proof.alignment.status, 'bottom_aligned');
  assert.equal(proof.alignment.thumbBottomGapPx, 6);
});

test('right-edge scrollbar pixel proof rejects visible thumb gap from bottom', () => {
  const proof = analyzeRightEdgeScrollbarPixels(decodedCrop({ thumbStart: 157, thumbEnd: 552, bottomCapStart: 691 }));

  assert.equal(proof.status, 'right_edge_scrollbar_not_at_bottom');
  assert.equal(proof.reason, 'right_edge_scrollbar_thumb_bottom_gap');
  assert.equal(proof.alignment.status, 'bottom_gap_exceeds_epsilon');
  assert.equal(proof.alignment.thumbBottomGapPx, 138);
});

test('scroll-bottom proof requires screenshot DOM and pixel bottom alignment together', () => {
  const visualScrollbarProof = analyzeRightEdgeScrollbarPixels(decodedCrop({ thumbEnd: 684, bottomCapStart: 691 }));
  const proof = buildScrollBottomProof({
    label: 'unit',
    fullViewportScreenshot: { status: 'saved', path: '/diagnostics/unit.png' },
    rightEdgeScrollbarCrop: { status: 'saved', path: '/diagnostics/unit.right-edge-scrollbar.png' },
    screenshotObservation: bottomObservation('screenshot'),
    domObservation: bottomObservation('dom'),
    visualScrollbarProof,
  });

  assert.equal(proof.status, 'verified');
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), true);
});

test('scroll-bottom proof remains unverified when pixel crop shows a thumb gap', () => {
  const visualScrollbarProof = analyzeRightEdgeScrollbarPixels(decodedCrop({ thumbStart: 157, thumbEnd: 552, bottomCapStart: 691 }));
  const proof = buildScrollBottomProof({
    label: 'unit',
    fullViewportScreenshot: { status: 'saved', path: '/diagnostics/unit.png' },
    rightEdgeScrollbarCrop: { status: 'saved', path: '/diagnostics/unit.right-edge-scrollbar.png' },
    screenshotObservation: bottomObservation('screenshot'),
    domObservation: bottomObservation('dom'),
    visualScrollbarProof,
  });

  assert.equal(proof.status, 'unverified');
  assert.equal(proof.reason, 'right_edge_scrollbar_thumb_bottom_gap');
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), false);
});


test('right-edge scrollbar pixel proof marks clipped content crops as unavailable instead of a non-bottom scrollbar', () => {
  const proof = analyzeRightEdgeScrollbarPixels(clippedContentCrop());

  assert.equal(proof.status, 'unavailable');
  assert.equal(proof.reason, 'right_edge_crop_contains_clipped_content');
  assert.equal(proof.artifact.reason, 'dominant_dark_band_is_not_on_right_edge');
});

test('right-edge PNG crop encoder anchors to the actual saved screenshot edge', () => {
  const width = 32;
  const height = 4;
  const rows = [];
  for (let y = 0; y < height; y += 1) {
    const row = Buffer.alloc(width * 3, 240);
    for (let x = 0; x < 8; x += 1) row.fill(20, x * 3, x * 3 + 3);
    for (let x = width - 8; x < width; x += 1) row.fill(250, x * 3, x * 3 + 3);
    rows.push(row);
  }
  const cropped = decodePng(encodeRightEdgeCropPng({ width, height, colorType: 2, rows }, 8));

  assert.equal(cropped.width, 8);
  assert.equal(cropped.height, height);
  for (const row of cropped.rows) {
    assert.equal([...row].every(value => value === 250), true);
  }
});

test('right-edge scrollbar pixel proof rejects the real failing ChatGPT crop fixture', async () => {
  const decoded = decodePng(await readFile(fixturePath('right-edge-scrollbar-not-bottom.png')));
  const proof = analyzeRightEdgeScrollbarPixels(decoded);

  assert.equal(proof.status, 'right_edge_scrollbar_not_at_bottom');
  assert.equal(proof.reason, 'right_edge_scrollbar_thumb_bottom_gap');
  assert.equal(proof.alignment.status, 'bottom_gap_exceeds_epsilon');
  assert.equal(proof.thumb.topPx, 175);
  assert.equal(proof.thumb.bottomPx, 469);
  assert.equal(proof.alignment.thumbBottomGapPx, 221);
});



test('scroll-bottom verifier fails closed when a malformed verified proof still reports an affordance', () => {
  const visualScrollbarProof = analyzeRightEdgeScrollbarPixels(decodedCrop({ thumbEnd: 684, bottomCapStart: 691 }));
  const proof = {
    status: 'verified',
    fullViewportScreenshot: { status: 'saved', path: '/diagnostics/unit.png' },
    rightEdgeScrollbarCrop: { status: 'saved', path: '/diagnostics/unit.right-edge-scrollbar.png' },
    visualScrollbarProof,
    moreContentAffordances: {
      status: 'visible',
      count: 1,
      samples: [{ labelPreview: 'Scroll to bottom' }],
    },
  };

  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), false);
});

test('scroll-bottom proof fails when a floating more-content affordance remains visible', () => {
  const visualScrollbarProof = analyzeRightEdgeScrollbarPixels(decodedCrop({ thumbEnd: 684, bottomCapStart: 691 }));
  const observation = {
    ...bottomObservation('screenshot'),
    moreContentAffordances: [{
      index: 0,
      tag: 'button',
      role: 'button',
      labelPreview: 'Scroll to bottom',
      rect: { x: 534, y: 558, width: 36, height: 36 },
      match: { labelMatch: true, centeredFloatingIcon: true },
    }],
  };
  const proof = buildScrollBottomProof({
    label: 'unit',
    fullViewportScreenshot: { status: 'saved', path: '/diagnostics/unit.png' },
    rightEdgeScrollbarCrop: { status: 'saved', path: '/diagnostics/unit.right-edge-scrollbar.png' },
    screenshotObservation: observation,
    domObservation: bottomObservation('dom'),
    visualScrollbarProof,
  });

  assert.equal(proof.status, 'unverified');
  assert.equal(proof.reason, 'more_content_below_affordance_visible');
  assert.equal(proof.moreContentAffordances.status, 'visible');
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), false);
});


test('scroll-bottom proof ignores left-sidebar history affordances when conversation scroll root is at bottom', () => {
  const visualScrollbarProof = analyzeRightEdgeScrollbarPixels(decodedCrop({ thumbEnd: 684, bottomCapStart: 691 }));
  const sidebarAffordances = [
    {
      index: 0,
      tag: 'a',
      role: '',
      testid: '',
      textPreview: 'PR72 Scroll Bottom Fix',
      labelPreview: 'PR72 Scroll Bottom Fix',
      titlePreview: '',
      rect: { x: 6, y: 552, width: 233, height: 36 },
      match: { labelMatch: true, centeredFloatingIcon: false },
    },
    {
      index: 1,
      tag: 'button',
      role: '',
      testid: '',
      textPreview: '',
      labelPreview: 'Pin PR72 Scroll Bottom Fix',
      titlePreview: '',
      rect: { x: -10, y: 552, width: 34, height: 36 },
      match: { labelMatch: true, centeredFloatingIcon: false },
    },
    {
      index: 2,
      tag: 'button',
      role: '',
      testid: 'history-item-0-options',
      textPreview: '',
      labelPreview: 'Open conversation options for PR72 Scroll Bottom Fix',
      titlePreview: '',
      rect: { x: 14, y: 552, width: 34, height: 36 },
      match: { labelMatch: true, centeredFloatingIcon: false },
    },
  ];
  const observation = {
    ...bottomObservation('screenshot'),
    selected: {
      ...bottomObservation('screenshot').selected,
      rect: { x: 52, y: 0, left: 52, top: 0, right: 1050, bottom: 703, width: 998, height: 703 },
    },
    status: 'more_content_affordance_visible',
    moreContentAffordances: sidebarAffordances,
  };
  const domObservation = {
    ...bottomObservation('dom'),
    selected: {
      ...bottomObservation('dom').selected,
      rect: { x: 52, y: 0, left: 52, top: 0, right: 1050, bottom: 703, width: 998, height: 703 },
    },
    status: 'more_content_affordance_visible',
    moreContentAffordances: sidebarAffordances,
  };

  const proof = buildScrollBottomProof({
    label: 'send-after-start-confirmation',
    fullViewportScreenshot: { status: 'saved', path: '/diagnostics/send-after-start-confirmation.png' },
    rightEdgeScrollbarCrop: { status: 'saved', path: '/diagnostics/send-after-start-confirmation.right-edge-scrollbar.png' },
    screenshotObservation: observation,
    domObservation,
    visualScrollbarProof,
  });

  assert.equal(proof.status, 'verified');
  assert.equal(proof.moreContentAffordances.status, 'clear');
  assert.equal(proof.ignoredMoreContentAffordances.status, 'ignored');
  assert.ok(proof.ignoredMoreContentAffordances.count >= 3);
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), true);
});

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

function shortConversationReadiness(overrides = {}) {
  return {
    schema: 'gpt-webai.bottom-readiness-evidence.v1',
    status: 'verified',
    urlKind: 'conversation',
    sessionIdPresent: true,
    sessionUrlMatches: true,
    authenticatedComposerReadyAtBottom: true,
    activeGenerationAtBottom: true,
    newestTurnAtBottom: false,
    evidenceKinds: ['authenticated_composer_ready_at_bottom', 'active_generation_at_bottom'],
    viewport: { width: 1020, height: 703 },
    composer: {
      visible: true,
      nearBottom: true,
      bottomGapPx: 34,
      rect: { x: 230, y: 615, left: 230, top: 615, right: 872, bottom: 669, width: 642, height: 54 },
    },
    activeGenerationControl: {
      visible: true,
      nearBottom: true,
      bottomGapPx: 42,
      rect: { x: 827, y: 623, left: 827, top: 623, right: 866, bottom: 662, width: 39, height: 39 },
    },
    ...overrides,
  };
}

function sidebarFalsePositiveAffordances(y = 696, historyIndex = 7) {
  return [
    {
      index: 0,
      tag: 'a',
      role: '',
      testid: '',
      textPreview: 'PR72 Scroll Bottom Fix',
      labelPreview: 'PR72 Scroll Bottom Fix',
      titlePreview: '',
      rect: { x: 6, y, width: 233, height: 36 },
      match: { labelMatch: true, centeredFloatingIcon: false },
    },
    {
      index: 1,
      tag: 'button',
      role: '',
      testid: '',
      textPreview: '',
      labelPreview: 'Pin PR72 Scroll Bottom Fix',
      titlePreview: '',
      rect: { x: -10, y: y - 12, width: 34, height: 36 },
      match: { labelMatch: true, centeredFloatingIcon: false },
    },
    {
      index: 2,
      tag: 'button',
      role: '',
      testid: `history-item-${historyIndex}-options`,
      textPreview: '',
      labelPreview: 'Open conversation options for PR72 Scroll Bottom Fix',
      titlePreview: '',
      rect: { x: 14, y: y - 12, width: 34, height: 36 },
      match: { labelMatch: true, centeredFloatingIcon: false },
    },
  ];
}

function scrollportNotFoundObservation(label, affordances) {
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
    moreContentAffordances: {
      status: affordances.length > 0 ? 'visible' : 'clear',
      count: affordances.length,
      samples: affordances,
    },
  };
}

test('scroll-bottom proof verifies attached short live session shape after ignoring sidebar history false positives', () => {
  const proof = buildScrollBottomProof({
    label: 'send-after-start-confirmation',
    fullViewportScreenshot: { status: 'saved', path: '/diagnostics/send-after-start-confirmation.png' },
    rightEdgeScrollbarCrop: {
      status: 'saved',
      path: '/diagnostics/send-after-start-confirmation.right-edge-scrollbar.png',
      width: 24,
      height: 703,
    },
    screenshotObservation: scrollportNotFoundObservation('send-after-start-confirmation:screenshot', sidebarFalsePositiveAffordances(696, 7)),
    domObservation: scrollportNotFoundObservation('send-after-start-confirmation:dom', sidebarFalsePositiveAffordances(732, 8)),
    visualScrollbarProof: noScrollbarVisualProof(),
    bottomReadinessEvidence: shortConversationReadiness(),
  });

  assert.equal(proof.status, 'verified');
  assert.equal(proof.reason, undefined);
  assert.equal(proof.moreContentAffordances.status, 'clear');
  assert.equal(proof.ignoredMoreContentAffordances.status, 'ignored');
  assert.equal(proof.ignoredMoreContentAffordances.count, 6);
  assert.equal(proof.noScrollableConversationOverflowProof.status, 'verified');
  assert.equal(proof.noScrollableConversationOverflowProof.observations.rightEdgeScrollbar, 'no_visible_scrollbar');
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), true);
});

test('short no-scrollbar proof fails closed without bottom composer, generation, or newest-turn evidence', () => {
  const proof = buildScrollBottomProof({
    label: 'send-after-start-confirmation',
    fullViewportScreenshot: { status: 'saved', path: '/diagnostics/send-after-start-confirmation.png' },
    rightEdgeScrollbarCrop: {
      status: 'saved',
      path: '/diagnostics/send-after-start-confirmation.right-edge-scrollbar.png',
      width: 24,
      height: 703,
    },
    screenshotObservation: scrollportNotFoundObservation('send-after-start-confirmation:screenshot', sidebarFalsePositiveAffordances(696, 7)),
    domObservation: scrollportNotFoundObservation('send-after-start-confirmation:dom', sidebarFalsePositiveAffordances(732, 8)),
    visualScrollbarProof: noScrollbarVisualProof(),
    bottomReadinessEvidence: {
      ...shortConversationReadiness({
        status: 'unverified',
        reason: 'bottom_readiness_evidence_missing',
        authenticatedComposerReadyAtBottom: false,
        activeGenerationAtBottom: false,
        newestTurnAtBottom: false,
        evidenceKinds: [],
      }),
    },
  });

  assert.equal(proof.status, 'unverified');
  assert.equal(proof.reason, 'bottom_readiness_evidence_missing');
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), false);
});

test('short no-scrollbar verifier fails closed when no-scrollbar pixel proof is malformed', () => {
  const proof = buildScrollBottomProof({
    label: 'send-after-start-confirmation',
    fullViewportScreenshot: { status: 'saved', path: '/diagnostics/send-after-start-confirmation.png' },
    rightEdgeScrollbarCrop: {
      status: 'saved',
      path: '/diagnostics/send-after-start-confirmation.right-edge-scrollbar.png',
      width: 24,
      height: 703,
    },
    screenshotObservation: scrollportNotFoundObservation('send-after-start-confirmation:screenshot', sidebarFalsePositiveAffordances(696, 7)),
    domObservation: scrollportNotFoundObservation('send-after-start-confirmation:dom', sidebarFalsePositiveAffordances(732, 8)),
    visualScrollbarProof: {
      status: 'unavailable',
      reason: 'right_edge_scrollbar_crop_missing',
      method: 'right_edge_crop_pixel_scan',
      alignment: { status: 'unavailable' },
    },
    bottomReadinessEvidence: shortConversationReadiness(),
  });

  assert.equal(proof.status, 'unverified');
  assert.equal(proof.reason, 'right_edge_scrollbar_crop_missing');

  const forged = {
    ...proof,
    status: 'verified',
    reason: undefined,
    noScrollableConversationOverflowProof: {
      ...proof.noScrollableConversationOverflowProof,
      status: 'verified',
      reason: undefined,
      observations: {
        ...proof.noScrollableConversationOverflowProof.observations,
        rightEdgeScrollbar: 'no_visible_scrollbar',
      },
    },
  };

  assert.equal(scrollBottomProofVerified({ scrollBottomProof: forged }), false);
});

test('short no-scrollbar proof still fails when a conversation-body down affordance remains visible', () => {
  const conversationAffordance = {
    index: 0,
    tag: 'button',
    role: 'button',
    labelPreview: 'Scroll to bottom',
    rect: { x: 492, y: 558, width: 36, height: 36 },
    match: { labelMatch: true, centeredFloatingIcon: true },
  };
  const proof = buildScrollBottomProof({
    label: 'send-after-start-confirmation',
    fullViewportScreenshot: { status: 'saved', path: '/diagnostics/send-after-start-confirmation.png' },
    rightEdgeScrollbarCrop: {
      status: 'saved',
      path: '/diagnostics/send-after-start-confirmation.right-edge-scrollbar.png',
      width: 24,
      height: 703,
    },
    screenshotObservation: scrollportNotFoundObservation('send-after-start-confirmation:screenshot', [
      ...sidebarFalsePositiveAffordances(696, 7),
      conversationAffordance,
    ]),
    domObservation: scrollportNotFoundObservation('send-after-start-confirmation:dom', sidebarFalsePositiveAffordances(732, 8)),
    visualScrollbarProof: noScrollbarVisualProof(),
    bottomReadinessEvidence: shortConversationReadiness(),
  });

  assert.equal(proof.status, 'unverified');
  assert.equal(proof.reason, 'more_content_below_affordance_visible');
  assert.equal(proof.moreContentAffordances.status, 'visible');
  assert.equal(scrollBottomProofVerified({ scrollBottomProof: proof }), false);
});

test('short no-scrollbar proof fails closed on root or stale session URL evidence', () => {
  const proof = buildScrollBottomProof({
    label: 'pre-poll-wait-gate',
    fullViewportScreenshot: { status: 'saved', path: '/diagnostics/pre-poll-wait-gate.png' },
    rightEdgeScrollbarCrop: {
      status: 'saved',
      path: '/diagnostics/pre-poll-wait-gate.right-edge-scrollbar.png',
      width: 24,
      height: 703,
    },
    screenshotObservation: scrollportNotFoundObservation('pre-poll-wait-gate:screenshot', []),
    domObservation: scrollportNotFoundObservation('pre-poll-wait-gate:dom', []),
    visualScrollbarProof: noScrollbarVisualProof(),
    bottomReadinessEvidence: shortConversationReadiness({
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
