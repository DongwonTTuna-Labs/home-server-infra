import assert from 'node:assert/strict';
import test from 'node:test';

import {
  selectedModelControl,
  verifyRequestedModel,
} from '../lib/model-evidence.mjs';
import { sha256Text } from '../lib/common.mjs';

function fakePage(rawEvidence) {
  return {
    evaluate() {
      return Promise.resolve(rawEvidence);
    },
  };
}

const accountProControl = {
  text: '이동원',
  label: '이동원 Pro',
  title: '',
  testid: 'accounts-profile-button',
  role: 'button',
  rect: { x: 1128, y: 8, left: 1128, top: 8, right: 1270, bottom: 48, width: 142, height: 40 },
  viewportHeight: 720,
  viewportWidth: 1280,
  profileOrAccount: true,
  matchesModelSurface: true,
};

const extraHighComposerControl = {
  text: 'Extra High',
  label: 'Model picker: Extra High',
  title: '',
  testid: 'model-switcher-dropdown-button',
  role: 'button',
  rect: { x: 410, y: 642, left: 410, top: 642, right: 526, bottom: 682, width: 116, height: 40 },
  viewportHeight: 720,
  viewportWidth: 1280,
  inComposerRoot: true,
  nearComposer: true,
  bottomComposerBand: true,
  matchesModelSurface: true,
};

const proExtendedComposerControl = {
  ...extraHighComposerControl,
  text: 'Pro Extended',
  label: 'Model picker: Pro Extended',
};

test('selected model evidence prefers the composer model control over account/profile Pro signals', () => {
  const selected = selectedModelControl([accountProControl, extraHighComposerControl]);

  assert.equal(selected.index, 1);
  assert.equal(selected.control.text, 'Extra High');
});

test('Pro Extended request fails closed when the visible composer model is Extra High', async () => {
  const evidence = await verifyRequestedModel(fakePage({
    bodyText: 'Pro Extended appears in a menu cache and should not override the visible selected model.',
    controls: [accountProControl, extraHighComposerControl],
  }), 'pro', 'extended');

  assert.equal(evidence.ok, false);
  assert.equal(evidence.status, 'model.selection_mismatch');
  assert.equal(evidence.reason, 'model.selection_mismatch');
  assert.equal(evidence.selectedTextSha256, sha256Text('Extra High Model picker: Extra High'));
  assert.equal(evidence.evidence.selectedIndex, 1);
});

test('Pro Extended request passes when the visible composer model is Pro Extended', async () => {
  const evidence = await verifyRequestedModel(fakePage({
    bodyText: 'Upgrade Team workspace text is unrelated.',
    controls: [accountProControl, proExtendedComposerControl],
  }), 'pro', 'extended');

  assert.equal(evidence.ok, true);
  assert.equal(evidence.selectedTextSha256, sha256Text('Pro Extended Model picker: Pro Extended'));
  assert.equal(evidence.evidence.selectedIndex, 1);
});

test('Pro request accepts the current plain Pro composer label with extended effort hint', async () => {
  const evidence = await verifyRequestedModel(fakePage({
    bodyText: '',
    controls: [{ ...extraHighComposerControl, text: 'Pro', label: 'Model picker: Pro' }],
  }), 'pro', 'extended');

  assert.equal(evidence.ok, true);
  assert.equal(evidence.selectedTextSha256, sha256Text('Pro Model picker: Pro'));
});
