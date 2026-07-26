import assert from 'node:assert/strict';
import test from 'node:test';

import { conversationHydrated } from '../lib/turns.mjs';

test('conversation hydration requires a visible turn or active generation', () => {
  assert.equal(conversationHydrated({ assistantCount: 0, userCount: 0, activeTurn: false }), false);
  assert.equal(conversationHydrated({ assistantCount: 1, userCount: 0, activeTurn: false }), true);
  assert.equal(conversationHydrated({ assistantCount: 0, userCount: 1, activeTurn: false }), true);
  assert.equal(conversationHydrated({ assistantCount: 0, userCount: 0, activeTurn: true }), true);
});
