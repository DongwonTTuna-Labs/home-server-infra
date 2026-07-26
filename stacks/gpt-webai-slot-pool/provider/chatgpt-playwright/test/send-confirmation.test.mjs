import test from 'node:test';
import assert from 'node:assert/strict';

import { waitForSendStartConfirmation } from '../lib/send-confirmation.mjs';

const ROOT_URL = 'https://chatgpt.com/';
const SESSION_URL = 'https://chatgpt.com/c/6a4dfake-session';

function fakeStartPage({ conversationAfterWaits }) {
  let waits = 0;
  let evaluateCalls = 0;

  return {
    stats() {
      return { waits };
    },
    url() {
      return waits >= conversationAfterWaits ? SESSION_URL : ROOT_URL;
    },
    async evaluate() {
      const phase = evaluateCalls % 3;
      evaluateCalls += 1;
      if (phase === 0) return [{ turnIndex: 0, domId: 'user-1', text: 'prompt' }];
      if (phase === 1) return [];
      return true;
    },
    async waitForTimeout() {
      waits += 1;
    },
  };
}

test('waitForSendStartConfirmation keeps waiting when generation starts before the conversation URL appears', async () => {
  const page = fakeStartPage({ conversationAfterWaits: 2 });

  const confirmation = await waitForSendStartConfirmation(page, { userCount: 0, assistantCount: 0 }, 60_000);

  assert.equal(confirmation.conversationUrl, SESSION_URL);
  assert.equal(confirmation.turnEvidence.activeTurn, true);
  assert.equal(confirmation.turnEvidence.newUserTurn, true);
  assert.deepEqual(page.stats(), { waits: 2 });
});

test('waitForSendStartConfirmation does not manufacture a session URL from root turn evidence', async () => {
  const page = fakeStartPage({ conversationAfterWaits: Infinity });

  const confirmation = await waitForSendStartConfirmation(page, { userCount: 0, assistantCount: 0 }, 0);

  assert.equal(confirmation.conversationUrl, ROOT_URL);
  assert.equal(confirmation.turnEvidence.activeTurn, true);
  assert.equal(confirmation.turnEvidence.newUserTurn, true);
});
