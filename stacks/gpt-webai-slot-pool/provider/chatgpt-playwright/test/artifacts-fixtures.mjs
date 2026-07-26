import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

export const PRIVATE_VISIBLE_TEXT = 'fixture-artifact.txt';
export const PRIVATE_ACCESSIBLE_NAME = 'private account label fixture-artifact.txt';
export const PRIVATE_HREF = 'https://example.invalid/private/signed?token=secret';
export const PRIVATE_TURN_TEXT = `Assistant final text with private prose and ${PRIVATE_VISIBLE_TEXT}`;
export const PRIVATE_CLASS = 'private-account-scoped-css-class';

globalThis.HTMLAnchorElement ||= class HTMLAnchorElement {};
globalThis.HTMLButtonElement ||= class HTMLButtonElement {};

export function enoent(message = 'download.saveAs: ENOENT: no such file or directory, copyfile /tmp/playwright-artifacts-x/source -> /broker-artifacts/session/downloads/001-fixture-artifact.txt') {
  const error = new Error(message);
  error.code = 'ENOENT';
  return error;
}

function fakeTextContext(text, box = { x: 0, y: 0, width: 600, height: 120 }) {
  return {
    innerText: text,
    textContent: text,
    getBoundingClientRect: () => box,
  };
}

function fakeDomNode(candidate = {}, index = 0) {
  const tag = candidate.tag || 'button';
  const node = {
    tagName: tag.toUpperCase(),
    innerText: candidate.innerText ?? candidate.visibleText ?? PRIVATE_VISIBLE_TEXT,
    textContent: candidate.textContent ?? candidate.visibleText ?? PRIVATE_VISIBLE_TEXT,
    getBoundingClientRect: () => candidate.boundingBox || { x: 10, y: 20 + index * 32, width: 120, height: 24 },
    getAttribute(name) {
      switch (name) {
        case 'role':
          return candidate.role || '';
        case 'class':
          return candidate.class ?? PRIVATE_CLASS;
        case 'aria-label':
          return candidate.accessibleName ?? PRIVATE_ACCESSIBLE_NAME;
        case 'download':
          return candidate.download ?? '';
        case 'href':
          return candidate.hrefDebug ?? PRIVATE_HREF;
        default:
          return '';
      }
    },
    closest(selector) {
      if (String(selector).includes('[tabindex="0"]')) {
        return candidate.fileCardOpener ? fakeTextContext(candidate.fileCardText ?? candidate.visibleText ?? PRIVATE_VISIBLE_TEXT) : null;
      }
      if (String(selector).includes('[data-message-author-role="assistant"]')) {
        return fakeTextContext(candidate.assistantTurnText ?? PRIVATE_TURN_TEXT);
      }
      return fakeTextContext(candidate.turnText ?? PRIVATE_TURN_TEXT);
    },
  };
  Object.setPrototypeOf(
    node,
    tag === 'a' ? globalThis.HTMLAnchorElement.prototype : globalThis.HTMLButtonElement.prototype,
  );
  return node;
}

export class FakeDownload {
  constructor({ suggestedFilename = 'fixture-artifact.txt', failWith = null, payload = 'fixture payload' } = {}) {
    this._suggestedFilename = suggestedFilename;
    this.failWith = failWith;
    this.payload = payload;
    this.saveAsCalls = 0;
  }

  suggestedFilename() {
    return this._suggestedFilename;
  }

  async saveAs(targetPath) {
    this.saveAsCalls += 1;
    if (this.failWith) throw typeof this.failWith === 'function' ? this.failWith() : this.failWith;
    await writeFile(targetPath, this.payload);
  }
}

class FakeLocator {
  constructor(page, kind, index = 0) {
    this.page = page;
    this.kind = kind;
    this.index = index;
  }

  candidate() {
    if (this.kind === 'candidate') return this.page.candidates[this.index] || {};
    if (this.kind === 'turnCandidate') {
      return this.page.candidatesForTurn(this.turnIndex)[this.index] || {};
    }
    if (this.kind === 'previewCandidate') return this.page.previewCandidates[this.index] || {};
    return {};
  }

  withTurnIndex(turnIndex) {
    this.turnIndex = turnIndex;
    return this;
  }

  async count() {
    if (this.kind === 'turns') return this.page.turnCount;
    if (this.kind === 'candidates') return this.page.candidates.length;
    if (this.kind === 'turnCandidates') return this.page.candidatesForTurn(this.turnIndex).length;
    if (this.kind === 'previewCandidates') return this.page.previewOpen ? this.page.previewCandidates.length : 0;
    return 0;
  }

  nth(index) {
    if (this.kind === 'turns') return new FakeLocator(this.page, 'turn', index);
    if (this.kind === 'candidates') return new FakeLocator(this.page, 'candidate', index);
    if (this.kind === 'turnCandidates') return new FakeLocator(this.page, 'turnCandidate', index).withTurnIndex(this.turnIndex);
    if (this.kind === 'previewCandidates') return new FakeLocator(this.page, 'previewCandidate', index);
    return new FakeLocator(this.page, 'empty');
  }

  async isVisible() {
    return this.kind === 'turn' || this.kind === 'candidate' || this.kind === 'turnCandidate'
      || (this.kind === 'previewCandidate' && this.page.previewOpen);
  }

  async evaluate(fn, arg) {
    if (this.kind === 'turn') return this.page.turnTexts[this.index] ?? PRIVATE_TURN_TEXT;
    if (this.kind === 'candidate' || this.kind === 'turnCandidate' || this.kind === 'previewCandidate') {
      return this.evaluateCandidate(fn, arg);
    }
    return '';
  }

  evaluateCandidate(fn, arg) {
    const candidate = this.candidate();
    const candidateTurnText = candidate.turnText
      ?? this.page.turnTexts[candidate.turnIndex ?? 0]
      ?? PRIVATE_TURN_TEXT;
    if (candidate.domBacked && typeof fn === 'function') {
      return fn(fakeDomNode(candidate, this.index), arg);
    }
    return {
      candidateIndex: this.index + 1,
      role: candidate.role || 'button',
      tag: candidate.tag || 'button',
      class: candidate.class ?? PRIVATE_CLASS,
      visibleText: candidate.visibleText ?? PRIVATE_VISIBLE_TEXT,
      accessibleName: candidate.accessibleName ?? PRIVATE_ACCESSIBLE_NAME,
      hrefDebug: candidate.hrefDebug ?? PRIVATE_HREF,
      boundingBox: candidate.boundingBox || { x: 10, y: 20 + this.index * 32, width: 120, height: 24 },
      turnTextSha256: '',
      assistantTurnTextSha256: '',
      turnText: candidateTurnText,
      assistantTurnText: candidate.assistantTurnText ?? candidateTurnText,
    };
  }

  async boundingBox() {
    return this.page.turnBoxes[this.index] || { x: 10, y: 20 + this.index * 200, width: 600, height: 120 };
  }

  locator() {
    if (this.kind === 'turn') return new FakeLocator(this.page, 'turnCandidates').withTurnIndex(this.index);
    return new FakeLocator(this.page, 'empty');
  }

  async click() {
    assert.ok(this.kind === 'candidate' || this.kind === 'turnCandidate' || this.kind === 'previewCandidate');
    this.page.clickedCandidates.push(this.candidate());
    this.page.clicks += 1;
    if (this.candidate().fileCardOpener) this.page.previewOpen = true;
  }
}

export class FakePage {
  constructor(downloads, candidates = [{}], options = {}) {
    this.downloads = downloads;
    this.candidates = candidates;
    this.turnCount = options.turnCount ?? Math.max(1, ...candidates.map(candidate => Number(candidate.turnIndex ?? 0) + 1));
    this.turnTexts = options.turnTexts ?? Array.from({ length: this.turnCount }, (_, index) => `${PRIVATE_TURN_TEXT} turn ${index}`);
    this.turnBoxes = options.turnBoxes ?? Array.from({ length: this.turnCount }, (_, index) => ({ x: 10, y: 20 + index * 300, width: 600, height: 120 }));
    this.waits = 0;
    this.clicks = 0;
    this.clickedCandidates = [];
    this.events = [];
    this.bottomScrolls = 0;
    this.previewCandidates = options.previewCandidates ?? [];
    this.previewOpen = false;
  }

  candidatesForTurn(turnIndex) {
    return this.candidates.filter(candidate => Number(candidate.turnIndex ?? 0) === Number(turnIndex));
  }

  locator(selector) {
    this.events.push(`locator:${selector}`);
    if (selector === '[data-message-author-role="assistant"]') return new FakeLocator(this, 'turns');
    if (selector === 'button.behavior-btn, button, [role="button"], a[download], [role="link"]') return new FakeLocator(this, 'previewCandidates');
    if (selector === 'button.behavior-btn, a[download], a[href], button, [role="button"], [role="link"]') return new FakeLocator(this, 'candidates');
    return new FakeLocator(this, 'empty');
  }

  async evaluate(fn) {
    if (fn?.name === 'scrollPrimaryConversationViewportInPage') {
      this.events.push('bottom-scroll');
      this.bottomScrolls += 1;
      return { schema: 'gpt-webai.bottom-scroll-gate.v1', status: 'at_bottom', selected: { id: 'conversation-scrollport' } };
    }
    return {};
  }

  waitForEvent(eventName) {
    assert.equal(eventName, 'download');
    const download = this.downloads[this.waits];
    this.waits += 1;
    if (!download) return Promise.reject(new Error('no fake download available'));
    return Promise.resolve(download);
  }
}

export function sha256TextFixture(text) {
  return createHash('sha256').update(text).digest('hex');
}

export async function withArtifactRoot(fn, options = {}) {
  const root = await mkdtemp(path.join(tmpdir(), 'artifact-reclick-test-'));
  const oldArtifactsDir = process.env.GPT_WEBAI_ARTIFACTS_DIR;
  const oldHostDir = process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR;
  const oldRetryDelay = process.env.GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_DELAY_MS;
  const oldRetryAttempts = process.env.GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_ATTEMPTS;
  const oldRetryMaxDelay = process.env.GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_MAX_DELAY_MS;
  const oldBrowserDownloadDir = process.env.GPT_WEBAI_BROWSER_DOWNLOAD_DIR;
  const oldBottomAttempts = process.env.GPT_WEBAI_BOTTOM_SCROLL_ATTEMPTS;
  const oldBottomDelay = process.env.GPT_WEBAI_BOTTOM_SCROLL_DELAY_MS;
  process.env.GPT_WEBAI_ARTIFACTS_DIR = root;
  process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR = root;
  process.env.GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_DELAY_MS = options.retryDelay ?? '1';
  process.env.GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_MAX_DELAY_MS = options.retryMaxDelay ?? '4';
  if (options.browserDownloadDir === null) delete process.env.GPT_WEBAI_BROWSER_DOWNLOAD_DIR;
  else process.env.GPT_WEBAI_BROWSER_DOWNLOAD_DIR = options.browserDownloadDir ?? '';
  if (options.retryAttempts === null) delete process.env.GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_ATTEMPTS;
  else process.env.GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_ATTEMPTS = options.retryAttempts ?? '2';
  process.env.GPT_WEBAI_BOTTOM_SCROLL_ATTEMPTS = options.bottomScrollAttempts ?? '1';
  process.env.GPT_WEBAI_BOTTOM_SCROLL_DELAY_MS = options.bottomScrollDelay ?? '1';
  try {
    return await fn(root);
  } finally {
    if (oldArtifactsDir === undefined) delete process.env.GPT_WEBAI_ARTIFACTS_DIR;
    else process.env.GPT_WEBAI_ARTIFACTS_DIR = oldArtifactsDir;
    if (oldHostDir === undefined) delete process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR;
    else process.env.GPT_WEBAI_ARTIFACTS_HOST_DIR = oldHostDir;
    if (oldRetryDelay === undefined) delete process.env.GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_DELAY_MS;
    else process.env.GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_DELAY_MS = oldRetryDelay;
    if (oldRetryAttempts === undefined) delete process.env.GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_ATTEMPTS;
    else process.env.GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_ATTEMPTS = oldRetryAttempts;
    if (oldRetryMaxDelay === undefined) delete process.env.GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_MAX_DELAY_MS;
    else process.env.GPT_WEBAI_DOWNLOAD_ENOENT_RECLICK_MAX_DELAY_MS = oldRetryMaxDelay;
    if (oldBrowserDownloadDir === undefined) delete process.env.GPT_WEBAI_BROWSER_DOWNLOAD_DIR;
    else process.env.GPT_WEBAI_BROWSER_DOWNLOAD_DIR = oldBrowserDownloadDir;
    if (oldBottomAttempts === undefined) delete process.env.GPT_WEBAI_BOTTOM_SCROLL_ATTEMPTS;
    else process.env.GPT_WEBAI_BOTTOM_SCROLL_ATTEMPTS = oldBottomAttempts;
    if (oldBottomDelay === undefined) delete process.env.GPT_WEBAI_BOTTOM_SCROLL_DELAY_MS;
    else process.env.GPT_WEBAI_BOTTOM_SCROLL_DELAY_MS = oldBottomDelay;
    await rm(root, { recursive: true, force: true });
  }
}
