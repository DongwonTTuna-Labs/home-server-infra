import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

import { downloadArtifacts } from '../lib/artifacts.mjs';
import {
  FakeDownload,
  FakePage,
  PRIVATE_ACCESSIBLE_NAME,
  PRIVATE_HREF,
  PRIVATE_VISIBLE_TEXT,
  enoent,
  withArtifactRoot,
} from './artifacts-fixtures.mjs';

test('current-turn artifact scope clicks the selected assistant turn when filenames collide', async () => {
  await withArtifactRoot(async () => {
    const page = new FakePage([
      new FakeDownload({ suggestedFilename: 'bundle.zip', payload: 'current turn archive' }),
    ], [
      { turnIndex: 0, visibleText: 'bundle.zip', accessibleName: 'bundle.zip' },
      { turnIndex: 1, visibleText: 'bundle.zip', accessibleName: 'bundle.zip' },
    ], {
      turnCount: 2,
      turnTexts: ['previous assistant turn with bundle.zip', 'current assistant turn with bundle.zip'],
    });

    const result = await downloadArtifacts(page, 'session-current-turn', { turnIndexes: [1], expectedFilenames: ['bundle.zip'] });

    assert.equal(page.clicks, 1);
    assert.equal(page.clickedCandidates.length, 1);
    assert.equal(page.clickedCandidates[0].turnIndex, 1);
    assert.equal(result.artifacts.length, 1);
    assert.equal(result.artifacts[0].turnScope, 'current-assistant-turn');
    assert.equal(result.artifacts[0].artifact.status, 'saved');
    assert.equal(await readFile(result.artifacts[0].artifact.containerPath, 'utf8'), 'current turn archive');
  });
});


test('artifact discovery scrolls the primary conversation bottom before collecting controls', async () => {
  await withArtifactRoot(async () => {
    const page = new FakePage([new FakeDownload()], [
      { turnIndex: 0, visibleText: PRIVATE_VISIBLE_TEXT, accessibleName: PRIVATE_VISIBLE_TEXT },
    ]);

    const result = await downloadArtifacts(page, 'session-scroll-before-artifact-discovery', { turnIndexes: [0], expectedFilenames: [PRIVATE_VISIBLE_TEXT] });

    assert.equal(page.bottomScrolls, 1);
    assert.equal(page.events[0], 'bottom-scroll');
    assert.match(page.events.find(event => event.startsWith('locator:')) || '', /assistant|button/);
    assert.equal(result.bottomScroll.status, 'at_bottom');
    assert.equal(result.artifacts.length, 1);
  });
});

test('filename fallback does not download a matching artifact from a prior assistant turn', async () => {
  await withArtifactRoot(async () => {
    const page = new FakePage([
      new FakeDownload({ suggestedFilename: 'bundle.zip', payload: 'previous turn archive' }),
    ], [
      { turnIndex: 0, visibleText: 'bundle.zip', accessibleName: 'bundle.zip' },
    ], {
      turnCount: 2,
      turnTexts: ['previous assistant turn with bundle.zip', 'current assistant turn without artifact controls'],
    });

    const result = await downloadArtifacts(page, 'session-no-prior-fallback', { turnIndexes: [1], expectedFilenames: ['bundle.zip'] });

    assert.equal(page.clicks, 0);
    assert.equal(page.waits, 0);
    assert.equal(result.downloadCandidateCount, 0);
    assert.deepEqual(result.artifacts, []);
    assert.deepEqual(result.artifactCandidates, []);
    assert.deepEqual(result.warnings, []);
  });
});

test('does not promote accessible-name-only artifact labels into durable buttonText', async () => {
  await withArtifactRoot(async () => {
    const page = new FakePage([new FakeDownload()], [{
      visibleText: '',
      accessibleName: PRIVATE_ACCESSIBLE_NAME,
      hrefDebug: '',
    }]);

    const result = await downloadArtifacts(page, 'session-6', { turnIndexes: [0], expectedFilenames: ['fixture-artifact.txt'] });

    assert.equal(page.clicks, 0);
    assert.equal(page.waits, 0);
    assert.equal(result.downloadCandidateCount, 0);
    assert.deepEqual(result.artifacts, []);
    assert.deepEqual(result.artifactCandidates, []);
    assert.deepEqual(result.warnings, []);
    assert.doesNotMatch(JSON.stringify(result), /private account label/);
  });
});

test('does not promote href-only artifact labels into durable buttonText', async () => {
  await withArtifactRoot(async () => {
    const page = new FakePage([new FakeDownload()], [{
      visibleText: '',
      accessibleName: '',
      hrefDebug: PRIVATE_HREF,
    }]);

    const result = await downloadArtifacts(page, 'session-7', { turnIndexes: [0], expectedFilenames: ['fixture-artifact.txt'] });

    assert.equal(page.clicks, 0);
    assert.equal(page.waits, 0);
    assert.equal(result.downloadCandidateCount, 0);
    assert.deepEqual(result.artifacts, []);
    assert.deepEqual(result.artifactCandidates, []);
    assert.deepEqual(result.warnings, []);
    assert.doesNotMatch(JSON.stringify(result), /token=secret|fixture-artifact.txt/);
  });
});

test('persists exact visible button text while parsing filename separately', async () => {
  await withArtifactRoot(async () => {
    const page = new FakePage([new FakeDownload()], [{
      visibleText: `Download ${PRIVATE_VISIBLE_TEXT}`,
      accessibleName: PRIVATE_ACCESSIBLE_NAME,
      hrefDebug: PRIVATE_HREF,
    }]);

    const result = await downloadArtifacts(page, 'session-8', { turnIndexes: [0], expectedFilenames: ['fixture-artifact.txt'] });

    assert.equal(page.clicks, 1);
    assert.equal(result.artifacts.length, 1);
    assert.equal(result.artifacts[0].buttonText, `Download ${PRIVATE_VISIBLE_TEXT}`);
    assert.equal(result.artifacts[0].artifact.visibleFilename, PRIVATE_VISIBLE_TEXT);
    assert.doesNotMatch(JSON.stringify(result), /private account label|token=secret/);
  });
});

test('does not promote textContent-only hidden artifact labels into durable buttonText', async () => {
  await withArtifactRoot(async () => {
    const page = new FakePage([new FakeDownload()], [{
      domBacked: true,
      innerText: '',
      textContent: PRIVATE_ACCESSIBLE_NAME,
      accessibleName: '',
      hrefDebug: '',
    }]);

    const result = await downloadArtifacts(page, 'session-9', { turnIndexes: [0], expectedFilenames: ['fixture-artifact.txt'] });

    assert.equal(page.clicks, 0);
    assert.equal(page.waits, 0);
    assert.equal(result.downloadCandidateCount, 0);
    assert.deepEqual(result.artifacts, []);
    assert.deepEqual(result.artifactCandidates, []);
    assert.deepEqual(result.warnings, []);
    assert.doesNotMatch(JSON.stringify(result), /private account label|fixture-artifact.txt/);
  });
});

test('does not create failed artifact candidates from textContent-only hidden labels', async () => {
  await withArtifactRoot(async () => {
    const page = new FakePage([new FakeDownload({ failWith: () => enoent() })], [{
      domBacked: true,
      innerText: '',
      textContent: PRIVATE_ACCESSIBLE_NAME,
      accessibleName: '',
      hrefDebug: '',
    }]);

    const result = await downloadArtifacts(page, 'session-10', { turnIndexes: [0], expectedFilenames: ['fixture-artifact.txt'] });

    assert.equal(page.clicks, 0);
    assert.equal(page.waits, 0);
    assert.equal(result.downloadCandidateCount, 0);
    assert.deepEqual(result.artifacts, []);
    assert.deepEqual(result.artifactCandidates, []);
    assert.deepEqual(result.warnings, []);
    assert.doesNotMatch(JSON.stringify(result), /private account label|fixture-artifact.txt/);
  });
});

test('visible innerText remains the only durable identity when textContent has hidden private text', async () => {
  await withArtifactRoot(async () => {
    const page = new FakePage([new FakeDownload()], [{
      domBacked: true,
      innerText: PRIVATE_VISIBLE_TEXT,
      textContent: `private account label ${PRIVATE_VISIBLE_TEXT}`,
      accessibleName: PRIVATE_ACCESSIBLE_NAME,
      hrefDebug: PRIVATE_HREF,
    }]);

    const result = await downloadArtifacts(page, 'session-11', { turnIndexes: [0], expectedFilenames: ['fixture-artifact.txt'] });

    assert.equal(page.clicks, 1);
    assert.equal(result.artifacts.length, 1);
    assert.equal(result.artifacts[0].buttonText, PRIVATE_VISIBLE_TEXT);
    assert.equal(result.artifacts[0].artifact.visibleFilename, PRIVATE_VISIBLE_TEXT);
    assert.doesNotMatch(JSON.stringify(result), /private account label|token=secret/);
  });
});
