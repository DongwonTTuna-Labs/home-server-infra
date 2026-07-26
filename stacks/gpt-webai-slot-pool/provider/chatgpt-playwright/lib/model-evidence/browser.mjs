export async function collectModelSurfaceEvidence(page) {
  return await page.evaluate(() => {
    const visible = node => {
      if (!node || typeof node.getBoundingClientRect !== 'function') return false;
      const rect = node.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return false;
      const style = window.getComputedStyle?.(node);
      return !style || (style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0');
    };
    const attr = (node, name) => (typeof node?.getAttribute === 'function' ? node.getAttribute(name) || '' : '');
    const textOf = node => (node?.innerText || node?.textContent || '').trim();
    const rectOf = node => {
      const rect = node.getBoundingClientRect();
      return {
        x: Math.round(rect.x),
        y: Math.round(rect.y),
        left: Math.round(rect.left ?? rect.x),
        top: Math.round(rect.top ?? rect.y),
        right: Math.round(rect.right ?? (rect.x + rect.width)),
        bottom: Math.round(rect.bottom ?? (rect.y + rect.height)),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
      };
    };
    const closest = (node, selector) => (node && typeof node.closest === 'function' ? node.closest(selector) : null);
    const matches = (node, selector) => Boolean(node && typeof node.matches === 'function' && node.matches(selector));
    const bodyText = document.body?.innerText || '';
    const viewportHeight = Number(window?.innerHeight || document?.documentElement?.clientHeight || 0);
    const viewportWidth = Number(window?.innerWidth || document?.documentElement?.clientWidth || 0);
    const main = document.querySelector?.('main') || null;
    const mainRect = main && typeof main.getBoundingClientRect === 'function' ? rectOf(main) : null;
    const textbox = Array.from(document.querySelectorAll('#prompt-textarea, textarea, [contenteditable="true"][role="textbox"], .ProseMirror[contenteditable="true"], [contenteditable="true"]'))
      .filter(visible)
      .sort((left, right) => rectOf(right).bottom - rectOf(left).bottom)[0] || null;
    const composerRoot = textbox
      ? (closest(textbox, 'form,[data-testid*="composer" i],[class*="composer" i],[data-testid*="prompt" i]') || textbox.parentElement || textbox)
      : null;
    const composerRect = composerRoot && typeof composerRoot.getBoundingClientRect === 'function'
      ? rectOf(composerRoot)
      : (textbox ? rectOf(textbox) : null);
    const inSidebarOrNav = node => {
      const signal = `${attr(node, 'data-testid')} ${attr(node, 'aria-label')} ${attr(node, 'title')}`;
      if (/sidebar|history|nav(?:igation)?/i.test(signal)) return true;
      if (closest(node, 'aside,nav,[role="navigation"],[data-testid*="sidebar" i],[data-testid*="history" i]')) return true;
      const rect = rectOf(node);
      return Boolean(mainRect && rect.right <= mainRect.left + 8);
    };
    const inComposerRoot = node => Boolean(composerRoot && (node === composerRoot || composerRoot.contains?.(node)));
    const nearComposer = node => {
      if (!composerRect) return false;
      const rect = rectOf(node);
      const vertical = rect.bottom >= composerRect.top - 96 && rect.top <= composerRect.bottom + 96;
      const horizontal = rect.right >= composerRect.left - 64 && rect.left <= composerRect.right + 64;
      return vertical && horizontal;
    };
    const bottomComposerBand = node => {
      if (!viewportHeight) return false;
      const rect = rectOf(node);
      return rect.top >= Math.round(viewportHeight * 0.58);
    };
    const controlNodes = Array.from(document.querySelectorAll('button,[role="button"],[aria-haspopup="menu"],[data-testid*="model" i]'));
    // The current ChatGPT composer renders the model switcher as a clickable
    // div/span without a role, aria-label, title, or test id.  It is still a
    // real model control (the visible text is the selected model), so include
    // exact model-label descendants in the composer area.  Do not use body
    // text as evidence: only a visible, near-composer exact label qualifies.
    const exactModelLabel = /^(?:Pro(?:\s+Extended)?|Instant|Extra\s+High|High|Auto|Thinking|Standard|Fast)$/i;
    const fallbackNodes = Array.from(document.querySelectorAll('div,span,p'))
      .filter(node => {
        if (!visible(node) || !nearComposer(node)) return false;
        const text = textOf(node).replace(/\s+/g, ' ').trim();
        if (!exactModelLabel.test(text)) return false;
        // Some builds wrap the label in a span alongside an SVG chevron, so
        // the exact text is not a direct child of the clickable wrapper.
        return true;
      });
    for (const node of fallbackNodes) {
      const clickable = closest(node, 'button,[role="button"],[aria-haspopup="menu"]') || node;
      if (!controlNodes.includes(clickable)) controlNodes.push(clickable);
    }
    const controls = controlNodes
      .filter(visible)
      .map(node => {
        const text = textOf(node);
        const label = attr(node, 'aria-label');
        const title = attr(node, 'title');
        const testid = attr(node, 'data-testid');
        const role = attr(node, 'role');
        const signal = `${text} ${label} ${title} ${testid} ${role}`;
        return {
          text,
          label,
          title,
          testid,
          role,
          rect: rectOf(node),
          viewportHeight,
          viewportWidth,
          inComposerRoot: inComposerRoot(node),
          nearComposer: nearComposer(node),
          bottomComposerBand: bottomComposerBand(node),
          sidebarOrNav: inSidebarOrNav(node),
          profileOrAccount: /account|profile|avatar|workspace|user-menu|accounts-profile/i.test(signal)
            || matches(node, '[data-testid*="account" i],[data-testid*="profile" i]'),
          matchesModelSurface: /\b(?:GPT(?:[-\s]?[45])?|Pro(?:\s+Extended)?|Model|Thinking|Reason(?:ing)?|Extra\s+High|High|Auto|Standard|Fast)\b|모델|프로/i.test(signal),
        };
      })
      .filter(item => `${item.text}\n${item.label}\n${item.title}\n${item.testid}`.trim());
    return { bodyText: bodyText.slice(0, 4000), controls };
  }).catch(error => ({ bodyText: '', controls: [], error: error instanceof Error ? error.message : String(error) }));
}
