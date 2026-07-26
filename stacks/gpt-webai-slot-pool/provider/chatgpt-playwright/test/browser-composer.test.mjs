import test from 'node:test';
import assert from 'node:assert/strict';

import { clickSend } from '../lib/browser-composer.mjs';

function fakeSendPage({ enabledAfter }) {
  let checks = 0;
  let clicks = 0;
  return {
    stats() {
      return { checks, clicks };
    },
    locator() {
      return {
        first() {
          return this;
        },
        async count() {
          return 1;
        },
        async isVisible() {
          return true;
        },
        async isEnabled() {
          checks += 1;
          return checks >= enabledAfter;
        },
        async click() {
          clicks += 1;
        },
      };
    },
    async waitForTimeout() {},
  };
}

test('clickSend waits for the visible send button to become enabled', async () => {
  const page = fakeSendPage({ enabledAfter: 3 });

  await clickSend(page);

  assert.deepEqual(page.stats(), { checks: 3, clicks: 1 });
});
