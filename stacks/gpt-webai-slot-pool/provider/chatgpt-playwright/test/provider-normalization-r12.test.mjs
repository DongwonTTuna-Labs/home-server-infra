import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const TEST_DIR = path.dirname(fileURLToPath(import.meta.url));
const STACK_ROOT = path.resolve(TEST_DIR, '../../..');
const SCRIPT = path.join(STACK_ROOT, 'scripts/check-provider-normalization-r12.mjs');

function records(relativePath) {
  return fs.readFileSync(path.join(STACK_ROOT, relativePath), 'utf8')
    .trimEnd()
    .split('\n')
    .map(line => JSON.parse(line));
}

function fixtureEnvironment(record) {
  return {
    ...process.env,
    GPT_WEBAI_FIXTURE_ID: record.fixture_id,
    GPT_WEBAI_NORMALIZATION_SCHEMA: 'pr72.provider_normalization.r12.v1',
    LANG: 'C.UTF-8',
    TZ: 'UTC',
  };
}

test('aggregate checker validates all immutable R12 inputs and semantic replay', () => {
  const result = spawnSync(process.execPath, [
    SCRIPT,
    '--inventory', 'contracts/provider-r12/provider-outcome-current.tsv',
    '--catalog', 'contracts/provider-r12/provider-outcome-normalized.tsv',
    '--legal-catalog', 'tests/fixtures/provider-r12/legal-catalog.jsonl',
    '--negative-catalog', 'tests/fixtures/provider-r12/negative-catalog.jsonl',
    '--semantic-replay', 'tests/fixtures/provider-r12/semantic-replay.jsonl',
  ], { cwd: STACK_ROOT, encoding: null });

  assert.equal(result.status, 0, result.stderr.toString());
  assert.equal(result.stdout.length, 0);
  assert.equal(result.stderr.length, 0);
});

test('legal fixture commands emit their exact catalog-selected lifecycle bytes and exit', () => {
  const legal = records('tests/fixtures/provider-r12/legal-catalog.jsonl');
  for (const record of [legal[0], legal.find(item => item.expected_lifecycle_rc === 70)]) {
    const result = spawnSync(process.execPath, [
      SCRIPT,
      '--catalog', 'contracts/provider-r12/provider-outcome-normalized.tsv',
      '--fixture', record.fixture_path,
    ], { cwd: STACK_ROOT, env: fixtureEnvironment(record), encoding: null });

    assert.equal(result.status, record.expected_lifecycle_rc, result.stderr.toString());
    assert.deepEqual(result.stdout, Buffer.from(record.expected_stdout_bytes_base64, 'base64'));
    assert.equal(result.stderr.length, 0);
  }
});

test('negative fixture replay preserves duplicate-key rejection and no-mutation output', () => {
  const record = records('tests/fixtures/provider-r12/negative-catalog.jsonl')
    .find(item => item.failure_class === 'input.duplicate_operation');
  const result = spawnSync(process.execPath, [
    SCRIPT,
    '--negative-input-base64', record.input_bytes_base64,
    '--fixture-id', record.fixture_id,
  ], { cwd: STACK_ROOT, env: fixtureEnvironment(record), encoding: null });

  assert.equal(result.status, 70, result.stderr.toString());
  assert.deepEqual(result.stdout, Buffer.from(record.expected_stdout_bytes_base64, 'base64'));
  assert.equal(result.stderr.length, 0);
  const output = JSON.parse(result.stdout);
  assert.equal(output.reason, 'input.duplicate_operation');
  assert.equal(output.stateOracle, 'state.unchanged');
  assert.equal(output.artifactEffectCount, 0);
  assert.equal(output.artifactReceiptCount, 0);
});
