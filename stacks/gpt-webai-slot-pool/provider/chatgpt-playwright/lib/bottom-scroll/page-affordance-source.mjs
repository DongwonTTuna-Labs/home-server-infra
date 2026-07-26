export const BOTTOM_SCROLL_AFFORDANCE_SOURCE = `  const textOf = node => (node?.innerText || node?.textContent || '').trim();
  const hasSvg = node => {
    try {
      return Boolean(typeof node?.querySelector === 'function' && node.querySelector('svg'));
    } catch {
      return false;
    }
  };
  const composerAncestor = node => safeClosest(node, [
    '#prompt-textarea',
    'textarea',
    '[contenteditable="true"]',
    '[role="textbox"]',
    'form',
    '[class*="composer" i]',
    '[data-testid*="composer" i]',
    '[data-testid*="prompt" i]',
  ].join(','));
  const moreContentAffordanceMatch = node => {
    const rect = safeRect(node);
    if (!rect || rect.width <= 0 || rect.height <= 0) return null;
    if (composerAncestor(node)) return null;
    const signature = [
      textOf(node),
      safeAttr(node, 'aria-label'),
      safeAttr(node, 'title'),
      safeAttr(node, 'data-testid'),
      safeAttr(node, 'data-state'),
      classNameOf(node),
    ].join(' ');
    const labelMatch = /scroll.{0,40}(bottom|down)|jump.{0,40}(bottom|latest)|latest message|new messages|more content|continue.{0,40}(below|latest)|down(?:ward)? arrow|arrow.{0,20}down|chevron.{0,20}down|↓|⌄|⌵/i.test(signature);
    const width = viewportWidth();
    const height = viewportHeight();
    const centerX = rect.left + rect.width / 2;
    const centerDelta = width > 0 ? Math.abs(centerX - width / 2) : 0;
    const bottomHalf = height <= 0 || rect.top >= Math.round(height * 0.42);
    const compactButton = rect.width >= 20 && rect.width <= 80 && rect.height >= 20 && rect.height <= 80;
    const centeredFloatingIcon = hasSvg(node)
      && compactButton
      && bottomHalf
      && (width <= 0 || centerDelta <= Math.max(180, Math.round(width * 0.25)));
    if (!labelMatch && !centeredFloatingIcon) return null;
    return {
      labelMatch,
      centeredFloatingIcon,
    };
  };
  const sidebarSignatureFor = node => [
    textOf(node),
    safeAttr(node, 'aria-label'),
    safeAttr(node, 'title'),
    safeAttr(node, 'data-testid'),
    classNameOf(node),
  ].join(' ');
  const textIdentifiesSidebarOrNavigation = (node, signature = sidebarSignatureFor(node)) => {
    if (/history-item|sidebar|side-bar|navigation|nav-|conversation-options|open conversation options/i.test(signature)) return true;
    if (/^(pin|open conversation options|archive|delete|rename)\b/i.test(safeAttr(node, 'aria-label'))) return true;
    if (/^(pin|open conversation options|archive|delete|rename)\b/i.test(safeAttr(node, 'title'))) return true;
    return false;
  };
  const leftSidebarGeometry = (node, rect) => {
    if (!rect) return false;
    const width = viewportWidth();
    if (width > 0 && width < 700) return false;
    const maxRight = width > 0 ? Math.min(360, Math.max(280, Math.round(width * 0.32))) : 360;
    const narrowEnough = rect.width <= 360;
    return narrowEnough && ((rect.left <= 24 && rect.right <= maxRight) || (rect.left < 0 && rect.right <= 96));
  };
  const isSidebarOrNav = (node, main, rect, mainRect) => {
    const tag = safeString(node?.tagName).toLowerCase();
    if (isRootNode(node)) return false;
    if (tag === 'aside' || tag === 'nav') return true;
    if (safeAttr(node, 'role').toLowerCase() === 'navigation') return true;
    if (safeMatches(node, '[data-testid*="sidebar" i],[data-testid*="history" i],[aria-label*="history" i],[aria-label*="navigation" i]')) return true;
    const navAncestor = safeClosest(node, 'aside,nav,[role="navigation"],[data-testid*="sidebar" i],[data-testid*="history" i]');
    if (navAncestor && !(main && safeClosest(node, 'main'))) return true;
    if (textIdentifiesSidebarOrNavigation(node)) return true;
    if (mainRect && rect && rect.right <= mainRect.left + 8) return true;
    if (leftSidebarGeometry(node, rect)) return true;
    return false;
  };
  const moreContentAffordanceRecord = (node, match, rect, index, scope = 'conversation', ignoredReason = '') => ({
    index,
    tag: safeString(node?.tagName).toLowerCase(),
    role: safeAttr(node, 'role'),
    testid: safeAttr(node, 'data-testid').slice(0, 120),
    textPreview: textOf(node).slice(0, 120),
    labelPreview: safeAttr(node, 'aria-label').slice(0, 120),
    titlePreview: safeAttr(node, 'title').slice(0, 120),
    rect: {
      x: Math.round(rect.x || 0),
      y: Math.round(rect.y || 0),
      left: Math.round(rect.left || rect.x || 0),
      top: Math.round(rect.top || rect.y || 0),
      right: Math.round(rect.right || ((rect.x || 0) + (rect.width || 0))),
      bottom: Math.round(rect.bottom || ((rect.y || 0) + (rect.height || 0))),
      width: Math.round(rect.width || 0),
      height: Math.round(rect.height || 0),
    },
    match,
    scope,
    ignoredReason: ignoredReason || undefined,
  });
  const collectMoreContentAffordances = ({ main, mainRect, selectedRect } = {}) => {
    const nodes = typeof document?.querySelectorAll === 'function'
      ? Array.from(document.querySelectorAll('button,[role="button"],a[role="button"],a'))
      : [];
    const relevant = [];
    const ignored = [];
    for (const node of nodes) {
      if (!visibleElement(node)) continue;
      const match = moreContentAffordanceMatch(node);
      if (!match) continue;
      const rect = safeRect(node) || { x: 0, y: 0, left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 };
      let ignoredReason = '';
      if (isSidebarOrNav(node, main, rect, mainRect)) {
        ignoredReason = 'sidebar_or_navigation';
      } else if (selectedRect && Number(selectedRect.left) > 0 && rect.left <= Number(selectedRect.left) + 8 && !match.centeredFloatingIcon) {
        ignoredReason = 'outside_conversation_left_edge';
      }
      if (ignoredReason) {
        if (ignored.length < 8) {
          ignored.push(moreContentAffordanceRecord(node, match, rect, ignored.length, 'sidebar_or_navigation', ignoredReason));
        }
        continue;
      }
      relevant.push(moreContentAffordanceRecord(node, match, rect, relevant.length, 'conversation'));
      if (relevant.length >= 8) break;
    }
    return { relevant, ignored };
  };`;
