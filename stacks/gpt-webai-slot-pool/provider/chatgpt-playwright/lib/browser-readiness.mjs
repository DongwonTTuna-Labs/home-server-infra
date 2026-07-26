import { classifyReadinessSignals } from './readiness.mjs';

export async function classifyReadiness(page) {
  const url = page.url();
  const signals = await page.evaluate(currentUrl => {
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
    const rectOf = node => {
      const rect = node.getBoundingClientRect();
      return {
        width: Math.round(rect.width),
        height: Math.round(rect.height),
      };
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
    const isNarrowToastOrLiveSurface = node => {
      const kind = surfaceKind(node);
      if (!['toast', 'live'].includes(kind)) return false;
      if (hasClosest(node, ignoredLimitContextSelectors)) return false;
      const rect = rectOf(node);
      const viewportWidth = Number(window.innerWidth || 0);
      const broadWidth = viewportWidth > 0 ? Math.min(viewportWidth * 0.55, 720) : 720;
      return rect.width > 0 && rect.height > 0 && rect.width <= broadWidth && rect.height <= 260;
    };
    const isBlockingProviderSurface = node => {
      if (hasClosest(node, ignoredLimitContextSelectors)) return false;
      const kind = surfaceKind(node);
      if (['dialog', 'alert'].includes(kind)) return true;
      return isNarrowToastOrLiveSurface(node);
    };
    const text = document.body?.innerText || '';
    const controls = Array.from(document.querySelectorAll('button,a,[role="button"]'))
      .filter(visible)
      .map(node => (textOf(node) || attr(node, 'aria-label')).trim())
      .filter(Boolean)
      .join('\n');
    const providerLimitSurfaces = Array.from(document.querySelectorAll(providerSurfaceSelector))
      .filter(visible)
      .filter(isBlockingProviderSurface)
      .map(textOf)
      .filter(value => providerLimitPattern.test(value))
      .join('\n');
    const login = /Thanks for trying ChatGPT|Log in or sign up|Log in to get answers|Sign up for free|get smarter responses, upload files|^Log in$/im.test(`${text}\n${controls}`);
    const challenge = /CAPTCHA|2FA|two-factor|verify you are human|verification code/i.test(text);
    const limit = providerLimitPattern.test(providerLimitSurfaces);
    const upgrade = /\bUpgrade\b|Get Plus|Go Plus|Free plan/i.test(text);
    const pro = /\bPro Extended\b|\bPro\b|프로/i.test(`${text}\n${controls}`);
    const composer = Array.from(document.querySelectorAll('#prompt-textarea, textarea[placeholder*="Message" i], [contenteditable="true"][role="textbox"], .ProseMirror[contenteditable="true"], [contenteditable="true"]')).some(visible);
    const send = Array.from(document.querySelectorAll('button[data-testid*="send" i], button[aria-label*="send" i], button')).some(node => {
      if (!visible(node) || node.disabled || attr(node, 'aria-disabled') === 'true') return false;
      const label = `${attr(node, 'aria-label')} ${node.innerText || node.textContent || ''}`;
      return /send|submit|전송/i.test(label);
    });
    return { url: currentUrl, composer, send, login, challenge, providerLimit: limit, upgrade, pro };
  }, url).catch(error => ({
    url,
    status: 'unknown',
    reason: 'provider.schema_drift',
    message: error instanceof Error ? error.message : String(error),
  }));
  if (signals.status) return signals;
  return classifyReadinessSignals(signals);
}
