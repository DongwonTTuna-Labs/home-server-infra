import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import { downloadArtifacts } from '../lib/artifacts.mjs';
import {
  FakeDownload,
  FakePage,
  withArtifactRoot,
} from './artifacts-fixtures.mjs';

const MANIFEST_NAME = 'pr72-corrected-source-tree-v3-20260709T0610Z.manifest';

test('discovers and downloads visible .manifest artifact buttons', async () => {
  await withArtifactRoot(async () => {
    const payload = 'sha256  size  path\nabc123  42  src/lib/common.mjs\n';
    const page = new FakePage([
      new FakeDownload({ suggestedFilename: MANIFEST_NAME, payload }),
    ], [
      { visibleText: MANIFEST_NAME, accessibleName: MANIFEST_NAME },
    ]);

    const result = await downloadArtifacts(page, 'session-manifest-artifact', { turnIndexes: [0] });

    assert.equal(result.downloadCandidateCount, 1);
    assert.equal(result.artifacts.length, 1);
    assert.equal(result.artifactCandidates.length, 1);
    assert.deepEqual(result.warnings, []);

    assert.equal(page.clicks, 1);
    assert.equal(page.waits, 1);

    const [object] = result.artifacts;
    assert.equal(result.artifactCandidates[0], object);
    assert.equal(object.buttonText, MANIFEST_NAME);
    assert.equal(object.artifact.status, 'saved');
    assert.equal(object.artifact.visibleFilename, MANIFEST_NAME);
    assert.equal(object.artifact.suggestedFilename, MANIFEST_NAME);
    assert.equal(object.artifact.finalFilename, `001-${MANIFEST_NAME}`);
    assert.equal(object.artifact.fileType, 'manifest');
    assert.equal(object.artifact.type, 'manifest');
    assert.equal(object.artifact.mime, 'text/plain');
    assert.equal(await readFile(object.artifact.containerPath, 'utf8'), payload);
  });
});

test('discovers a file card exposed as a role=button control', async () => {
  await withArtifactRoot(async () => {
    const page = new FakePage([
      new FakeDownload({ suggestedFilename: 'pr72-design-delta.txt', payload: 'role button payload' }),
    ], [
      { tag: 'div', role: 'button', visibleText: 'pr72-design-delta.txt', accessibleName: 'pr72-design-delta.txt' },
    ]);

    const result = await downloadArtifacts(page, 'session-role-button-artifact', { turnIndexes: [0] });

    assert.equal(result.downloadCandidateCount, 1);
    assert.equal(result.artifacts.length, 1);
    assert.equal(result.artifacts[0].artifact.visibleFilename, 'pr72-design-delta.txt');
    assert.equal(await readFile(result.artifacts[0].artifact.containerPath, 'utf8'), 'role button payload');
    assert.equal(page.clicks, 1);
  });
});

test('opens a file card and downloads from the visible preview Download button', async () => {
  await withArtifactRoot(async () => {
    const filename = 'pr72-preview-card.txt';
    const page = new FakePage([
      new FakeDownload({ suggestedFilename: filename, payload: 'preview payload' }),
    ], [
      { tag: 'button', role: 'button', visibleText: filename, accessibleName: filename, domBacked: true, fileCardOpener: true },
    ], {
      previewCandidates: [
        { tag: 'button', role: 'button', visibleText: 'Download', accessibleName: 'Download', domBacked: true },
      ],
    });

    const result = await downloadArtifacts(page, 'session-preview-card-artifact', {
      turnIndexes: [0],
      expectedFilenames: [filename],
    });

    assert.equal(result.downloadCandidateCount, 2);
    assert.equal(result.artifacts.length, 1);
    assert.deepEqual(result.warnings, []);
    assert.equal(result.artifacts[0].buttonText, 'Download');
    assert.equal(result.artifacts[0].artifact.visibleFilename, filename);
    assert.equal(await readFile(result.artifacts[0].artifact.containerPath, 'utf8'), 'preview payload');
    assert.equal(page.clicks, 2);
  });
});
