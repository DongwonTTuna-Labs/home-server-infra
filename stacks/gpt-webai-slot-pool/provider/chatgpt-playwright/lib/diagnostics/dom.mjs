import { scrollPrimaryConversationToBottom } from '../bottom-scroll.mjs';
import { sanitizeDomDiagnostics } from './sanitize.mjs';

export async function pageDiagnostics(page, { label = 'capture', sessionId = '' } = {}) {
  const bottomScroll = await scrollPrimaryConversationToBottom(page, { label: `${label}:dom` });
  const title = await page.title().catch(() => '');
  const raw = await page.evaluate(({ captureLabel, captureSessionId, captureTitle }) => {
    const providerLimitPattern = /too many requests|request limit|rate limit|usage limit|temporarily limited|message cap|try again later|you(?:'|’| have)?ve? reached (?:the )?.{0,80}limit/i;
    const providerSurfaceSelector = '[role="dialog"],dialog,[aria-modal="true"],[role="alert"],[data-testid*="toast" i],[class*="toast" i],[aria-live]';
    const ignoredLimitContextSelectors = [
      'main',
      'main article',
      'article',
      '[data-message-author-role]',
      '[data-testid*="conversation" i]',
      '[data-testid*="thread" i]',
      'aside',
      'nav',
      '[data-testid*="sidebar" i]',
      '[data-testid*="history" i]',
      '#prompt-textarea',
      'textarea',
      '[contenteditable="true"]',
      '[role="textbox"]',
      '[class*="composer" i]',
      '[data-testid*="composer" i]',
      '[class*="attachment" i]',
      '[data-testid*="attachment" i]',
    ];
    const visible = node => {
      if (!node || typeof node.getBoundingClientRect !== 'function') return false;
      const rect = node.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return false;
      const style = window.getComputedStyle?.(node);
      return !style || (style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0');
    };
    const textOf = node => (node?.innerText || node?.textContent || '').trim();
    const attr = (node, name) => (typeof node?.getAttribute === 'function' ? node.getAttribute(name) || '' : '');
    const matches = (node, selector) => Boolean(node && typeof node.matches === 'function' && node.matches(selector));
    const closest = (node, selector) => (node && typeof node.closest === 'function' ? node.closest(selector) : null);
    const hasClosest = (node, selectors) => selectors.some(selector => Boolean(closest(node, selector)));
    const numberValue = (...values) => {
      for (const value of values) {
        const number = Number(value);
        if (Number.isFinite(number)) return number;
      }
      return 0;
    };
    const rectOf = node => {
      const rect = node.getBoundingClientRect();
      const x = numberValue(rect.x, rect.left);
      const y = numberValue(rect.y, rect.top);
      const width = numberValue(rect.width);
      const height = numberValue(rect.height);
      const left = numberValue(rect.left, x);
      const top = numberValue(rect.top, y);
      const right = numberValue(rect.right, left + width);
      const bottom = numberValue(rect.bottom, top + height);
      return {
        x: Math.round(x),
        y: Math.round(y),
        left: Math.round(left),
        top: Math.round(top),
        right: Math.round(right),
        bottom: Math.round(bottom),
        width: Math.round(width),
        height: Math.round(height),
      };
    };
    const digestText = value => {
      const text = String(value || '');
      return { length: text.length, sha256: '' };
    };
    const surfaceKind = node => {
      const role = attr(node, 'role').toLowerCase();
      const tag = String(node?.tagName || '').toLowerCase();
      if (role === 'dialog' || tag === 'dialog' || attr(node, 'aria-modal') === 'true') return 'dialog';
      if (role === 'alert') return 'alert';
      if (matches(node, '[data-testid*="toast" i],[class*="toast" i]')) return 'toast';
      if (attr(node, 'aria-live')) return 'live';
      return 'unknown';
    };
    const isNarrowToastOrLiveSurface = (node, rect) => {
      const kind = surfaceKind(node);
      if (!['toast', 'live'].includes(kind)) return false;
      if (hasClosest(node, ignoredLimitContextSelectors)) return false;
      const viewportWidth = Number(window.innerWidth || 0);
      const broadWidth = viewportWidth > 0 ? Math.min(viewportWidth * 0.55, 720) : 720;
      return rect.width > 0 && rect.height > 0 && rect.width <= broadWidth && rect.height <= 260;
    };
    const isBlockingProviderSurface = node => {
      if (hasClosest(node, ignoredLimitContextSelectors)) return false;
      const kind = surfaceKind(node);
      const rect = rectOf(node);
      if (['dialog', 'alert'].includes(kind)) return true;
      return isNarrowToastOrLiveSurface(node, rect);
    };
    const buttonsForSurface = node => {
      const controls = typeof node?.querySelectorAll === 'function'
        ? Array.from(node.querySelectorAll('button,a,[role="button"]')).filter(visible).slice(0, 8)
        : [];
      return controls.map((control, index) => ({
        index,
        tag: String(control.tagName || '').toLowerCase(),
        role: attr(control, 'role'),
        text: textOf(control).slice(0, 160),
        label: attr(control, 'aria-label').slice(0, 160),
        rect: rectOf(control),
      }));
    };
    const viewport = {
      width: Math.round(Number(window.innerWidth || document.documentElement?.clientWidth || 0)),
      height: Math.round(Number(window.innerHeight || document.documentElement?.clientHeight || 0)),
    };
    const bottomGapPx = rect => (viewport.height > 0 && rect ? Math.max(0, viewport.height - Number(rect.bottom || (rect.y + rect.height) || 0)) : null);
    const nearBottom = rect => {
      const gap = bottomGapPx(rect);
      if (gap === null) return false;
      return gap <= Math.max(180, Math.round(viewport.height * 0.28));
    };
    const rectEvidence = (node, extra = {}) => {
      if (!node) return { visible: false, ...extra };
      const rect = rectOf(node);
      return {
        visible: true,
        nearBottom: nearBottom(rect),
        bottomGapPx: bottomGapPx(rect),
        rect,
        ...extra,
      };
    };
    const bodyText = document.body?.innerText || '';
    const allControls = Array.from(document.querySelectorAll('button,a,[role="button"],input[type="file"],textarea,[contenteditable="true"]'))
      .filter(visible);
    const stopControls = allControls.filter(control => /stop generating|stop responding|stop answering|stop-button|중지|정지/i.test(`${textOf(control)} ${attr(control, 'aria-label')} ${attr(control, 'data-testid')}`));
    const controls = allControls
      .slice(0, 80)
      .map((node, index) => ({
        index,
        tag: String(node.tagName || '').toLowerCase(),
        role: attr(node, 'role'),
        type: attr(node, 'type'),
        testid: attr(node, 'data-testid'),
        text: textOf(node),
        label: attr(node, 'aria-label'),
        title: attr(node, 'title'),
        rect: rectOf(node),
        disabled: Boolean(node.disabled || attr(node, 'aria-disabled') === 'true'),
      }));
    const dialogs = Array.from(document.querySelectorAll('[role="dialog"],dialog,[aria-modal="true"]'))
      .filter(visible)
      .slice(0, 12)
      .map((node, index) => ({
        index,
        tag: String(node.tagName || '').toLowerCase(),
        role: attr(node, 'role'),
        className: String(node.className || ''),
        text: textOf(node).slice(0, 500),
        rect: rectOf(node),
      }));
    const providerLimitSurfaces = Array.from(document.querySelectorAll(providerSurfaceSelector))
      .filter(visible)
      .filter(isBlockingProviderSurface)
      .map((node, index) => ({
        index,
        tag: String(node.tagName || '').toLowerCase(),
        role: attr(node, 'role'),
        kind: surfaceKind(node),
        className: String(node.className || ''),
        text: textOf(node).slice(0, 500),
        rect: rectOf(node),
        actionButtons: buttonsForSurface(node),
      }))
      .filter(surface => providerLimitPattern.test(surface.text))
      .slice(0, 12);
    const assistants = Array.from(document.querySelectorAll('[data-message-author-role="assistant"], main article'))
      .filter(visible)
      .slice(-8)
      .map((node, index) => ({
        index,
        tag: String(node.tagName || '').toLowerCase(),
        domId: node.id || attr(node, 'data-testid'),
        text: textOf(node),
        rect: rectOf(node),
      }));
    const users = Array.from(document.querySelectorAll('[data-message-author-role="user"]'))
      .filter(visible)
      .slice(-8)
      .map((node, index) => ({
        index,
        tag: String(node.tagName || '').toLowerCase(),
        domId: node.id || attr(node, 'data-testid'),
        text: textOf(node),
        rect: rectOf(node),
      }));
    const allTurns = Array.from(document.querySelectorAll('[data-message-author-role="user"],[data-message-author-role="assistant"], main article'))
      .filter(visible)
      .map((node, index) => {
        const role = attr(node, 'data-message-author-role');
        const rect = rectOf(node);
        const text = textOf(node);
        return {
          index,
          kind: role === 'user' || role === 'assistant' ? role : 'assistant',
          tag: String(node.tagName || '').toLowerCase(),
          domId: node.id || attr(node, 'data-testid'),
          text,
          textLength: text.length,
          rect,
          visible: true,
          nearBottom: nearBottom(rect),
          bottomGapPx: bottomGapPx(rect),
        };
      });
    const latestTurn = allTurns[allTurns.length - 1] || null;
    const fileInputs = Array.from(document.querySelectorAll('input[type="file"]'));
    const textboxes = Array.from(document.querySelectorAll('#prompt-textarea, textarea, [contenteditable="true"][role="textbox"], .ProseMirror[contenteditable="true"], [contenteditable="true"]')).filter(visible);
    const login = /Thanks for trying ChatGPT|Log in or sign up|Log in to get answers|Sign up for free|get smarter responses, upload files|^Log in$/im.test(bodyText);
    const providerLimitText = providerLimitSurfaces.map(surface => surface.text).join('\n');
    const limit = providerLimitPattern.test(providerLimitText);
    const upgrade = /\bUpgrade\b|Get Plus|Go Plus|Free plan/i.test(bodyText);
    const pro = /\bPro Extended\b|\bPro\b|프로/i.test(bodyText);
    const url = String(window.location?.href || '');
    const escapeRegExp = value => String(value || '').replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const sessionUrlMatches = captureSessionId
      ? new RegExp(`^https://(?:www\\.)?chatgpt\\.com/c/${escapeRegExp(captureSessionId)}(?:[/?#]|$)`).test(url)
      : true;
    const rootUrl = /^https:\/\/(?:www\.)?chatgpt\.com\/?(?:[?#].*)?$/.test(url);
    const conversationUrl = /^https:\/\/(?:www\.)?chatgpt\.com\/c\/[^/?#]+/.test(url);
    const urlKind = conversationUrl ? 'conversation' : rootUrl ? 'root' : 'other';
    const composerNode = textboxes[textboxes.length - 1] || null;
    const composerEvidence = rectEvidence(composerNode, {
      count: textboxes.length,
      disabled: composerNode ? Boolean(composerNode.disabled || attr(composerNode, 'aria-disabled') === 'true') : undefined,
    });
    const activeGenerationNode = stopControls[stopControls.length - 1] || null;
    const activeGenerationControl = rectEvidence(activeGenerationNode, { count: stopControls.length });
    const authenticatedComposerReadyAtBottom = !login && !limit && pro && composerEvidence.visible === true && composerEvidence.nearBottom === true;
    const activeGenerationAtBottom = stopControls.some(control => nearBottom(rectOf(control)));
    const newestTurnAtBottom = Boolean(latestTurn?.nearBottom);
    const evidenceKinds = [];
    if (authenticatedComposerReadyAtBottom) evidenceKinds.push('authenticated_composer_ready_at_bottom');
    if (activeGenerationAtBottom) evidenceKinds.push('active_generation_at_bottom');
    if (newestTurnAtBottom) evidenceKinds.push('newest_turn_at_bottom');
    const readinessReason = !sessionUrlMatches
      ? 'session_url_mismatch'
      : evidenceKinds.length > 0
        ? ''
        : 'bottom_readiness_evidence_missing';
    return {
      schema: 'gpt-webai-provider-dom-diagnostics.v1',
      capturedAt: new Date().toISOString(),
      label: captureLabel,
      sessionId: captureSessionId,
      url,
      title: captureTitle,
      bodyTextPreview: bodyText.slice(0, 2000),
      bodyText: digestText(bodyText),
      readinessSignals: {
        login,
        limit,
        upgrade,
        pro,
        composer: textboxes.length > 0,
        stopControls: stopControls.length,
        dialogs: dialogs.length,
        providerLimitSurfaceCount: providerLimitSurfaces.length,
        fileInputs: fileInputs.length,
        textboxes: textboxes.length,
      },
      selectorInventory: {
        controls: controls.length,
        dialogs: dialogs.length,
        fileInputs: fileInputs.length,
        textboxes: textboxes.length,
        assistantTurns: assistants.length,
        userTurns: users.length,
      },
      controls,
      dialogs,
      providerLimitSurfaces,
      assistantTurns: assistants,
      userTurns: users,
      bottomReadinessEvidence: {
        schema: 'gpt-webai.bottom-readiness-evidence.v1',
        label: captureLabel,
        status: !readinessReason ? 'verified' : 'unverified',
        reason: readinessReason || undefined,
        urlKind,
        sessionIdPresent: Boolean(captureSessionId),
        sessionUrlMatches,
        authenticatedComposerReadyAtBottom,
        activeGenerationAtBottom,
        newestTurnAtBottom,
        evidenceKinds,
        viewport,
        composer: composerEvidence,
        activeGenerationControl,
        newestTurn: latestTurn ? {
          kind: latestTurn.kind,
          visible: latestTurn.visible,
          nearBottom: latestTurn.nearBottom,
          bottomGapPx: latestTurn.bottomGapPx,
          rect: latestTurn.rect,
          textLength: latestTurn.textLength,
          text: latestTurn.text,
        } : undefined,
      },
    };
  }, { captureLabel: label, captureSessionId: sessionId, captureTitle: title });
  return sanitizeDomDiagnostics(raw, bottomScroll);
}
