import { sanitizeFilename, sha256Text } from './common.mjs';

export async function candidateSnapshot(locator, index) {
  return await locator.evaluate((node, candidateIndex) => {
    const rect = node.getBoundingClientRect();
    const text = (node.innerText || '').trim();
    const name = node.getAttribute('aria-label') || node.getAttribute('download') || text;
    const nearestContext = node.closest('[data-message-author-role="assistant"], article, .markdown, [class*="markdown"]');
    const assistantTurn = node.closest('[data-message-author-role="assistant"], article');
    const turnText = (nearestContext?.innerText || nearestContext?.textContent || '').trim();
    const assistantTurnText = (assistantTurn?.innerText || assistantTurn?.textContent || '').trim();
    const fileCard = node.closest('[tabindex="0"]');
    const fileCardOpener = Boolean(
      fileCard
      && fileCard !== node
      && /\.(?:tar\.gz|zip|tgz|gz|diff|patch|txt|md|manifest|sha256|json|csv)\b/i.test((fileCard.innerText || fileCard.textContent || '').trim()),
    );
    return {
      candidateIndex,
      role: node.getAttribute('role') || (node instanceof HTMLAnchorElement ? 'link' : node instanceof HTMLButtonElement ? 'button' : ''),
      tag: node.tagName.toLowerCase(),
      class: node.getAttribute('class') || '',
      visibleText: text,
      accessibleName: name,
      hrefDebug: node.getAttribute('href') || null,
      boundingBox: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
      turnTextSha256: turnText ? '' : '',
      assistantTurnTextSha256: assistantTurnText ? '' : '',
      turnText,
      assistantTurnText,
      fileCardOpener,
    };
  }, index);
}

export async function assistantTurnLocator(page) {
  const primary = page.locator('[data-message-author-role="assistant"]');
  if (await primary.count().catch(() => 0)) return primary;
  return page.locator('main article');
}

export function normalizeExpectedFilenames(values) {
  if (!Array.isArray(values)) return new Set();
  return new Set(values.map(value => sanitizeFilename(value, '')).filter(Boolean).map(value => value.toLowerCase()));
}

export function filenameMatchesExpected(filename, expectedFilenames) {
  if (!expectedFilenames || expectedFilenames.size === 0) return false;
  const safe = sanitizeFilename(filename, '').toLowerCase();
  return Boolean(safe && expectedFilenames.has(safe));
}

export function candidateDedupeKey(snapshot) {
  if (!snapshot) return '';
  const box = snapshot.boundingBox || {};
  return [
    snapshot.role || '',
    snapshot.tag || '',
    snapshot.class || '',
    snapshot.visibleText || '',
    snapshot.accessibleName || '',
    snapshot.hrefDebug || '',
    Math.round(Number(box.x || 0)),
    Math.round(Number(box.y || 0)),
    Math.round(Number(box.width || 0)),
    Math.round(Number(box.height || 0)),
  ].join('\u001f');
}

function candidateNearScopedTurn(snapshot, scopedTurnBounds) {
  if (!Array.isArray(scopedTurnBounds) || scopedTurnBounds.length === 0) return true;
  const box = snapshot?.boundingBox;
  if (!box) return false;
  return scopedTurnBounds.some(turnBox => (
    Number(box.y) + Number(box.height || 0) >= Number(turnBox.y) - 80
    && Number(box.y) <= Number(turnBox.y) + Number(turnBox.height || 0) + 320
  ));
}

export function candidateInCurrentTurnScope(snapshot, scopedTurnTextHashes, scopedTurnBounds) {
  const assistantTurnText = snapshot?.assistantTurnText || '';
  if (assistantTurnText) return scopedTurnTextHashes.has(sha256Text(assistantTurnText));

  const turnText = snapshot?.turnText || '';
  if (turnText && scopedTurnTextHashes.has(sha256Text(turnText))) return true;

  if (scopedTurnTextHashes.size === 0 && (!Array.isArray(scopedTurnBounds) || scopedTurnBounds.length === 0)) return false;
  return candidateNearScopedTurn(snapshot, scopedTurnBounds);
}
