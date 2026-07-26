
import test from 'node:test';
import assert from 'node:assert/strict';

import { pageDiagnostics } from '../../lib/diagnostics.mjs';
import { fakeDomPage, visibleButton, visibleSurface } from './fixtures.mjs';

test('pageDiagnostics records scoped provider-limit blocking surface evidence', async () => {
  const gotIt = visibleButton({ text: 'Got it' });
  const surface = visibleSurface({
    role: 'dialog',
    className: 'provider-limit-modal secret-class',
    text: 'Too many requests. Please try again later.',
  }, { buttons: [gotIt] });

  const diagnostics = await pageDiagnostics(fakeDomPage([], {
    providerLimitSurfaces: [surface],
  }), { label: 'unit', sessionId: 'sid-unit' });

  assert.equal(diagnostics.readinessSignals.limit, true);
  assert.equal(diagnostics.readinessSignals.providerLimitSurfaceCount, 1);
  assert.equal(diagnostics.providerLimitSurfaces.length, 1);
  assert.equal(diagnostics.providerLimitSurfaces[0].textSha256.length, 64);
  assert.equal(diagnostics.providerLimitSurfaces[0].classNameSha256.length, 64);
  assert.equal(diagnostics.providerLimitSurfaces[0].actionButtons[0].textSha256.length, 64);
  assert.doesNotMatch(JSON.stringify(diagnostics.providerLimitSurfaces), /Got it|secret-class/);
});

test('pageDiagnostics ignores provider-limit phrases in broad conversation live regions', async () => {
  const broadLive = visibleSurface({
    'aria-live': 'polite',
    text: 'Too many requests appears in a broad conversation live region.',
    rect: { x: 0, y: 0, width: 1200, height: 600 },
  }, { closestSelectors: ['main'] });

  const diagnostics = await pageDiagnostics(fakeDomPage([], {
    providerLimitSurfaces: [broadLive],
  }), { label: 'unit', sessionId: 'sid-unit' });

  assert.equal(diagnostics.readinessSignals.limit, false);
  assert.equal(diagnostics.readinessSignals.providerLimitSurfaceCount, 0);
  assert.deepEqual(diagnostics.providerLimitSurfaces, []);
});

test('pageDiagnostics ignores provider-limit phrases in sidebar composer and attachment contexts', async () => {
  const sidebar = visibleSurface({ role: 'alert', text: 'rate limit history item' }, { closestSelectors: ['aside'] });
  const composer = visibleSurface({ role: 'alert', text: 'request limit typed in composer' }, { closestSelectors: ['[class*="composer" i]'] });
  const attachment = visibleSurface({ role: 'alert', text: 'too many requests.txt' }, { closestSelectors: ['[data-testid*="attachment" i]'] });

  const diagnostics = await pageDiagnostics(fakeDomPage([], {
    providerLimitSurfaces: [sidebar, composer, attachment],
  }), { label: 'unit', sessionId: 'sid-unit' });

  assert.equal(diagnostics.readinessSignals.limit, false);
  assert.equal(diagnostics.readinessSignals.providerLimitSurfaceCount, 0);
});
