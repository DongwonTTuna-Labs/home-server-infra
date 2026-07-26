import assert from 'node:assert/strict';
import { mkdirSync, writeFileSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';

import { downloadArtifacts } from '../lib/artifacts.mjs';
import {
  FakeDownload,
  FakePage,
  enoent,
  withArtifactRoot,
} from './artifacts-fixtures.mjs';

test('recovers saveAs ENOENT from the browser download path using suggested filename', async () => {
  await withArtifactRoot(async root => {
    const downloadsDir = path.join(root, 'downloads');
    const directPath = path.join(downloadsDir, 'browser-saved.zip');
    const payload = 'browser path payload\n';
    const page = new FakePage([
      new FakeDownload({
        suggestedFilename: 'browser-saved.zip',
        failWith: () => {
          mkdirSync(downloadsDir, { recursive: true });
          writeFileSync(directPath, payload);
          return enoent(`download.saveAs: ENOENT: no such file or directory, copyfile '${directPath}.tmp' -> '${path.join(downloadsDir, '001-artifact-1.zip')}'`);
        },
      }),
    ], [
      { visibleText: 'Download ZIP', accessibleName: 'Download ZIP' },
    ]);

    const result = await downloadArtifacts(page, 'session-direct-download', { turnIndexes: [0] });

    assert.deepEqual(result.warnings, []);
    assert.equal(result.artifacts.length, 1);
    assert.equal(result.artifacts[0].buttonText, 'Download ZIP');
    assert.equal(result.artifacts[0].artifact.status, 'saved');
    assert.equal(result.artifacts[0].artifact.suggestedFilename, 'browser-saved.zip');
    assert.equal(result.artifacts[0].artifact.recoveredFrom, 'browser.downloadPath');
    assert.equal(await readFile(result.artifacts[0].artifact.containerPath, 'utf8'), payload);
  });
});

test('recovers saveAs ENOENT from the browser default Downloads directory', async () => {
  await withArtifactRoot(async root => {
    const browserDownloadsDir = path.join(root, 'browser-downloads');
    process.env.GPT_WEBAI_BROWSER_DOWNLOAD_DIR = browserDownloadsDir;
    const payload = 'browser default downloads payload\n';
    const page = new FakePage([
      new FakeDownload({
        suggestedFilename: 'browser-default.zip',
        failWith: () => {
          mkdirSync(browserDownloadsDir, { recursive: true });
          writeFileSync(path.join(browserDownloadsDir, 'browser-default.zip'), payload);
          return enoent('download.saveAs: ENOENT: no such file or directory, copyfile /broker-artifacts/run/playwright-artifacts/guid -> /broker-artifacts/run/downloads/001-artifact-1.zip');
        },
      }),
    ], [
      { visibleText: 'Download ZIP', accessibleName: 'Download ZIP' },
    ]);

    const result = await downloadArtifacts(page, 'session-browser-default-download', { turnIndexes: [0] });

    assert.deepEqual(result.warnings, []);
    assert.equal(result.artifacts.length, 1);
    assert.equal(result.artifacts[0].artifact.status, 'saved');
    assert.equal(result.artifacts[0].artifact.suggestedFilename, 'browser-default.zip');
    assert.equal(result.artifacts[0].artifact.recoveredFrom, 'browser.downloadPath');
    assert.equal(await readFile(result.artifacts[0].artifact.containerPath, 'utf8'), payload);
  });
});
