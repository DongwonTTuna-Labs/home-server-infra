
import { writeFile } from 'node:fs/promises';

export function fakeRawDiagnostics() {
  return {
    schema: 'gpt-webai-provider-dom-diagnostics.v1',
    capturedAt: '2026-07-07T00:00:00.000Z',
    label: 'unit',
    sessionId: 'sid-unit',
    url: 'https://chatgpt.com/c/sid-unit',
    title: 'Unit',
    bodyTextPreview: 'Safe visible body preview',
    bodyText: { length: 24, sha256: '' },
    readinessSignals: {
      login: false,
      limit: false,
      upgrade: false,
      pro: true,
      composer: true,
      stopControls: 1,
      dialogs: 0,
      fileInputs: 1,
      textboxes: 1,
    },
    selectorInventory: {
      controls: 1,
      dialogs: 0,
      fileInputs: 1,
      textboxes: 1,
      assistantTurns: 1,
    },
    controls: [{
      index: 0,
      tag: 'button',
      role: 'button',
      type: '',
      testid: 'send-button',
      text: 'private visible control text',
      label: 'private label',
      title: 'private title',
      rect: { x: 1, y: 2, width: 3, height: 4 },
      disabled: false,
    }],
    dialogs: [],
    assistantTurns: [{
      index: 0,
      tag: 'article',
      domId: 'turn',
      text: 'assistant answer preview',
      rect: { x: 5, y: 6, width: 7, height: 8 },
    }],
  };
}

export function fakePage() {
  return {
    async title() {
      return 'Unit';
    },
    url() {
      return 'https://chatgpt.com/c/sid-unit';
    },
    async evaluate(_fn, args = {}) {
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
    async screenshot({ path: screenshotPath }) {
      await writeFile(screenshotPath, 'fake png bytes');
    },
  };
}

export function visibleButton(attributes = {}) {
  return {
    tagName: 'BUTTON',
    innerText: attributes.text || '',
    textContent: attributes.text || '',
    disabled: false,
    style: {
      display: 'flex',
      visibility: 'visible',
      opacity: '1',
    },
    getAttribute(name) {
      return attributes[name] || '';
    },
    getBoundingClientRect() {
      return { x: 1, y: 1, width: 36, height: 36 };
    },
  };
}


export function visibleSurface(attributes = {}, options = {}) {
  return {
    tagName: attributes.tagName || 'DIV',
    innerText: attributes.text || '',
    textContent: attributes.text || '',
    className: attributes.className || '',
    style: {
      display: 'flex',
      visibility: 'visible',
      opacity: '1',
    },
    getAttribute(name) {
      return attributes[name] || '';
    },
    getBoundingClientRect() {
      return attributes.rect || { x: 12, y: 18, width: 420, height: 120 };
    },
    matches(selector) {
      if (selector.includes('toast')) {
        return /toast/i.test(`${attributes.className || ''} ${attributes['data-testid'] || ''}`);
      }
      if (selector.includes('[aria-live]')) return Boolean(attributes['aria-live']);
      return false;
    },
    closest(selector) {
      return (options.closestSelectors || []).includes(selector) ? { tagName: 'MATCH' } : null;
    },
    querySelectorAll(selector) {
      if (selector.includes('button') || selector.includes('[role="button"]') || selector.includes('a,')) {
        return options.buttons || [];
      }
      return [];
    },
  };
}

export function fakeDomPage(nodes, options = {}) {
  return {
    async title() {
      return 'Unit DOM';
    },
    async evaluate(fn, args = {}) {
      const oldDocument = globalThis.document;
      const oldWindow = globalThis.window;
      globalThis.document = {
        body: { innerText: options.bodyText || 'Dongwon Lee\nPro\nPro Extended' },
        querySelectorAll(selector) {
          if (selector.includes('[role="dialog"]') || selector.includes('[role="alert"]') || selector.includes('[aria-live]')) {
            return options.providerLimitSurfaces || [];
          }
          if (selector.includes('button')) return nodes;
          if (selector.includes('textarea') || selector.includes('prompt-textarea')) return [];
          return [];
        },
      };
      globalThis.window = {
        location: { href: 'https://chatgpt.com/c/sid-unit' },
        getComputedStyle(node) {
          return node.style;
        },
      };
      try {
        return fn(args);
      } finally {
        globalThis.document = oldDocument;
        globalThis.window = oldWindow;
      }
    },
  };
}
