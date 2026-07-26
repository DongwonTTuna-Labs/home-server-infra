
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { writePageDiagnostics } from '../../lib/diagnostics.mjs';
import { decodePng, encodePng } from '../../lib/scroll-proof.mjs';
import { fakePage, fakeRawDiagnostics } from './fixtures.mjs';


function rgbImage(width, height, fillPixel) {
  const rows = [];
  for (let y = 0; y < height; y += 1) {
    const row = Buffer.alloc(width * 3, 245);
    for (let x = 0; x < width; x += 1) {
      const [r, g, b] = fillPixel(x, y);
      row[x * 3] = r;
      row[x * 3 + 1] = g;
      row[x * 3 + 2] = b;
    }
    rows.push(row);
  }
  return { width, height, colorType: 2, rows };
}

function noScrollableOverflowObservation() {
  return {
    schema: 'gpt-webai.bottom-scroll-gate.v1',
    status: 'scrollport_not_found',
    candidateCount: 0,
    visualScrollbarProof: {
      status: 'unavailable',
      reason: 'scrollport_not_found',
      method: 'dom_scroll_metrics_and_right_edge_scroll_root_scrollbar',
    },
    moreContentAffordances: [],
    ignoredMoreContentAffordances: [],
  };
}

function verifiedBottomReadiness() {
  return {
    schema: 'gpt-webai.bottom-readiness-evidence.v1',
    status: 'verified',
    urlKind: 'conversation',
    sessionIdPresent: true,
    sessionUrlMatches: true,
    authenticatedComposerReadyAtBottom: true,
    activeGenerationAtBottom: false,
    newestTurnAtBottom: false,
    evidenceKinds: ['authenticated_composer_ready_at_bottom'],
    viewport: { width: 1050, height: 8 },
    composer: {
      visible: true,
      nearBottom: true,
      bottomGapPx: 2,
      rect: { x: 230, y: 4, left: 230, top: 4, right: 870, bottom: 6, width: 640, height: 2 },
      count: 1,
      disabled: false,
    },
    activeGenerationControl: { visible: false, count: 0 },
  };
}

test('writePageDiagnostics saves screenshot and DOM JSON under artifact diagnostics dir', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'gpt-webai-diagnostics-'));
  const oldArtifacts = process.env.GPT_WEBAI_ARTIFACTS_DIR;
  const oldHost = process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR;
  process.env.GPT_WEBAI_ARTIFACTS_DIR = root;
  process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR = root;
  try {
    const result = await writePageDiagnostics(fakePage(), { label: 'poll-start-before-wait', sessionId: 'sid-unit' });
    assert.equal(result.screenshot, 'saved');
    assert.equal(result.dom, 'saved');
    assert.equal(result.rightEdgeScrollbarCrop.status, 'saved');
    assert.equal(result.scrollBottomProof.status, 'unverified');
    assert.equal(result.readinessSignals.stopControls, 1);
    assert.equal(await readFile(result.screenshotPath, 'utf8'), 'fake png bytes');
    assert.equal(await readFile(result.rightEdgeScrollbarCrop.path, 'utf8'), 'fake png bytes');
    const saved = JSON.parse(await readFile(result.domPath, 'utf8'));
    assert.equal(saved.label, 'poll-start-before-wait');
    assert.equal(saved.sessionId, 'sid-unit');
    assert.equal(saved.selectorInventory.controls, 1);
    assert.equal(saved.rightEdgeScrollbarCrop.status, 'saved');
    assert.equal(saved.scrollBottomProof.status, 'unverified');
    const proof = JSON.parse(await readFile(result.scrollBottomProofPath, 'utf8'));
    assert.equal(proof.schema, 'gpt-webai.scroll-bottom-proof.v1');
  } finally {
    if (oldArtifacts === undefined) delete process.env.GPT_WEBAI_ARTIFACTS_DIR;
    else process.env.GPT_WEBAI_ARTIFACTS_DIR = oldArtifacts;
    if (oldHost === undefined) delete process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR;
    else process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR = oldHost;
    await rm(root, { recursive: true, force: true });
  }
});



test('writePageDiagnostics derives the right-edge crop from the saved full screenshot dimensions', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'gpt-webai-diagnostics-crop-'));
  const oldArtifacts = process.env.GPT_WEBAI_ARTIFACTS_DIR;
  const oldHost = process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR;
  const oldAttempts = process.env.GPT_WEBAI_BOTTOM_SCROLL_ATTEMPTS;
  process.env.GPT_WEBAI_ARTIFACTS_DIR = root;
  process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR = root;
  process.env.GPT_WEBAI_BOTTOM_SCROLL_ATTEMPTS = '1';
  const fullImage = rgbImage(1050, 8, x => (x < 24 ? [20, 20, 20] : x >= 1026 ? [250, 250, 250] : [245, 245, 245]));
  const events = [];
  const page = {
    async title() {
      return 'Unit';
    },
    url() {
      return 'https://chatgpt.com/c/sid-unit';
    },
    viewportSize() {
      return { width: 24, height: 8 };
    },
    async evaluate(fn, args = {}) {
      if (fn?.name === 'scrollPrimaryConversationViewportInPage') {
        events.push('bottom-scroll');
        return noScrollableOverflowObservation();
      }
      events.push('dom-capture');
      return {
        ...fakeRawDiagnostics(),
        label: args.captureLabel || 'unit',
        sessionId: args.captureSessionId || 'sid-unit',
        title: args.captureTitle || 'Unit',
        bottomReadinessEvidence: verifiedBottomReadiness(),
      };
    },
    async screenshot({ path: screenshotPath, clip }) {
      if (clip) throw new Error('clip screenshot should not be used for a valid full screenshot PNG');
      events.push('screenshot');
      await writeFile(screenshotPath, encodePng(fullImage));
    },
  };

  try {
    const result = await writePageDiagnostics(page, { label: 'actual-edge-crop', sessionId: 'sid-unit' });
    assert.equal(result.screenshot, 'saved');
    assert.equal(result.fullViewportScreenshot.width, 1050);
    assert.equal(result.rightEdgeScrollbarCrop.source, 'full_viewport_screenshot_right_edge');
    assert.equal(result.rightEdgeScrollbarCrop.sourceWidth, 1050);
    assert.equal(result.rightEdgeScrollbarCrop.width, 24);
    assert.equal(result.scrollBottomProof.status, 'verified');
    assert.deepEqual(events, ['bottom-scroll', 'screenshot', 'bottom-scroll', 'dom-capture']);

    const crop = decodePng(await readFile(result.rightEdgeScrollbarCrop.path));
    assert.equal(crop.width, 24);
    assert.equal(crop.height, 8);
    for (const row of crop.rows) {
      assert.equal([...row].every(value => value === 250), true);
    }
  } finally {
    if (oldArtifacts === undefined) delete process.env.GPT_WEBAI_ARTIFACTS_DIR;
    else process.env.GPT_WEBAI_ARTIFACTS_DIR = oldArtifacts;
    if (oldHost === undefined) delete process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR;
    else process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR = oldHost;
    if (oldAttempts === undefined) delete process.env.GPT_WEBAI_BOTTOM_SCROLL_ATTEMPTS;
    else process.env.GPT_WEBAI_BOTTOM_SCROLL_ATTEMPTS = oldAttempts;
    await rm(root, { recursive: true, force: true });
  }
});

test('writePageDiagnostics scrolls the conversation bottom before screenshot and before DOM capture', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'gpt-webai-scroll-order-'));
  const oldArtifacts = process.env.GPT_WEBAI_ARTIFACTS_DIR;
  const oldHost = process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR;
  const oldAttempts = process.env.GPT_WEBAI_BOTTOM_SCROLL_ATTEMPTS;
  process.env.GPT_WEBAI_ARTIFACTS_DIR = root;
  process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR = root;
  process.env.GPT_WEBAI_BOTTOM_SCROLL_ATTEMPTS = '1';
  const events = [];
  const page = {
    async title() {
      return 'Unit';
    },
    async evaluate(fn, args = {}) {
      if (fn?.name === 'scrollPrimaryConversationViewportInPage') {
        events.push('bottom-scroll');
        return {
          schema: 'gpt-webai.bottom-scroll-gate.v1',
          status: 'at_bottom',
          selected: {
            id: 'conversation-scrollport',
            rect: { x: 0, y: 0, right: 1280, width: 1280, height: 720 },
            scrollTop: 100,
            maxScrollTop: 100,
            atBottom: true,
          },
          visualScrollbarProof: { status: 'right_edge_scrollbar_at_bottom' },
        };
      }
      events.push('dom-capture');
      return {
        ...fakeRawDiagnostics(),
        label: args.captureLabel || 'unit',
        sessionId: args.captureSessionId || 'sid-unit',
        title: args.captureTitle || 'Unit',
      };
    },
    viewportSize() {
      return { width: 1280, height: 720 };
    },
    async screenshot({ path: screenshotPath, clip }) {
      events.push(clip ? 'right-edge-crop' : 'screenshot');
      await writeFile(screenshotPath, 'fake png bytes');
    },
  };

  try {
    const result = await writePageDiagnostics(page, { label: 'scroll-order', sessionId: 'sid-unit' });
    assert.equal(result.screenshot, 'saved');
    assert.equal(result.dom, 'saved');
    assert.deepEqual(events, ['bottom-scroll', 'screenshot', 'right-edge-crop', 'bottom-scroll', 'dom-capture']);
    assert.equal(result.bottomScroll.screenshot.status, 'at_bottom');
    assert.equal(result.bottomScroll.dom.status, 'at_bottom');
    assert.equal(result.rightEdgeScrollbarCrop.status, 'saved');
    assert.equal(result.scrollBottomProof.status, 'unverified');
  } finally {
    if (oldArtifacts === undefined) delete process.env.GPT_WEBAI_ARTIFACTS_DIR;
    else process.env.GPT_WEBAI_ARTIFACTS_DIR = oldArtifacts;
    if (oldHost === undefined) delete process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR;
    else process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR = oldHost;
    if (oldAttempts === undefined) delete process.env.GPT_WEBAI_BOTTOM_SCROLL_ATTEMPTS;
    else process.env.GPT_WEBAI_BOTTOM_SCROLL_ATTEMPTS = oldAttempts;
    await rm(root, { recursive: true, force: true });
  }
});

test('writePageDiagnostics propagates scoped provider-limit surfaces in wrapper diagnostics', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'gpt-webai-provider-limit-wrapper-'));
  const oldArtifacts = process.env.GPT_WEBAI_ARTIFACTS_DIR;
  const oldHost = process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR;
  process.env.GPT_WEBAI_ARTIFACTS_DIR = root;
  process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR = root;
  const page = fakePage();
  const originalEvaluate = page.evaluate;
  page.evaluate = async (fn, args = {}) => ({
    ...(await originalEvaluate(fn, args)),
    readinessSignals: {
      ...fakeRawDiagnostics().readinessSignals,
      limit: true,
      providerLimitSurfaceCount: 1,
    },
    providerLimitSurfaces: [{
      index: 0,
      tag: 'div',
      role: 'dialog',
      kind: 'dialog',
      className: '',
      text: 'Too many requests. Try again later.',
      rect: { x: 1, y: 1, width: 300, height: 100 },
      actionButtons: [],
    }],
  });

  try {
    const result = await writePageDiagnostics(page, { label: 'limit-wrapper', sessionId: 'sid-unit' });
    assert.equal(result.readinessSignals.limit, true);
    assert.equal(result.providerLimitSurfaces.length, 1);
    assert.equal(result.providerLimitSurfaces[0].textSha256.length, 64);
  } finally {
    if (oldArtifacts === undefined) delete process.env.GPT_WEBAI_ARTIFACTS_DIR;
    else process.env.GPT_WEBAI_ARTIFACTS_DIR = oldArtifacts;
    if (oldHost === undefined) delete process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR;
    else process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR = oldHost;
    await rm(root, { recursive: true, force: true });
  }
});
