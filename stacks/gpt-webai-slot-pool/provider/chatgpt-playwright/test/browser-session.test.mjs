import test from 'node:test';
import assert from 'node:assert/strict';

import {
  cdpEndpoint,
  playwrightArtifactsDir,
  selectExistingPage,
  selectFreshPage,
} from '../lib/browser-session.mjs';

function fakeBrowserWithPage(initialUrl) {
  const calls = [];
  const page = {
    url() {
      return calls.at(-1)?.url || initialUrl;
    },
    async goto(url) {
      calls.push({ method: 'goto', url });
    },
  };
  return {
    calls() {
      return calls;
    },
    contexts() {
      return [{
        pages() {
          return [page];
        },
      }];
    },
  };
}

test('selectFreshPage navigates a stale conversation page back to the root composer', async () => {
  const browser = fakeBrowserWithPage('https://chatgpt.com/c/old-session');

  const page = await selectFreshPage(browser);

  assert.equal(page.url(), 'https://chatgpt.com/');
  assert.deepEqual(browser.calls(), [{ method: 'goto', url: 'https://chatgpt.com/' }]);
});

test('CDP connection uses local browser artifact storage from the lifecycle artifact dir', () => {
  const oldPort = process.env.CDP_PORT;
  const oldArtifacts = process.env.GPT_WEBAI_ARTIFACTS_DIR;
  process.env.CDP_PORT = '9444';
  process.env.GPT_WEBAI_ARTIFACTS_DIR = '/broker-artifacts/run-session';
  try {
    assert.equal(cdpEndpoint(), 'http://127.0.0.1:9444');
    assert.equal(playwrightArtifactsDir(), '/broker-artifacts/run-session/playwright-artifacts');
  } finally {
    if (oldPort === undefined) delete process.env.CDP_PORT;
    else process.env.CDP_PORT = oldPort;
    if (oldArtifacts === undefined) delete process.env.GPT_WEBAI_ARTIFACTS_DIR;
    else process.env.GPT_WEBAI_ARTIFACTS_DIR = oldArtifacts;
  }
});

test('selectExistingPage searches every browser context for the pinned session', async () => {
  const unrelated = { url: () => 'https://chatgpt.com/' };
  const pinned = { url: () => 'https://chatgpt.com/c/session_2' };
  const browser = {
    contexts: () => [
      { pages: () => [unrelated] },
      { pages: () => [pinned] },
    ],
  };
  assert.equal(await selectExistingPage(browser, 'session_2'), pinned);
});
