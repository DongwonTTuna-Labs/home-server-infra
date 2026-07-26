import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import { downloadArtifacts } from '../lib/artifacts.mjs';
import {
  FakeDownload,
  FakePage,
  PRIVATE_ACCESSIBLE_NAME,
  PRIVATE_CLASS,
  PRIVATE_HREF,
  PRIVATE_VISIBLE_TEXT,
  enoent,
  sha256TextFixture,
  withArtifactRoot,
} from './artifacts-fixtures.mjs';

test('re-clicks the same artifact candidate after saveAs ENOENT and saves the fresh Download', async () => {
  await withArtifactRoot(async () => {
    const stale = new FakeDownload({ failWith: () => enoent() });
    const fresh = new FakeDownload({ payload: 'fresh artifact payload' });
    const page = new FakePage([stale, fresh]);

    const result = await downloadArtifacts(page, 'session-1', { turnIndexes: [0], expectedFilenames: ['fixture-artifact.txt'] });

    assert.equal(page.clicks, 2);
    assert.equal(page.waits, 2);
    assert.equal(stale.saveAsCalls, 1);
    assert.equal(fresh.saveAsCalls, 1);
    assert.equal(result.downloadCandidateCount, 1);
    assert.deepEqual(result.warnings, []);
    assert.equal(result.artifacts.length, 1);
    assert.equal(result.artifactCandidates.length, 1);
    assert.equal(result.artifactCandidates[0], result.artifacts[0]);
    assert.equal(result.artifacts[0].buttonText, PRIVATE_VISIBLE_TEXT);
    assert.equal(result.artifacts[0].buttonTextSha256.length, 64);
    assert.equal(result.artifacts[0].turnScope, 'current-assistant-turn');
    assert.equal(result.artifacts[0].clickedElement.visibleTextSha256.length, 64);
    assert.equal(result.artifacts[0].artifact.status, 'saved');
    assert.equal(result.artifacts[0].artifact.finalFilename, '001-fixture-artifact.txt');
    assert.equal(result.artifacts[0].artifact.hostPath, result.artifacts[0].artifact.savedPath);
    assert.equal(result.artifacts[0].artifact.containerPath, result.artifacts[0].artifact.containerSavedPath);
    assert.equal(result.artifacts[0].download, result.artifacts[0].artifact);
    assert.equal(result.artifacts[0].download.saveAttempts, 2);
    assert.equal(await readFile(result.artifacts[0].download.containerSavedPath, 'utf8'), 'fresh artifact payload');
  });
});

test('default ENOENT recovery window tolerates delayed artifact readiness', async () => {
  await withArtifactRoot(async () => {
    const page = new FakePage([
      new FakeDownload({ failWith: () => enoent() }),
      new FakeDownload({ failWith: () => enoent() }),
      new FakeDownload({ failWith: () => enoent() }),
      new FakeDownload({ payload: 'eventually ready artifact' }),
    ]);

    const result = await downloadArtifacts(page, 'session-default-retry', { turnIndexes: [0], expectedFilenames: ['fixture-artifact.txt'] });

    assert.equal(page.clicks, 4);
    assert.equal(page.waits, 4);
    assert.deepEqual(result.warnings, []);
    assert.equal(result.artifacts.length, 1);
    assert.equal(result.artifacts[0].artifact.status, 'saved');
    assert.equal(result.artifacts[0].artifact.saveAttempts, 4);
    assert.equal(await readFile(result.artifacts[0].artifact.containerPath, 'utf8'), 'eventually ready artifact');
  }, { retryAttempts: null, retryDelay: '1', retryMaxDelay: '1' });
});

test('keeps terminal artifact failure fail-closed when re-clicked downloads still cannot be saved', async () => {
  await withArtifactRoot(async () => {
    const page = new FakePage([
      new FakeDownload({ failWith: () => enoent('download.saveAs: ENOENT for private account label fixture-artifact.txt at https://example.invalid/private/signed?token=secret') }),
      new FakeDownload({ failWith: () => enoent('download.saveAs: ENOENT for private account label fixture-artifact.txt at https://example.invalid/private/signed?token=secret') }),
    ]);

    const result = await downloadArtifacts(page, 'session-2', { turnIndexes: [0], expectedFilenames: ['fixture-artifact.txt'] });

    assert.equal(page.clicks, 2);
    assert.equal(result.downloadCandidateCount, 1);
    assert.deepEqual(result.artifacts, []);
    assert.equal(result.artifactCandidates.length, 1);
    assert.equal(result.artifactCandidates[0].buttonText, PRIVATE_VISIBLE_TEXT);
    assert.equal(result.artifactCandidates[0].buttonTextSha256.length, 64);
    assert.equal(result.artifactCandidates[0].artifact.status, 'failed');
    assert.equal(result.artifactCandidates[0].artifact.reason, 'artifact.download_timeout');
    assert.equal(result.artifactCandidates[0].artifact.phase, 'download.saveAs');
    assert.equal(result.artifactCandidates[0].download, undefined);
    assert.equal(result.warnings.length, 1);
    assert.equal(result.warnings[0].reason, 'artifact.download_timeout');
    assert.equal(result.warnings[0].phase, 'download.saveAs');
    assert.equal(result.warnings[0].saveAttempts, 2);
    assert.match(result.warnings[0].message, /\[redacted-url\]/);
    assert.doesNotMatch(JSON.stringify(result), /token=secret|private account label|private-account-scoped-css-class/);
  });
});

test('does not re-click for non-ENOENT saveAs failures', async () => {
  await withArtifactRoot(async () => {
    const page = new FakePage([new FakeDownload({ failWith: () => new Error('download.saveAs: canceled') })]);

    const result = await downloadArtifacts(page, 'session-3', { turnIndexes: [0], expectedFilenames: ['fixture-artifact.txt'] });

    assert.equal(page.clicks, 1);
    assert.equal(page.waits, 1);
    assert.deepEqual(result.artifacts, []);
    assert.equal(result.artifactCandidates.length, 1);
    assert.equal(result.artifactCandidates[0].artifact.status, 'failed');
    assert.equal(result.warnings.length, 1);
    assert.equal(result.warnings[0].phase, 'download.saveAs');
    assert.equal(result.warnings[0].saveAttempts, 1);
  });
});

test('artifact element evidence contains only hashes and lengths for raw DOM text fields', async () => {
  await withArtifactRoot(async () => {
    const page = new FakePage([new FakeDownload()]);

    const result = await downloadArtifacts(page, 'session-4', { turnIndexes: [0], expectedFilenames: ['fixture-artifact.txt'] });
    const element = result.artifacts[0].element;
    const clickedElement = result.artifacts[0].clickedElement;

    assert.equal(element.visibleText, undefined);
    assert.equal(element.accessibleName, undefined);
    assert.equal(element.hrefDebug, undefined);
    assert.equal(element.class, undefined);
    assert.equal(element.turnText, undefined);
    assert.equal(element.assistantTurnText, undefined);
    assert.equal(element.visibleTextLength, PRIVATE_VISIBLE_TEXT.length);
    assert.equal(element.visibleTextSha256.length, 64);
    assert.equal(element.accessibleNameLength, PRIVATE_ACCESSIBLE_NAME.length);
    assert.equal(element.accessibleNameSha256.length, 64);
    assert.equal(element.classNameLength, PRIVATE_CLASS.length);
    assert.equal(element.classNameSha256.length, 64);
    assert.equal(element.turnTextSha256.length, 64);
    assert.equal(element.assistantTurnTextSha256.length, 64);
    assert.doesNotMatch(JSON.stringify(element), /private account label|token=secret|private-account-scoped-css-class/);
    assert.deepEqual(clickedElement.boundingBox, element.boundingBox);
    assert.equal(clickedElement.turnScope, undefined);
    assert.equal(result.artifacts[0].artifact.visibleFilename, PRIVATE_VISIBLE_TEXT);
    assert.doesNotMatch(JSON.stringify(clickedElement), /private account label|token=secret|private-account-scoped-css-class/);
  });
});

test('links sha256 sidecar artifact objects to the downloaded base artifact', async () => {
  await withArtifactRoot(async () => {
    const payload = 'archive bytes\n';
    const payloadSha256 = sha256TextFixture(payload);
    const page = new FakePage([
      new FakeDownload({ suggestedFilename: 'bundle.tar.gz', payload }),
      new FakeDownload({ suggestedFilename: 'bundle.tar.gz.sha256', payload: `${payloadSha256}  bundle.tar.gz\n` }),
    ], [
      { visibleText: 'bundle.tar.gz', accessibleName: 'bundle.tar.gz' },
      { visibleText: 'bundle.tar.gz.sha256', accessibleName: 'bundle.tar.gz.sha256' },
    ]);

    const result = await downloadArtifacts(page, 'session-5', { turnIndexes: [0], expectedFilenames: ['bundle.tar.gz', 'bundle.tar.gz.sha256'] });

    assert.equal(result.artifacts.length, 2);
    assert.equal(result.artifactCandidates.length, 2);
    assert.deepEqual(result.warnings, []);
    const base = result.artifacts.find(item => item.buttonText === 'bundle.tar.gz');
    const sidecar = result.artifacts.find(item => item.buttonText === 'bundle.tar.gz.sha256');
    assert.ok(base);
    assert.ok(sidecar);
    assert.equal(base.artifact.integrity.sha256Sidecar, 'verified');
    assert.equal(sidecar.artifact.sidecarOf.buttonText, 'bundle.tar.gz');
    assert.equal(sidecar.artifact.sidecarOf.buttonTextSha256, base.buttonTextSha256);
    assert.equal(sidecar.artifact.sidecarOf.sha256, base.artifact.sha256);
    assert.equal(sidecar.artifact.integrity.status, 'verified');
    assert.equal(sidecar.artifact.integrity.declaredSha256, base.artifact.sha256);
  });
});

test('downloads generic current-turn file controls while preserving visible button text identity', async () => {
  await withArtifactRoot(async () => {
    const payload = 'archive bytes\n';
    const payloadSha256 = sha256TextFixture(payload);
    const page = new FakePage([
      new FakeDownload({ suggestedFilename: 'bundle.zip', payload }),
      new FakeDownload({ suggestedFilename: 'bundle.zip.sha256', payload: `${payloadSha256}  bundle.zip\n` }),
    ], [
      { visibleText: 'Download ZIP', accessibleName: 'Download ZIP' },
      { visibleText: 'Download SHA256 sidecar', accessibleName: 'Download SHA256 sidecar' },
    ]);

    const result = await downloadArtifacts(page, 'session-generic-controls', { turnIndexes: [0] });

    assert.equal(result.downloadCandidateCount, 2);
    assert.equal(result.artifacts.length, 2);
    assert.deepEqual(result.warnings, []);
    const base = result.artifacts.find(item => item.buttonText === 'Download ZIP');
    const sidecar = result.artifacts.find(item => item.buttonText === 'Download SHA256 sidecar');
    assert.ok(base);
    assert.ok(sidecar);
    assert.equal(base.artifact.visibleFilename, 'artifact-1.zip');
    assert.equal(base.artifact.suggestedFilename, 'bundle.zip');
    assert.equal(base.artifact.finalFilename, '001-artifact-1.zip');
    assert.equal(sidecar.artifact.visibleFilename, 'artifact-2.zip.sha256');
    assert.equal(sidecar.artifact.suggestedFilename, 'bundle.zip.sha256');
    assert.equal(base.artifact.integrity.sha256Sidecar, 'verified');
    assert.equal(sidecar.artifact.sidecarOf.buttonText, 'Download ZIP');
  });
});
