
import { BOTTOM_SCROLL_AFFORDANCE_SOURCE } from './page-affordance-source.mjs';
import { BOTTOM_SCROLL_CANDIDATE_SOURCE } from './page-candidate-source.mjs';
import { BOTTOM_SCROLL_DOM_SOURCE } from './page-dom-source.mjs';
import { BOTTOM_SCROLL_RUN_SOURCE } from './page-run-source.mjs';

export function makeBottomScrollPageFunction() {
  const source = [
    'return function scrollPrimaryConversationViewportInPage() {',
    BOTTOM_SCROLL_DOM_SOURCE,
    BOTTOM_SCROLL_AFFORDANCE_SOURCE,
    BOTTOM_SCROLL_CANDIDATE_SOURCE,
    BOTTOM_SCROLL_RUN_SOURCE,
    '}',
  ].join('\n');
  return Function(source)();
}
