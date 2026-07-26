import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';

import {
  artifactDownloadFailureStatus,
  artifactExpectationRequiresControls,
  artifactExpectedFromText,
  artifactFilenamesFromText,
  filenameFromText,
} from '../lib/common.mjs';

test('provider failpoint exits 99 with the exact stderr line and no stdout', () => {
  const moduleUrl = new URL('../lib/common.mjs', import.meta.url).href;
  const name = 'after-physical-send-click-before-provider-stdout';
  const result = spawnSync(
    process.execPath,
    ['--input-type=module', '--eval', `import { hitFailpoint } from ${JSON.stringify(moduleUrl)}; hitFailpoint(${JSON.stringify(name)});`],
    {
      encoding: null,
      env: { ...process.env, GPT_WEBAI_FAILPOINT: name },
    },
  );
  assert.equal(result.status, 99);
  assert.deepEqual(result.stdout, Buffer.alloc(0));
  assert.deepEqual(result.stderr, Buffer.from(`failpoint:${name}\n`));
});

test('patch-ready body text requires a downloadable artifact', () => {
  assert.equal(artifactExpectedFromText('PATCH_READY\n\nDiff\ndiff --git a/x b/x\n+x'), true);
  assert.equal(
    artifactDownloadFailureStatus({
      artifactExpected: true,
      artifacts: [],
      warnings: [],
      downloadCandidateCount: 0,
    }),
    'artifact.controls_absent',
  );
});

test('colonless ChatGPT download filename text requires artifact recovery', () => {
  assert.equal(
    artifactExpectedFromText('I prepared it.\n\nDownload pr72-start-unconfirmed.patch\n\nPatch SHA-256:\nabc'),
    true,
  );
});

test('artifact expectation ignores fenced diff examples after a real manifest-free review', () => {
  assert.equal(
    artifactExpectedFromText('LGTM_NO_BLOCKING\n\nNo artifact is required for this review.\n```diff\ndiff --git a/x b/x\n```'),
    false,
  );
});

test('explicit artifact expectation overrides answer-text inference safely', () => {
  assert.equal(artifactExpectedFromText('LGTM_NO_BLOCKING\n\nNo downloadable artifact requested.'), false);
  assert.equal(artifactExpectationRequiresControls('none', 'ARTIFACT_READY\npr72.zip'), false);
  assert.equal(artifactExpectationRequiresControls('required', 'Complete text answer.'), true);
  assert.equal(artifactExpectationRequiresControls('optional', 'Complete text answer.'), false);
  assert.equal(
    artifactDownloadFailureStatus({
      artifactExpected: false,
      artifacts: [],
      warnings: [],
      downloadCandidateCount: 0,
    }),
    '',
  );
});


test('no-artifact design and review verdicts can finish without download controls', () => {
  const designAnswer = 'DESIGN_READY\n\nNo downloadable artifact is required for this design review step.';
  const reviewAnswer = 'LGTM_NO_BLOCKING\n\nNo artifact is required for this independent review.';
  assert.equal(artifactExpectedFromText(designAnswer), false);
  assert.equal(artifactExpectedFromText(reviewAnswer), false);
  assert.equal(artifactExpectationRequiresControls('none', designAnswer), false);
  assert.equal(artifactDownloadFailureStatus({ artifactExpected: false, artifacts: [], warnings: [], downloadCandidateCount: 0 }), '');
});

test('required or claimed artifacts fail closed when controls and downloads are absent', () => {
  for (const expectation of ['required', 'claimed']) {
    const artifactExpected = artifactExpectationRequiresControls(expectation, 'ARTIFACT_READY\n\nArchive: pr72-source.tar.gz');
    assert.equal(artifactExpected, true);
    assert.equal(artifactDownloadFailureStatus({ artifactExpected, artifacts: [], warnings: [], downloadCandidateCount: 0 }), 'artifact.controls_absent');
  }
});


test('manifest filenames are treated as downloadable artifact filenames', () => {
  const answer = `ARTIFACT_READY

Manifest: pr72-corrected-source-tree-v3-20260709T0610Z.manifest`;
  assert.equal(filenameFromText('pr72-corrected-source-tree-v3-20260709T0610Z.manifest'), 'pr72-corrected-source-tree-v3-20260709T0610Z.manifest');
  assert.deepEqual(artifactFilenamesFromText(answer), ['pr72-corrected-source-tree-v3-20260709T0610Z.manifest']);
  assert.equal(artifactExpectedFromText('Manifest: pr72-corrected-source-tree-v3-20260709T0610Z.manifest'), true);
});
