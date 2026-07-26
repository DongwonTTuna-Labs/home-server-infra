import process from 'node:process';
import { mkdir } from 'node:fs/promises';
import path from 'node:path';

import {
  CHATGPT_ROOT,
  conversationUrlMatchesSession,
  jsonOut,
  validConversationUrl,
} from './common.mjs';
import {
  browserGuidFromWebSocketDebuggerUrl,
  deriveBrowserPageIdentity,
} from './contracts/r13.mjs';

export async function loadPlaywright() {
  try {
    return await import('playwright-core');
  } catch (error) {
    jsonOut({
      ok: true,
      vendor: 'chatgpt',
      status: 'unreachable',
      reason: 'provider.missing_playwright',
      message: error instanceof Error ? error.message : String(error),
    });
    process.exit(70);
  }
}

export function cdpEndpoint() {
  const port = process.env.CDP_PORT || '9222';
  return `http://127.0.0.1:${port}`;
}

export function playwrightArtifactsDir() {
  const root = process.env.GPT_WEBAI_ARTIFACTS_DIR || '/tmp/gpt-webai-artifacts';
  return path.join(root, 'playwright-artifacts');
}

export async function withBrowser(callback) {
  const { chromium } = await loadPlaywright();
  const artifactsDir = playwrightArtifactsDir();
  try {
    await mkdir(artifactsDir, { recursive: true });
    const browser = await chromium.connectOverCDP(cdpEndpoint(), {
      artifactsDir,
      isLocal: true,
    });
    return await callback(browser);
  } catch (error) {
    jsonOut({
      ok: true,
      vendor: 'chatgpt',
      status: 'unreachable',
      cdp: 'unreachable',
      reason: 'browser.cdp_unreachable',
      message: error instanceof Error ? error.message : String(error),
    });
    process.exit(70);
  }
}

export async function withBrowserR13(callback) {
  const { chromium } = await import('playwright-core');
  const artifactsDir = playwrightArtifactsDir();
  await mkdir(artifactsDir, { recursive: true });
  const browser = await chromium.connectOverCDP(cdpEndpoint(), {
    artifactsDir,
    isLocal: true,
  });
  return callback(browser);
}

export async function selectExistingPage(browser, sessionId = '') {
  const contexts = browser.contexts();
  if (contexts.length === 0) throw new Error('probe.unreachable: browser context missing');
  const pages = contexts.flatMap(context => context.pages());
  const page = pages.find(item => (
    sessionId && conversationUrlMatchesSession(item.url(), sessionId)
  ))
    || pages.find(item => /^https:\/\/chatgpt\.com\//.test(item.url()))
    || pages[0];
  if (!page) throw new Error('probe.unreachable: page target missing');
  return page;
}

export async function selectPage(browser, sessionId = '') {
  const contexts = browser.contexts();
  const context = contexts[0] || await browser.newContext({ acceptDownloads: true });
  const pages = context.pages();
  let page = pages.find(item => (
    sessionId && conversationUrlMatchesSession(item.url(), sessionId)
  ));
  page ||= pages.find(item => /^https:\/\/chatgpt\.com\//.test(item.url()));
  page ||= pages[0] || await context.newPage();

  if (sessionId && !conversationUrlMatchesSession(page.url(), sessionId)) {
    await page.goto(`${CHATGPT_ROOT}c/${sessionId}`, { waitUntil: 'domcontentloaded', timeout: 60_000 }).catch(() => undefined);
  } else if (!/^https:\/\/chatgpt\.com\//.test(page.url())) {
    await page.goto(CHATGPT_ROOT, { waitUntil: 'domcontentloaded', timeout: 60_000 }).catch(() => undefined);
  }
  return page;
}

export async function selectFreshPage(browser) {
  const page = await selectPage(browser);
  if (validConversationUrl(page.url())) {
    await page.goto(CHATGPT_ROOT, { waitUntil: 'domcontentloaded', timeout: 60_000 }).catch(() => undefined);
  }
  return page;
}

export async function captureBrowserPageIdentity(page) {
  const versionResponse = await fetch(`${cdpEndpoint()}/json/version`);
  if (!versionResponse.ok) {
    throw new Error(`capture.timeout: CDP version endpoint returned ${versionResponse.status}`);
  }
  const version = await versionResponse.json();
  const browserGuid = browserGuidFromWebSocketDebuggerUrl(version.webSocketDebuggerUrl);
  const cdp = await page.context().newCDPSession(page);
  try {
    const [{ targetInfo }, { frameTree }] = await Promise.all([
      cdp.send('Target.getTargetInfo'),
      cdp.send('Page.getFrameTree'),
    ]);
    const frame = frameTree?.frame;
    if (!targetInfo || !frame) {
      throw new Error('capture.timeout: incomplete CDP target/frame identity');
    }
    return {
      browserGuid,
      cdpBrowserContextId: targetInfo.browserContextId ?? '',
      cdpTargetId: targetInfo.targetId,
      mainFrameId: frame.id,
      loaderId: frame.loaderId,
      ...deriveBrowserPageIdentity({
        browserGuid,
        cdpBrowserContextId: targetInfo.browserContextId ?? '',
        cdpTargetId: targetInfo.targetId,
        mainFrameId: frame.id,
        loaderId: frame.loaderId,
      }),
    };
  } finally {
    await cdp.detach().catch(() => undefined);
  }
}
