import assert from 'node:assert/strict';
import test from 'node:test';

import { scrollPrimaryConversationViewportInPage } from '../lib/bottom-scroll.mjs';

function rect({ x, y, width, height }) {
  return { x, y, left: x, top: y, right: x + width, bottom: y + height, width, height };
}

function fakeNode({ tagName = 'div', id = '', role = '', testid = '', className = '', text = '', label = '', title = '', hasSvg = false, box, scrollHeight = 0, clientHeight = 0, scrollTop = 0, overflowY = 'visible' } = {}) {
  const node = {
    tagName: tagName.toUpperCase(),
    id,
    role,
    testid,
    className,
    innerText: text,
    textContent: text,
    parentNode: null,
    children: [],
    scrollHeight,
    clientHeight,
    scrollTop,
    scrollLeft: 0,
    style: { display: 'block', visibility: 'visible', opacity: '1', overflowY },
    append(child) {
      child.parentNode = this;
      this.children.push(child);
      return child;
    },
    contains(target) {
      if (target === this) return true;
      return this.children.some(child => child.contains(target));
    },
    getAttribute(name) {
      if (name === 'role') return this.role;
      if (name === 'data-testid') return this.testid;
      if (name === 'aria-label') return label;
      if (name === 'title') return title;
      if (name === 'class') return this.className;
      return '';
    },
    querySelector(selector) {
      if (selector === 'svg' && hasSvg) return { tagName: 'svg' };
      return null;
    },
    getBoundingClientRect() {
      return rect(box || { x: 0, y: 0, width: 0, height: 0 });
    },
    matches(selector) {
      if (selector.includes('sidebar')) return /sidebar/i.test(this.testid);
      if (selector.includes('history')) return /history/i.test(this.testid);
      if (selector.includes('[role="navigation"]')) return this.role === 'navigation';
      return false;
    },
    closest(selector) {
      let current = this;
      while (current) {
        const tag = String(current.tagName || '').toLowerCase();
        if (selector.includes('aside') && tag === 'aside') return current;
        if (selector.includes('nav') && tag === 'nav') return current;
        if (selector.includes('[role="navigation"]') && current.role === 'navigation') return current;
        if (selector.includes('main') && tag === 'main') return current;
        if (selector.includes('sidebar') && /sidebar/i.test(current.testid)) return current;
        if (selector.includes('history') && /history/i.test(current.testid)) return current;
        current = current.parentNode;
      }
      return null;
    },
    scrollTo(options = {}) {
      this.scrollTop = Number(options.top || 0);
    },
  };
  return node;
}

function withFakeDocument({ nodes, main, body }, fn) {
  const oldDocument = globalThis.document;
  const oldWindow = globalThis.window;
  globalThis.document = {
    body,
    documentElement: body,
    scrollingElement: body,
    querySelector(selector) {
      if (selector === 'main') return main;
      return null;
    },
    querySelectorAll(selector) {
      if (selector === '*') return nodes;
      if (selector.includes('scroll-root') && selector.includes('scrollbar')) {
        return nodes.filter(node => /scroll-root/i.test(node.className || '') && /scrollbar/i.test(node.className || ''));
      }
      if (selector.includes('button') || selector.includes('[role="button"]') || selector.includes('a[')) {
        return nodes.filter(node => ['button', 'a'].includes(String(node.tagName || '').toLowerCase()) || node.role === 'button');
      }
      return [];
    },
  };
  globalThis.window = {
    innerWidth: 1280,
    innerHeight: 720,
    getComputedStyle(node) {
      return node.style;
    },
  };
  try {
    return fn();
  } finally {
    globalThis.document = oldDocument;
    globalThis.window = oldWindow;
  }
}

test('bottom-scroll gate scrolls the visible browser viewport root and the main conversation scrollport', () => {
  const body = fakeNode({ tagName: 'body', box: { x: 0, y: 0, width: 1280, height: 720 }, scrollHeight: 2000, clientHeight: 720, overflowY: 'auto' });
  const sidebar = body.append(fakeNode({ tagName: 'aside', id: 'left-sidebar', testid: 'history-sidebar', box: { x: 0, y: 0, width: 260, height: 720 }, scrollHeight: 2200, clientHeight: 720, overflowY: 'auto' }));
  const conversationScrollport = body.append(fakeNode({ tagName: 'div', id: 'conversation-scrollport', testid: 'conversation-thread', box: { x: 300, y: 0, width: 900, height: 720 }, scrollHeight: 2400, clientHeight: 720, overflowY: 'auto' }));
  const main = conversationScrollport.append(fakeNode({ tagName: 'main', id: 'main-chat', box: { x: 300, y: 0, width: 900, height: 720 }, scrollHeight: 720, clientHeight: 720, overflowY: 'visible' }));

  const result = withFakeDocument({ nodes: [body, sidebar, conversationScrollport, main], main, body }, () => scrollPrimaryConversationViewportInPage());

  assert.equal(result.status, 'at_bottom');
  assert.equal(result.selected.selectionKind, 'browser_viewport_scrollbar');
  assert.equal(result.selected.rootKind, 'document.scrollingElement');
  assert.equal(result.primary.id, 'conversation-scrollport');
  assert.equal(conversationScrollport.scrollTop, 1680);
  assert.equal(body.scrollTop, 1280);
  assert.equal(sidebar.scrollTop, 0);
});


test('bottom-scroll gate uses the ChatGPT scroll-root scrollbar when the browser viewport root has no scroll range', () => {
  const body = fakeNode({ tagName: 'body', box: { x: 0, y: 0, width: 1280, height: 720 }, scrollHeight: 720, clientHeight: 720, overflowY: 'visible' });
  const genericContainer = body.append(fakeNode({ tagName: 'div', id: 'generic-conversation', testid: 'conversation-thread', box: { x: 260, y: 0, width: 900, height: 720 }, scrollHeight: 3000, clientHeight: 720, overflowY: 'auto' }));
  const scrollRoot = body.append(fakeNode({ tagName: 'div', id: 'actual-scroll-root', className: 'scroll-root overflow-y-auto scrollbar', box: { x: 280, y: 0, width: 1000, height: 720 }, scrollHeight: 1900, clientHeight: 720, overflowY: 'auto' }));
  const main = scrollRoot.append(fakeNode({ tagName: 'main', id: 'main-chat', box: { x: 280, y: 0, width: 1000, height: 720 }, scrollHeight: 720, clientHeight: 720, overflowY: 'visible' }));

  const result = withFakeDocument({ nodes: [body, genericContainer, scrollRoot, main], main, body }, () => scrollPrimaryConversationViewportInPage());

  assert.equal(result.status, 'at_bottom');
  assert.equal(result.selected.id, 'actual-scroll-root');
  assert.equal(result.selected.scrollTop, 1180);
  assert.equal(result.selected.maxScrollTop, 1180);
  assert.equal(result.selected.selectionKind, 'chatgpt_scroll_root_scrollbar');
  assert.equal(result.selected.classHints.containsScrollRoot, true);
  assert.equal(result.selected.classHints.containsScrollbar, true);
  assert.equal(result.visualScrollbarProof.status, 'right_edge_scrollbar_at_bottom');
  assert.equal(result.visualScrollbarProof.atBottom, true);
  assert.equal(body.scrollTop, 0);
  assert.equal(genericContainer.scrollTop, 0);
});

test('bottom-scroll gate reports unavailable visual scrollbar proof when selected element is not right-edge scroll-root scrollbar', () => {
  const body = fakeNode({ tagName: 'body', box: { x: 0, y: 0, width: 1280, height: 720 }, scrollHeight: 720, clientHeight: 720, overflowY: 'visible' });
  const conversationScrollport = body.append(fakeNode({ tagName: 'div', id: 'conversation-scrollport', testid: 'conversation-thread', box: { x: 300, y: 0, width: 900, height: 720 }, scrollHeight: 2400, clientHeight: 720, overflowY: 'auto' }));
  const main = conversationScrollport.append(fakeNode({ tagName: 'main', id: 'main-chat', box: { x: 300, y: 0, width: 900, height: 720 }, scrollHeight: 720, clientHeight: 720, overflowY: 'visible' }));

  const result = withFakeDocument({ nodes: [body, conversationScrollport, main], main, body }, () => scrollPrimaryConversationViewportInPage());

  assert.equal(result.status, 'at_bottom');
  assert.equal(result.selected.id, 'conversation-scrollport');
  assert.equal(result.visualScrollbarProof.status, 'unavailable');
  assert.equal(result.visualScrollbarProof.reason, 'selected_element_missing_scroll_root_scrollbar_class');
});

test('bottom-scroll gate refuses at_bottom while a floating more-content affordance is visible', () => {
  const body = fakeNode({ tagName: 'body', box: { x: 0, y: 0, width: 1280, height: 720 }, scrollHeight: 720, clientHeight: 720, overflowY: 'visible' });
  const scrollRoot = body.append(fakeNode({ tagName: 'div', id: 'actual-scroll-root', className: 'scroll-root overflow-y-auto scrollbar', box: { x: 280, y: 0, width: 1000, height: 720 }, scrollHeight: 1900, clientHeight: 720, overflowY: 'auto' }));
  const main = scrollRoot.append(fakeNode({ tagName: 'main', id: 'main-chat', box: { x: 280, y: 0, width: 1000, height: 720 }, scrollHeight: 720, clientHeight: 720, overflowY: 'visible' }));
  const downArrow = body.append(fakeNode({ tagName: 'button', role: 'button', label: 'Scroll to bottom', hasSvg: true, box: { x: 622, y: 560, width: 36, height: 36 }, scrollHeight: 0, clientHeight: 0, overflowY: 'visible' }));

  const result = withFakeDocument({ nodes: [body, scrollRoot, main, downArrow], main, body }, () => scrollPrimaryConversationViewportInPage());

  assert.equal(result.status, 'more_content_affordance_visible');
  assert.equal(result.selected.id, 'actual-scroll-root');
  assert.equal(result.selected.atBottom, true);
  assert.equal(result.moreContentAffordances.length, 1);
  assert.equal(result.moreContentAffordances[0].labelPreview, 'Scroll to bottom');
});


test('bottom-scroll gate ignores sidebar/history affordances while preserving main at-bottom proof', () => {
  const body = fakeNode({ tagName: 'body', box: { x: 0, y: 0, width: 1280, height: 720 }, scrollHeight: 720, clientHeight: 720, overflowY: 'visible' });
  const sidebar = body.append(fakeNode({ tagName: 'aside', id: 'left-sidebar', testid: 'history-sidebar', box: { x: 0, y: 0, width: 252, height: 720 }, scrollHeight: 2200, clientHeight: 720, overflowY: 'auto' }));
  const historyLink = sidebar.append(fakeNode({ tagName: 'a', text: 'PR72 Scroll Bottom Fix', label: 'PR72 Scroll Bottom Fix', box: { x: 6, y: 552, width: 233, height: 36 }, overflowY: 'visible' }));
  const pinButton = sidebar.append(fakeNode({ tagName: 'button', label: 'Pin PR72 Scroll Bottom Fix', box: { x: -10, y: 552, width: 34, height: 36 }, overflowY: 'visible' }));
  const optionsButton = sidebar.append(fakeNode({ tagName: 'button', testid: 'history-item-0-options', label: 'Open conversation options for PR72 Scroll Bottom Fix', box: { x: 14, y: 552, width: 34, height: 36 }, overflowY: 'visible' }));
  const scrollRoot = body.append(fakeNode({ tagName: 'div', id: 'actual-scroll-root', className: 'scroll-root overflow-y-auto scrollbar', box: { x: 280, y: 0, width: 1000, height: 720 }, scrollHeight: 1900, clientHeight: 720, overflowY: 'auto' }));
  const main = scrollRoot.append(fakeNode({ tagName: 'main', id: 'main-chat', box: { x: 280, y: 0, width: 1000, height: 720 }, scrollHeight: 720, clientHeight: 720, overflowY: 'visible' }));

  const result = withFakeDocument({ nodes: [body, sidebar, historyLink, pinButton, optionsButton, scrollRoot, main], main, body }, () => scrollPrimaryConversationViewportInPage());

  assert.equal(result.status, 'at_bottom');
  assert.equal(result.selected.id, 'actual-scroll-root');
  assert.equal(result.moreContentAffordances.length, 0);
  assert.ok(result.ignoredMoreContentAffordances.length >= 3);
  assert.equal(result.ignoredMoreContentAffordances[0].scope, 'sidebar_or_navigation');
});

test('bottom-scroll gate ignores detached left-history controls when short conversation has no scrollport', () => {
  const body = fakeNode({ tagName: 'body', box: { x: 0, y: 0, width: 1020, height: 703 }, scrollHeight: 703, clientHeight: 703, overflowY: 'visible' });
  const main = body.append(fakeNode({ tagName: 'main', id: 'main-chat', box: { x: 52, y: 0, width: 968, height: 703 }, scrollHeight: 703, clientHeight: 703, overflowY: 'visible' }));
  const historyLink = body.append(fakeNode({ tagName: 'a', text: 'PR72 Scroll Bottom Fix', label: 'PR72 Scroll Bottom Fix', box: { x: 6, y: 696, width: 233, height: 36 }, overflowY: 'visible' }));
  const pinButton = body.append(fakeNode({ tagName: 'button', label: 'Pin PR72 Scroll Bottom Fix', box: { x: -10, y: 684, width: 34, height: 36 }, overflowY: 'visible' }));
  const optionsButton = body.append(fakeNode({ tagName: 'button', testid: 'history-item-7-options', label: 'Open conversation options for PR72 Scroll Bottom Fix', box: { x: 14, y: 684, width: 34, height: 36 }, overflowY: 'visible' }));

  const result = withFakeDocument({ nodes: [body, main, historyLink, pinButton, optionsButton], main, body }, () => scrollPrimaryConversationViewportInPage());

  assert.equal(result.status, 'scrollport_not_found');
  assert.equal(result.candidateCount, 0);
  assert.equal(result.moreContentAffordances.length, 0);
  assert.equal(result.ignoredMoreContentAffordances.length, 3);
  assert.equal(result.ignoredMoreContentAffordances[0].ignoredReason, 'sidebar_or_navigation');
});
