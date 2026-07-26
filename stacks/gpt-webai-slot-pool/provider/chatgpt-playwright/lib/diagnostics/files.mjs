
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';

import { sanitizeFilename, sha256Text } from '../common.mjs';
import {
  writeEvidenceBytes,
  writeEvidenceJson,
} from '../contracts/r13.mjs';
import { captureBrowserPageIdentity } from '../browser-session.mjs';
import { scrollPrimaryConversationToBottom } from '../bottom-scroll.mjs';
import {
  analyzeRightEdgeScrollbarCrop,
  buildScrollBottomProof,
  decodePng,
  encodeRightEdgeCropPng,
} from '../scroll-proof.mjs';
import { pageDiagnostics } from './dom.mjs';
import { hostPathFor } from './paths.mjs';

export async function writePageDiagnostics(page, { label = 'capture', sessionId = '' } = {}) {
  const root = process.env.GPT_WEBAI_ARTIFACTS_DIR || '/broker-artifacts/manual';
  const safeLabel = sanitizeFilename(label, 'capture').replace(/\.[A-Za-z0-9]+$/, '');
  const diagnosticsDir = path.join(root, 'diagnostics');
  await mkdir(diagnosticsDir, { recursive: true });
  const screenshotPath = path.join(diagnosticsDir, `${safeLabel}.png`);
  const cropPath = path.join(diagnosticsDir, `${safeLabel}.right-edge-scrollbar.png`);
  const domPath = path.join(diagnosticsDir, `${safeLabel}.dom.json`);
  const proofPath = path.join(diagnosticsDir, `${safeLabel}.scroll-proof.json`);
  const result = {
    label,
    screenshotPath: hostPathFor(screenshotPath),
    domPath: hostPathFor(domPath),
    rightEdgeScrollbarCropPath: hostPathFor(cropPath),
    scrollBottomProofPath: hostPathFor(proofPath),
    bottomScroll: {},
  };

  result.bottomScroll.screenshot = await scrollPrimaryConversationToBottom(page, { label: `${label}:screenshot` });

  let fullScreenshotPng = null;
  try {
    await page.screenshot({ path: screenshotPath, fullPage: false, timeout: 15_000 });
    result.screenshot = 'saved';
    result.fullViewportScreenshot = { status: 'saved', path: hostPathFor(screenshotPath) };
    try {
      fullScreenshotPng = decodePng(await readFile(screenshotPath));
      result.fullViewportScreenshot.width = fullScreenshotPng.width;
      result.fullViewportScreenshot.height = fullScreenshotPng.height;
    } catch (decodeError) {
      result.fullViewportScreenshot.dimensionError = decodeError instanceof Error
        ? decodeError.message
        : String(decodeError);
    }
  } catch (error) {
    result.screenshot = 'failed';
    result.screenshotError = error instanceof Error ? error.message : String(error);
    result.fullViewportScreenshot = {
      status: 'failed',
      path: hostPathFor(screenshotPath),
      error: result.screenshotError,
    };
  }

  result.rightEdgeScrollbarCrop = await saveRightEdgeScrollbarCrop({
    page,
    cropPath,
    fullScreenshotPng,
  });

  const pixelProof = result.rightEdgeScrollbarCrop.status === 'saved'
    ? await analyzeRightEdgeScrollbarCrop(cropPath)
    : {
        status: 'unavailable',
        reason: 'right_edge_scrollbar_crop_missing',
        method: 'right_edge_crop_pixel_scan',
        alignment: { status: 'unavailable' },
      };

  try {
    const diagnostics = await pageDiagnostics(page, { label, sessionId });
    const scrollBottomProof = buildScrollBottomProof({
      label,
      fullViewportScreenshot: result.fullViewportScreenshot,
      rightEdgeScrollbarCrop: result.rightEdgeScrollbarCrop,
      screenshotObservation: result.bottomScroll.screenshot,
      domObservation: diagnostics.bottomScroll,
      visualScrollbarProof: pixelProof,
      bottomReadinessEvidence: diagnostics.bottomReadinessEvidence,
    });
    diagnostics.fullViewportScreenshot = result.fullViewportScreenshot;
    diagnostics.rightEdgeScrollbarCrop = result.rightEdgeScrollbarCrop;
    diagnostics.scrollBottomProof = scrollBottomProof;
    await writeFile(domPath, `${JSON.stringify(diagnostics, null, 2)}\n`, 'utf8');
    await writeFile(proofPath, `${JSON.stringify(scrollBottomProof, null, 2)}\n`, 'utf8');
    result.dom = 'saved';
    result.url = diagnostics.url;
    result.title = diagnostics.title;
    result.readinessSignals = diagnostics.readinessSignals;
    result.selectorInventory = diagnostics.selectorInventory;
    result.dialogs = diagnostics.dialogs;
    result.providerLimitSurfaces = diagnostics.providerLimitSurfaces;
    result.fullViewportScreenshot = diagnostics.fullViewportScreenshot;
    result.rightEdgeScrollbarCrop = diagnostics.rightEdgeScrollbarCrop;
    result.scrollBottomProof = scrollBottomProof;
    result.bottomScroll.dom = diagnostics.bottomScroll;
  } catch (error) {
    result.dom = 'failed';
    result.domError = error instanceof Error ? error.message : String(error);
    const scrollBottomProof = buildScrollBottomProof({
      label,
      fullViewportScreenshot: result.fullViewportScreenshot,
      rightEdgeScrollbarCrop: result.rightEdgeScrollbarCrop,
      screenshotObservation: result.bottomScroll.screenshot,
      domObservation: {},
      visualScrollbarProof: pixelProof,
    });
    result.scrollBottomProof = scrollBottomProof;
    await writeFile(proofPath, `${JSON.stringify(scrollBottomProof, null, 2)}\n`, 'utf8').catch(() => undefined);
  }
  return result;
}

export async function writeR13OperationEvidence(page, {
  request,
  evidenceRoot,
  captureIndex = 0,
}) {
  const capturedAtMs = Date.now();
  const masks = [
    page.locator('main'),
    page.locator('aside,nav'),
    page.locator('[role="dialog"],dialog,[aria-modal="true"]'),
  ];
  const screenshot = await page.screenshot({
    animations: 'disabled',
    fullPage: false,
    mask: masks,
    maskColor: '#000000',
    timeout: Math.min(request.deadlineMs, 15_000),
    type: 'png',
  });
  const screenshotRef = await writeEvidenceBytes({
    evidenceRoot,
    relPath: evidenceCapturePath(request.evidence.screenshotRelPath, captureIndex),
    bytes: screenshot,
    mediaType: 'image/png',
    operationId: request.identity.operationId,
  });

  const rawDom = await page.evaluate(() => {
    const visible = node => {
      const rect = node?.getBoundingClientRect?.();
      if (!rect || rect.width <= 0 || rect.height <= 0) return false;
      const style = getComputedStyle(node);
      return style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0';
    };
    const nodes = Array.from(document.querySelectorAll(
      'button,[role="button"],[role="combobox"],textarea,[contenteditable="true"],[data-message-author-role]',
    )).filter(visible).slice(0, 256);
    return {
      controlCount: nodes.filter(node => !node.hasAttribute('data-message-author-role')).length,
      mutationGeneration: Number(window.__gptWebaiR13MutationGeneration || 0),
      nodes: nodes.map((node, index) => {
        const rect = node.getBoundingClientRect();
        const text = node.innerText || node.textContent || '';
        const label = node.getAttribute('aria-label') || '';
        const messageId = node.getAttribute('data-message-id') || '';
        return {
          authorRole: node.getAttribute('data-message-author-role') || null,
          disabled: Boolean(node.disabled || node.getAttribute('aria-disabled') === 'true'),
          index,
          label,
          messageId,
          rect: {
            height: Math.round(rect.height),
            width: Math.round(rect.width),
            x: Math.round(rect.x),
            y: Math.round(rect.y),
          },
          role: node.getAttribute('role') || null,
          tagName: String(node.tagName || '').toLowerCase(),
          testId: node.getAttribute('data-testid') || '',
          text,
        };
      }),
      url: window.location.href,
    };
  });
  const dom = {
    capturedAtMs,
    controlCount: rawDom.controlCount,
    mutationGeneration: rawDom.mutationGeneration,
    nodes: rawDom.nodes.map(node => ({
      authorRole: node.authorRole,
      disabled: node.disabled,
      index: node.index,
      labelLength: node.label.length,
      labelSha256: node.label ? `sha256:${sha256Text(node.label)}` : null,
      messageIdSha256: node.messageId ? `sha256:${sha256Text(node.messageId)}` : null,
      rect: node.rect,
      role: node.role,
      tagName: node.tagName,
      testIdSha256: node.testId ? `sha256:${sha256Text(node.testId)}` : null,
      textLength: node.text.length,
      textSha256: node.text ? `sha256:${sha256Text(node.text)}` : null,
    })),
    operation: request.operation,
    operationId: request.identity.operationId,
    schema: 'pr72.dom-sanitized.r13.v1',
    url: rawDom.url,
  };
  const domRef = await writeEvidenceJson({
    evidenceRoot,
    relPath: evidenceCapturePath(request.evidence.domRelPath, captureIndex),
    value: dom,
    operationId: request.identity.operationId,
  });

  const identity = await captureBrowserPageIdentity(page);
  const cdp = {
    browserContextId: identity.browserContextId,
    browserGuidSha256: `sha256:${sha256Text(identity.browserGuid)}`,
    capturedAtMs,
    loaderIdSha256: `sha256:${sha256Text(identity.loaderId)}`,
    mainFrameIdSha256: `sha256:${sha256Text(identity.mainFrameId)}`,
    operation: request.operation,
    operationId: request.identity.operationId,
    pageIncarnationId: identity.pageIncarnationId,
    schema: 'pr72.cdp-sanitized.r13.v1',
    targetId: identity.targetId,
  };
  const cdpRef = await writeEvidenceJson({
    evidenceRoot,
    relPath: evidenceCapturePath(request.evidence.cdpRelPath, captureIndex),
    value: cdp,
    operationId: request.identity.operationId,
  });
  return [screenshotRef, domRef, cdpRef];
}

export function evidenceCapturePath(relPath, captureIndex) {
  if (!Number.isInteger(captureIndex) || captureIndex < 0 || captureIndex > 999) {
    throw new Error('provider.schema_drift: evidence capture index');
  }
  if (captureIndex === 0) return relPath;
  const extension = path.posix.extname(relPath);
  const stem = extension ? relPath.slice(0, -extension.length) : relPath;
  return `${stem}.wait-${String(captureIndex).padStart(3, '0')}${extension}`;
}


async function saveRightEdgeScrollbarCrop({ page, cropPath, fullScreenshotPng }) {
  const requestedCropWidth = positiveInt(process.env.GPT_WEBAI_SCROLLBAR_CROP_WIDTH, 24);
  if (fullScreenshotPng) {
    try {
      const cropWidth = Math.min(requestedCropWidth, fullScreenshotPng.width);
      await writeFile(cropPath, encodeRightEdgeCropPng(fullScreenshotPng, cropWidth));
      return {
        status: 'saved',
        path: hostPathFor(cropPath),
        width: cropWidth,
        height: fullScreenshotPng.height,
        source: 'full_viewport_screenshot_right_edge',
        sourceWidth: fullScreenshotPng.width,
        sourceHeight: fullScreenshotPng.height,
      };
    } catch (error) {
      return await saveRightEdgeScrollbarCropWithPlaywrightClip({
        page,
        cropPath,
        requestedCropWidth,
        previousError: error,
      });
    }
  }
  return await saveRightEdgeScrollbarCropWithPlaywrightClip({ page, cropPath, requestedCropWidth });
}

async function saveRightEdgeScrollbarCropWithPlaywrightClip({
  page,
  cropPath,
  requestedCropWidth,
  previousError = null,
}) {
  try {
    const viewport = await pageViewportSize(page);
    const cropWidth = Math.min(requestedCropWidth, viewport.width || requestedCropWidth);
    await page.screenshot({
      path: cropPath,
      fullPage: false,
      timeout: 15_000,
      clip: {
        x: Math.max(0, (viewport.width || cropWidth) - cropWidth),
        y: 0,
        width: cropWidth,
        height: Math.max(1, viewport.height || 1),
      },
    });
    return {
      status: 'saved',
      path: hostPathFor(cropPath),
      width: cropWidth,
      height: Math.max(1, viewport.height || 1),
      source: 'playwright_clip_fallback',
      sourceWidth: viewport.width,
      sourceHeight: viewport.height,
      previousError: previousError instanceof Error ? previousError.message : previousError ? String(previousError) : undefined,
    };
  } catch (error) {
    return {
      status: 'failed',
      path: hostPathFor(cropPath),
      error: error instanceof Error ? error.message : String(error),
      previousError: previousError instanceof Error ? previousError.message : previousError ? String(previousError) : undefined,
    };
  }
}

function positiveInt(value, fallback) {
  const parsed = Number.parseInt(String(value || ''), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

async function pageViewportSize(page) {
  if (typeof page?.viewportSize === 'function') {
    const viewport = page.viewportSize();
    if (viewport && Number.isFinite(Number(viewport.width)) && Number.isFinite(Number(viewport.height))) {
      return { width: Math.round(Number(viewport.width)), height: Math.round(Number(viewport.height)) };
    }
  }
  try {
    const viewport = await page.evaluate(() => ({
      width: Number(window?.innerWidth || document?.documentElement?.clientWidth || 0),
      height: Number(window?.innerHeight || document?.documentElement?.clientHeight || 0),
    }));
    return {
      width: Math.max(1, Math.round(Number(viewport?.width || 0))),
      height: Math.max(1, Math.round(Number(viewport?.height || 0))),
    };
  } catch {
    return { width: 1280, height: 720 };
  }
}
