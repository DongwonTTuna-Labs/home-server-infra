
import test from 'node:test';
import assert from 'node:assert/strict';

import { pageDiagnostics } from '../../lib/diagnostics.mjs';
import { fakeDomPage, fakePage, visibleButton } from './fixtures.mjs';

test('pageDiagnostics hashes control text fields instead of storing raw control text', async () => {
  const diagnostics = await pageDiagnostics(fakePage(), { label: 'unit', sessionId: 'sid-unit' });

  assert.equal(diagnostics.url, 'https://chatgpt.com/c/sid-unit');
  assert.equal(diagnostics.readinessSignals.pro, true);
  assert.equal(diagnostics.controls.length, 1);
  assert.equal(diagnostics.controls[0].textLength, 'private visible control text'.length);
  assert.equal(diagnostics.controls[0].textSha256.length, 64);
  assert.equal(diagnostics.controls[0].labelSha256.length, 64);
  assert.equal(diagnostics.controls[0].titleSha256.length, 64);
  assert.doesNotMatch(JSON.stringify(diagnostics.controls), /private visible control text|private label|private title/);
  assert.equal('textPreview' in diagnostics.assistantTurns[0], false);
});

test('pageDiagnostics counts visible stop controls even beyond sampled control inventory', async () => {
  const controls = Array.from({ length: 90 }, (_, index) => visibleButton({ text: `button ${index}` }));
  controls.push(visibleButton({ 'aria-label': 'Stop answering', 'data-testid': 'stop-button' }));

  const diagnostics = await pageDiagnostics(fakeDomPage(controls), { label: 'unit', sessionId: 'sid-unit' });

  assert.equal(diagnostics.controls.length, 80);
  assert.equal(diagnostics.selectorInventory.controls, 80);
  assert.equal(diagnostics.readinessSignals.stopControls, 1);
});

test('pageDiagnostics does not mark provider limit from assistant/body prose alone', async () => {
  const diagnostics = await pageDiagnostics(fakeDomPage([], {
    bodyText: 'Dongwon Lee\nPro Extended\nThe answer discusses provider limit and too many requests as a bug.',
  }), { label: 'unit', sessionId: 'sid-unit' });

  assert.equal(diagnostics.readinessSignals.limit, false);
  assert.equal(diagnostics.readinessSignals.providerLimitSurfaceCount, 0);
});
