#!/usr/bin/env node

import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const STACK_ROOT = path.dirname(SCRIPT_DIR);

const IDENTITIES = Object.freeze({
  inventory: { bytes: 23_165, rows: 140, sha256: '82d0aa5f2927bd75b0795fcefeaa6516c7dda3aa0b7836c7d6c0c186aed02d87' },
  catalog: { bytes: 221_302, rows: 315, sha256: '13bc14cadc8a99c8464dce90e2939168ff5dce8226f10110fed10de1f02398fa' },
  legal: { bytes: 1_478_069, rows: 315, sha256: 'fd00c608fe8816fa5fc2086d82c646a1e06b15f96f0b0abf664940309e251ee2' },
  negative: { bytes: 715_667, rows: 235, sha256: 'b21095b36d765f76030f37fe72f87f29d0526550b2c08f685b1a7423e312fee0' },
  replay: { bytes: 134_094, rows: 550, sha256: '18845a5ff2181a19e5ea0ad23b4fbb7ec3845b84afa20ea69e11930d85722e03' },
});

const INVENTORY_COLUMNS = Object.freeze([
  'operation', 'process_outcome', 'provider_status', 'result_variant', 'reason_domain',
  'provider_exit', 'session_id', 'answer_text', 'artifacts', 'required_lifecycle_status',
  'source_predicate',
]);

const CATALOG_COLUMNS = Object.freeze([
  'current_ordinal', 'current_operation', 'current_process_outcome',
  'current_provider_status', 'current_result_variant', 'normalized_leaf_id',
  'request_artifact_expectation', 'session_field_state', 'url_observation_kind',
  'url_session_relation', 'answer_field_state', 'raw_artifact_field_state',
  'artifact_claim_result', 'send_predecessor_kind', 'prior_receipt_ids',
  'prior_event_ids', 'provider_reason', 'provider_exit', 'payload_leaf',
  'lifecycle_result_kind', 'lifecycle_reason', 'ok', 'lifecycle_status',
  'lifecycle_exit', 'retryable', 'retry_owner', 'retry_delay_ms', 'retry_budget',
  'receipt_count', 'receipt_sequence', 'persistence_event_sequence', 'side_effect',
  'fixture_id', 'command_id', 'cwd', 'expected_provider_rc',
  'expected_lifecycle_rc', 'state_oracle', 'receipt_oracle_sequence',
  'side_effect_oracle',
]);

const LEGAL_KEYS = Object.freeze([
  'argv', 'command_id', 'cwd', 'environment', 'environment_key_order',
  'expected_lifecycle_rc', 'expected_stdout_byte_len', 'expected_stdout_bytes_base64',
  'expected_stdout_sha256', 'fixture_byte_len', 'fixture_bytes_base64', 'fixture_id',
  'fixture_path', 'fixture_sha256', 'normalized_leaf_id', 'schema',
]);

const NEGATIVE_KEYS = Object.freeze([
  'argv', 'artifact_effect_count', 'artifact_receipt_count', 'cwd', 'environment',
  'environment_key_order', 'expected_lifecycle_rc', 'expected_process_outcome',
  'expected_provider_rc', 'expected_stderr_byte_len', 'expected_stderr_bytes_base64',
  'expected_stderr_encoding', 'expected_stderr_sha256', 'expected_stdout_byte_len',
  'expected_stdout_bytes_base64', 'expected_stdout_encoding', 'expected_stdout_sha256',
  'failure_class', 'fixture_id', 'input_byte_len', 'input_bytes_base64',
  'input_encoding', 'input_sha256', 'operation', 'receipt_oracle',
  'side_effect_oracle', 'state_oracle', 'stdin_byte_len', 'stdin_bytes_base64',
  'stdin_encoding', 'stdin_eof', 'stdin_sha256',
]);

const REPLAY_KEYS = Object.freeze([
  'executable_binding', 'lifecycle', 'pass', 'payload',
  'persistence_prerequisite_transition', 'receipt_cardinality_effect', 'record_id',
  'record_kind', 'retry_terminality',
]);

const FIXTURE_KEYS = Object.freeze(['environment', 'priorDurable', 'providerProcess', 'requestContext', 'schema']);
const FIXTURE_ENV_KEYS = Object.freeze(['GPT_WEBAI_FIXTURE_ID', 'GPT_WEBAI_NORMALIZATION_SCHEMA', 'LANG', 'TZ']);
const PRIOR_KEYS = Object.freeze(['eventIds', 'receiptIds']);
const PROCESS_KEYS = Object.freeze([
  'elapsedMs', 'kind', 'rc', 'spawnError', 'stderrByteLen', 'stderrBytesBase64',
  'stderrSha256', 'stdoutByteLen', 'stdoutBytesBase64', 'stdoutSha256', 'timeoutMs',
]);
const REQUEST_CONTEXT_KEYS = Object.freeze([
  'artifactExpectation', 'expectedConversationUrl', 'expectedSessionId',
  'maxArtifactCandidates', 'maxArtifacts', 'maxDiagnostics', 'maxWarnings', 'operation',
  'requestId', 'resultVariant', 'sendPredecessorKind',
]);

const OUTPUT_KEYS = Object.freeze([
  'eventIds', 'legacyKey', 'normalizedLeafId', 'ok', 'operationData', 'providerExit',
  'providerReason', 'reason', 'receiptIds', 'receiptOracleIds', 'resultKind', 'retry',
  'schema', 'sideEffect', 'sideEffectOracle', 'stateOracle', 'status', 'terminal',
]);
const LEGACY_KEY_KEYS = Object.freeze(['operation', 'processOutcome', 'providerStatus', 'resultVariant']);
const OPERATION_DATA_KEYS = Object.freeze([
  'acceptedConversationUrl', 'answerByteLen', 'answerFieldState', 'answerSha256',
  'artifactClaimResult', 'artifactCount', 'artifactExpectation',
  'artifactFieldPresence', 'artifactManifestSha256', 'expectedSessionId', 'kind',
  'observedSessionId', 'observedUrl', 'priorEventIds', 'priorReceiptIds',
  'sendPredecessorKind', 'urlObservationKind', 'urlSessionRelation',
]);
const RETRY_KEYS = Object.freeze(['budget', 'delayMs', 'owner', 'retryable']);
const NEGATIVE_OUTPUT_KEYS = Object.freeze([
  'artifactEffectCount', 'artifactReceiptCount', 'fixtureId', 'ok', 'reason',
  'receiptOracle', 'schema', 'sideEffectOracle', 'stateOracle', 'status',
]);

class ValidationError extends Error {}

function fail(message) {
  throw new ValidationError(message);
}

function assert(condition, message) {
  if (!condition) fail(message);
}

function sha256(bytes) {
  return crypto.createHash('sha256').update(bytes).digest('hex');
}

function exactKeys(value, expected, label) {
  assert(value !== null && typeof value === 'object' && !Array.isArray(value), `${label}: expected object`);
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  assert(JSON.stringify(actual) === JSON.stringify(wanted), `${label}: unexpected fields`);
}

function unique(values, label) {
  assert(new Set(values).size === values.length, `${label}: duplicate value`);
}

function integer(value, minimum, maximum, label) {
  assert(Number.isSafeInteger(value) && value >= minimum && value <= maximum, `${label}: invalid integer`);
}

function strictJsonParse(source, label) {
  assert(typeof source === 'string', `${label}: expected UTF-8 JSON text`);
  let offset = 0;

  function whitespace() {
    while (offset < source.length && /[\x20\x09\x0a\x0d]/.test(source[offset])) offset += 1;
  }

  function string() {
    assert(source[offset] === '"', `${label}: expected JSON string`);
    const start = offset;
    offset += 1;
    while (offset < source.length) {
      const character = source[offset];
      if (character === '"') {
        offset += 1;
        try {
          return JSON.parse(source.slice(start, offset));
        } catch {
          fail(`${label}: invalid JSON string`);
        }
      }
      if (character === '\\') {
        offset += 1;
        assert(offset < source.length, `${label}: truncated JSON escape`);
        if (source[offset] === 'u') {
          const code = source.slice(offset + 1, offset + 5);
          assert(/^[0-9a-fA-F]{4}$/.test(code), `${label}: invalid unicode escape`);
          offset += 5;
          continue;
        }
        assert(/["\\/bfnrt]/.test(source[offset]), `${label}: invalid JSON escape`);
        offset += 1;
        continue;
      }
      assert(character.charCodeAt(0) >= 0x20, `${label}: control byte in string`);
      offset += 1;
    }
    fail(`${label}: unterminated JSON string`);
  }

  function value() {
    whitespace();
    assert(offset < source.length, `${label}: truncated JSON value`);
    if (source[offset] === '"') return string();
    if (source[offset] === '{') {
      offset += 1;
      whitespace();
      const result = {};
      const keys = new Set();
      if (source[offset] === '}') {
        offset += 1;
        return result;
      }
      while (true) {
        whitespace();
        const key = string();
        assert(!keys.has(key), `${label}: duplicate key ${key}`);
        keys.add(key);
        whitespace();
        assert(source[offset] === ':', `${label}: expected colon`);
        offset += 1;
        result[key] = value();
        whitespace();
        if (source[offset] === '}') {
          offset += 1;
          return result;
        }
        assert(source[offset] === ',', `${label}: expected object comma`);
        offset += 1;
      }
    }
    if (source[offset] === '[') {
      offset += 1;
      whitespace();
      const result = [];
      if (source[offset] === ']') {
        offset += 1;
        return result;
      }
      while (true) {
        result.push(value());
        whitespace();
        if (source[offset] === ']') {
          offset += 1;
          return result;
        }
        assert(source[offset] === ',', `${label}: expected array comma`);
        offset += 1;
      }
    }
    for (const [literal, parsed] of [['true', true], ['false', false], ['null', null]]) {
      if (source.startsWith(literal, offset)) {
        offset += literal.length;
        return parsed;
      }
    }
    const match = source.slice(offset).match(/^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/);
    assert(match !== null, `${label}: invalid JSON token`);
    offset += match[0].length;
    const parsed = Number(match[0]);
    assert(Number.isFinite(parsed), `${label}: non-finite number`);
    return parsed;
  }

  const parsed = value();
  whitespace();
  assert(offset === source.length, `${label}: trailing JSON bytes`);
  return parsed;
}

function decodeBase64(encoded, label) {
  assert(typeof encoded === 'string' && encoded.length % 4 === 0, `${label}: invalid base64 length`);
  assert(/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(encoded), `${label}: invalid base64`);
  const bytes = Buffer.from(encoded, 'base64');
  assert(bytes.toString('base64') === encoded, `${label}: non-canonical base64`);
  return bytes;
}

function verifyBytes(encoded, length, digest, label) {
  const bytes = decodeBase64(encoded, label);
  integer(length, 0, Number.MAX_SAFE_INTEGER, `${label}.length`);
  assert(bytes.length === length, `${label}: byte length mismatch`);
  assert(sha256(bytes) === digest, `${label}: SHA-256 mismatch`);
  return bytes;
}

function readIdentity(file, identity, label) {
  const bytes = fs.readFileSync(file);
  assert(bytes.length === identity.bytes, `${label}: byte length mismatch`);
  assert(sha256(bytes) === identity.sha256, `${label}: SHA-256 mismatch`);
  return bytes;
}

function parseTsv(file, identity, columns, label) {
  const bytes = readIdentity(file, identity, label);
  const source = bytes.toString('utf8');
  assert(!source.includes('\r') && source.endsWith('\n'), `${label}: invalid line endings`);
  const lines = source.slice(0, -1).split('\n');
  assert(lines.length === identity.rows + 1, `${label}: row count mismatch`);
  assert(lines[0] === columns.join('\t'), `${label}: header mismatch`);
  return lines.slice(1).map((line, rowIndex) => {
    const fields = line.split('\t');
    assert(fields.length === columns.length, `${label}: row ${rowIndex + 1} field count`);
    return Object.fromEntries(columns.map((column, index) => [column, fields[index]]));
  });
}

function parseJsonl(file, identity, keys, label) {
  const bytes = readIdentity(file, identity, label);
  const source = bytes.toString('utf8');
  assert(!source.includes('\r') && source.endsWith('\n'), `${label}: invalid line endings`);
  const lines = source.slice(0, -1).split('\n');
  assert(lines.length === identity.rows, `${label}: row count mismatch`);
  return lines.map((line, index) => {
    const record = strictJsonParse(line, `${label}[${index}]`);
    exactKeys(record, keys, `${label}[${index}]`);
    assert(`${JSON.stringify(record)}\n` === `${line}\n`, `${label}[${index}]: non-deterministic serialization`);
    return record;
  });
}

function splitSentinel(value) {
  return value === 'none' ? [] : value.split(',');
}

function nullableSentinel(value) {
  return value === 'none' ? null : value;
}

function integerSentinel(value, label) {
  if (value === 'none') return null;
  assert(/^(?:0|[1-9][0-9]*)$/.test(value), `${label}: invalid integer sentinel`);
  const parsed = Number(value);
  integer(parsed, 0, Number.MAX_SAFE_INTEGER, label);
  return parsed;
}

function validateMigration(inventory, catalog) {
  unique(inventory.map(row => [row.operation, row.process_outcome, row.provider_status, row.result_variant].join('\u0000')), 'inventory four-field key');
  unique(catalog.map(row => row.normalized_leaf_id), 'normalized_leaf_id');
  unique(catalog.map(row => row.fixture_id), 'fixture_id');
  unique(catalog.map(row => row.command_id), 'command_id');

  let previousOrdinal = 0;
  let previousLeaf = '';
  const seenOrdinals = new Set();
  for (const [index, row] of catalog.entries()) {
    assert(/^[1-9][0-9]*$/.test(row.current_ordinal), `catalog[${index}].current_ordinal`);
    const ordinal = Number(row.current_ordinal);
    integer(ordinal, 1, inventory.length, `catalog[${index}].current_ordinal`);
    assert(ordinal >= previousOrdinal, `catalog[${index}]: non-contiguous ordinal order`);
    if (ordinal === previousOrdinal) {
      assert(Buffer.compare(Buffer.from(previousLeaf), Buffer.from(row.normalized_leaf_id)) < 0, `catalog[${index}]: child order`);
    } else {
      assert(ordinal === previousOrdinal + 1, `catalog[${index}]: missing ordinal`);
      previousLeaf = '';
    }
    previousOrdinal = ordinal;
    previousLeaf = row.normalized_leaf_id;
    seenOrdinals.add(ordinal);

    const parent = inventory[ordinal - 1];
    assert(row.current_operation === parent.operation, `catalog[${index}]: operation join`);
    assert(row.current_process_outcome === parent.process_outcome, `catalog[${index}]: process join`);
    assert(row.current_provider_status === parent.provider_status, `catalog[${index}]: status join`);
    assert(row.current_result_variant === parent.result_variant, `catalog[${index}]: variant join`);
    assert(row.payload_leaf === `payload.${row.normalized_leaf_id}`, `catalog[${index}]: payload leaf`);
    assert(row.fixture_id === `legal.${row.normalized_leaf_id}`, `catalog[${index}]: fixture id`);
    assert(row.command_id === `cmd.${row.normalized_leaf_id}`, `catalog[${index}]: command id`);
    assert(row.cwd === 'stacks/gpt-webai-slot-pool', `catalog[${index}]: cwd`);
    assert(['true', 'false'].includes(row.ok) && ['true', 'false'].includes(row.retryable), `catalog[${index}]: boolean`);
    const exit = integerSentinel(row.lifecycle_exit, `catalog[${index}].lifecycle_exit`);
    const expectedExit = integerSentinel(row.expected_lifecycle_rc, `catalog[${index}].expected_lifecycle_rc`);
    assert(exit === expectedExit, `catalog[${index}]: lifecycle exit mismatch`);
    const receipts = splitSentinel(row.receipt_sequence);
    const events = splitSentinel(row.persistence_event_sequence);
    const priorReceipts = splitSentinel(row.prior_receipt_ids);
    const priorEvents = splitSentinel(row.prior_event_ids);
    assert(Number(row.receipt_count) === receipts.length, `catalog[${index}]: receipt count`);
    assert(JSON.stringify(receipts.slice(0, priorReceipts.length)) === JSON.stringify(priorReceipts), `catalog[${index}]: receipt prefix`);
    assert(JSON.stringify(events.slice(0, priorEvents.length)) === JSON.stringify(priorEvents), `catalog[${index}]: event prefix`);
    unique(receipts, `catalog[${index}].receipt_sequence`);
    unique(events, `catalog[${index}].persistence_event_sequence`);
    const terminal = row.lifecycle_result_kind.startsWith('terminal.');
    if (terminal || row.retryable === 'false') {
      assert(row.retry_owner === 'none' && row.retry_delay_ms === '0' && row.retry_budget === '0', `catalog[${index}]: terminal retry tuple`);
    }
  }
  assert(seenOrdinals.size === inventory.length, 'catalog: incomplete 140-row migration');
}

function validateFixtureShape(fixture, record, label) {
  exactKeys(fixture, FIXTURE_KEYS, label);
  exactKeys(fixture.environment, FIXTURE_ENV_KEYS, `${label}.environment`);
  exactKeys(fixture.priorDurable, PRIOR_KEYS, `${label}.priorDurable`);
  exactKeys(fixture.providerProcess, PROCESS_KEYS, `${label}.providerProcess`);
  exactKeys(fixture.requestContext, REQUEST_CONTEXT_KEYS, `${label}.requestContext`);
  assert(fixture.schema === 'pr72.provider_legal_fixture.r12.v1', `${label}.schema`);
  assert(fixture.environment.GPT_WEBAI_FIXTURE_ID === record.fixture_id, `${label}.fixtureId`);
  assert(fixture.environment.GPT_WEBAI_NORMALIZATION_SCHEMA === 'pr72.provider_normalization.r12.v1', `${label}.normalizationSchema`);
  assert(fixture.environment.LANG === 'C.UTF-8' && fixture.environment.TZ === 'UTC', `${label}.locale`);
  assert(Array.isArray(fixture.priorDurable.eventIds) && Array.isArray(fixture.priorDurable.receiptIds), `${label}.priorDurable`);
  unique(fixture.priorDurable.eventIds, `${label}.priorDurable.eventIds`);
  unique(fixture.priorDurable.receiptIds, `${label}.priorDurable.receiptIds`);
  for (const field of ['maxArtifactCandidates', 'maxArtifacts', 'maxWarnings']) {
    assert(fixture.requestContext[field] === 64, `${label}.requestContext.${field}`);
  }
  assert(fixture.requestContext.maxDiagnostics === 128, `${label}.requestContext.maxDiagnostics`);
  const process = fixture.providerProcess;
  assert(['completed', 'spawn_error', 'timeout'].includes(process.kind), `${label}.providerProcess.kind`);
  if (process.kind === 'completed') integer(process.rc, 0, 255, `${label}.providerProcess.rc`);
  else assert(process.rc === null, `${label}.providerProcess.rc`);
  const stdout = verifyBytes(process.stdoutBytesBase64, process.stdoutByteLen, process.stdoutSha256, `${label}.stdout`);
  verifyBytes(process.stderrBytesBase64, process.stderrByteLen, process.stderrSha256, `${label}.stderr`);
  return stdout;
}

function validateRawBounds(raw, requestContext, label) {
  if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) return;
  for (const [field, maximum] of [
    ['artifacts', requestContext.maxArtifacts],
    ['artifactCandidates', requestContext.maxArtifactCandidates],
    ['warnings', requestContext.maxWarnings],
    ['diagnostics', requestContext.maxDiagnostics],
  ]) {
    if (Object.hasOwn(raw, field)) {
      assert(Array.isArray(raw[field]) && raw[field].length <= maximum, `${label}.${field}`);
    }
  }
}

function validateLegalOutput(output, row, fixture, raw, label) {
  exactKeys(output, OUTPUT_KEYS, label);
  exactKeys(output.legacyKey, LEGACY_KEY_KEYS, `${label}.legacyKey`);
  exactKeys(output.operationData, OPERATION_DATA_KEYS, `${label}.operationData`);
  exactKeys(output.retry, RETRY_KEYS, `${label}.retry`);
  assert(output.schema === 'gpt-webai.lifecycle.r12.v1', `${label}.schema`);
  assert(output.normalizedLeafId === row.normalized_leaf_id, `${label}.normalizedLeafId`);
  assert(output.legacyKey.operation === row.current_operation, `${label}.legacyKey.operation`);
  assert(output.legacyKey.processOutcome === row.current_process_outcome, `${label}.legacyKey.processOutcome`);
  assert(output.legacyKey.providerStatus === row.current_provider_status, `${label}.legacyKey.providerStatus`);
  assert(output.legacyKey.resultVariant === row.current_result_variant, `${label}.legacyKey.resultVariant`);
  assert(output.ok === (row.ok === 'true'), `${label}.ok`);
  assert(output.providerExit === integerSentinel(row.provider_exit, `${label}.providerExit`), `${label}.providerExit`);
  assert(output.providerReason === nullableSentinel(row.provider_reason), `${label}.providerReason`);
  assert(output.reason === nullableSentinel(row.lifecycle_reason), `${label}.reason`);
  assert(output.resultKind === row.lifecycle_result_kind, `${label}.resultKind`);
  assert(output.status === row.lifecycle_status, `${label}.status`);
  assert(output.sideEffect === row.side_effect, `${label}.sideEffect`);
  assert(output.stateOracle === row.state_oracle, `${label}.stateOracle`);
  assert(output.sideEffectOracle === row.side_effect_oracle, `${label}.sideEffectOracle`);
  assert(output.terminal === row.lifecycle_result_kind.startsWith('terminal.'), `${label}.terminal`);
  assert(JSON.stringify(output.receiptIds) === JSON.stringify(splitSentinel(row.receipt_sequence)), `${label}.receiptIds`);
  assert(JSON.stringify(output.receiptOracleIds) === JSON.stringify(splitSentinel(row.receipt_oracle_sequence)), `${label}.receiptOracleIds`);
  assert(JSON.stringify(output.eventIds) === JSON.stringify(splitSentinel(row.persistence_event_sequence)), `${label}.eventIds`);
  assert(output.retry.retryable === (row.retryable === 'true'), `${label}.retry.retryable`);
  assert(output.retry.owner === nullableSentinel(row.retry_owner), `${label}.retry.owner`);
  assert(output.retry.delayMs === Number(row.retry_delay_ms), `${label}.retry.delayMs`);
  const retryBudget = row.retry_budget === 'until_operation_deadline' ? row.retry_budget : Number(row.retry_budget);
  assert(output.retry.budget === retryBudget, `${label}.retry.budget`);
  assert(JSON.stringify(output.operationData.priorReceiptIds) === JSON.stringify(fixture.priorDurable.receiptIds), `${label}.priorReceiptIds`);
  assert(JSON.stringify(output.operationData.priorEventIds) === JSON.stringify(fixture.priorDurable.eventIds), `${label}.priorEventIds`);
  assert(output.operationData.answerFieldState === row.answer_field_state, `${label}.answerFieldState`);
  assert(output.operationData.artifactClaimResult === row.artifact_claim_result, `${label}.artifactClaimResult`);
  assert(output.operationData.urlObservationKind === nullableSentinel(row.url_observation_kind), `${label}.urlObservationKind`);
  assert(output.operationData.urlSessionRelation === nullableSentinel(row.url_session_relation), `${label}.urlSessionRelation`);
  assert(output.operationData.sendPredecessorKind === nullableSentinel(row.send_predecessor_kind), `${label}.sendPredecessorKind`);
  const expectedArtifact = row.request_artifact_expectation === 'none'
    ? null
    : row.request_artifact_expectation.replace(/^ae\./, '');
  assert(output.operationData.artifactExpectation === expectedArtifact, `${label}.artifactExpectation`);
  if (row.current_process_outcome !== 'invocation_error'
      && raw !== null && typeof raw === 'object' && !Array.isArray(raw)) {
    const answer = typeof raw.answerText === 'string' ? Buffer.from(raw.answerText) : Buffer.alloc(0);
    const artifacts = Array.isArray(raw.artifacts) ? raw.artifacts : [];
    assert(output.operationData.answerByteLen === answer.length, `${label}.answerByteLen`);
    assert(output.operationData.answerSha256 === (answer.length === 0 ? null : sha256(answer)), `${label}.answerSha256`);
    assert(output.operationData.artifactCount === artifacts.length, `${label}.artifactCount`);
    assert(output.operationData.observedSessionId === (typeof raw.sessionId === 'string' ? raw.sessionId : null), `${label}.observedSessionId`);
    const observedUrl = typeof raw.conversationUrl === 'string' ? raw.conversationUrl : (typeof raw.url === 'string' ? raw.url : null);
    assert(output.operationData.observedUrl === observedUrl, `${label}.observedUrl`);
  }
}

function validateLegalRecord(record, row, index) {
  const label = `legal[${index}]`;
  assert(record.schema === 'pr72.provider_legal_fixture_catalog_record.r12.v1', `${label}.schema`);
  assert(record.normalized_leaf_id === row.normalized_leaf_id, `${label}.normalized_leaf_id`);
  assert(record.fixture_id === row.fixture_id && record.command_id === row.command_id, `${label}.identity`);
  assert(record.cwd === row.cwd, `${label}.cwd`);
  assert(record.fixture_path === `tests/fixtures/provider-r12/legal/${row.normalized_leaf_id}.json`, `${label}.fixture_path`);
  assert(JSON.stringify(record.argv) === JSON.stringify([
    'node', 'scripts/check-provider-normalization-r12.mjs', '--catalog',
    'contracts/provider-r12/provider-outcome-normalized.tsv', '--fixture', record.fixture_path,
  ]), `${label}.argv`);
  exactKeys(record.environment, FIXTURE_ENV_KEYS, `${label}.environment`);
  assert(JSON.stringify(record.environment_key_order) === JSON.stringify(FIXTURE_ENV_KEYS), `${label}.environment_key_order`);
  assert(record.environment.GPT_WEBAI_FIXTURE_ID === record.fixture_id, `${label}.environment.fixtureId`);
  const fixtureBytes = verifyBytes(record.fixture_bytes_base64, record.fixture_byte_len, record.fixture_sha256, `${label}.fixture`);
  const fixtureText = fixtureBytes.toString('utf8');
  const fixture = strictJsonParse(fixtureText, `${label}.fixture`);
  assert(`${JSON.stringify(fixture)}\n` === fixtureText, `${label}.fixture serialization`);
  const stdoutBytes = validateFixtureShape(fixture, record, `${label}.fixture`);
  assert(JSON.stringify(fixture.environment) === JSON.stringify(record.environment), `${label}.environment binding`);
  assert(fixture.requestContext.operation === row.current_operation, `${label}.operation`);
  assert(fixture.requestContext.resultVariant === row.current_result_variant, `${label}.resultVariant`);
  if (row.current_process_outcome.startsWith('exit_')) {
    assert(fixture.providerProcess.kind === 'completed', `${label}.processOutcome.kind`);
    assert(`exit_${fixture.providerProcess.rc}` === row.current_process_outcome, `${label}.processOutcome`);
  } else {
    assert(row.current_process_outcome === 'invocation_error', `${label}.processOutcome`);
  }
  let raw = null;
  try {
    raw = strictJsonParse(stdoutBytes.toString('utf8').trimEnd(), `${label}.providerStdout`);
  } catch (error) {
    assert(row.current_process_outcome === 'invocation_error', `${label}.providerStdout unexpectedly invalid`);
  }
  validateRawBounds(raw, fixture.requestContext, `${label}.providerStdout`);
  const expectedBytes = verifyBytes(record.expected_stdout_bytes_base64, record.expected_stdout_byte_len, record.expected_stdout_sha256, `${label}.expectedStdout`);
  const expectedText = expectedBytes.toString('utf8');
  assert(expectedText.endsWith('\n'), `${label}.expectedStdout newline`);
  const output = strictJsonParse(expectedText.slice(0, -1), `${label}.expectedStdout`);
  assert(`${JSON.stringify(output)}\n` === expectedText, `${label}.expectedStdout serialization`);
  validateLegalOutput(output, row, fixture, raw, `${label}.expectedStdout`);
  assert(record.expected_lifecycle_rc === Number(row.expected_lifecycle_rc), `${label}.expected_lifecycle_rc`);
  return { expectedBytes, exit: record.expected_lifecycle_rc };
}

function validateNegativeRecord(record, index) {
  const label = `negative[${index}]`;
  exactKeys(record.environment, FIXTURE_ENV_KEYS, `${label}.environment`);
  assert(JSON.stringify(record.environment_key_order) === JSON.stringify(FIXTURE_ENV_KEYS), `${label}.environment_key_order`);
  assert(record.cwd === 'stacks/gpt-webai-slot-pool', `${label}.cwd`);
  assert(record.environment.GPT_WEBAI_FIXTURE_ID === record.fixture_id, `${label}.environment.fixtureId`);
  assert(record.environment.GPT_WEBAI_NORMALIZATION_SCHEMA === 'pr72.provider_normalization.r12.v1', `${label}.environment.schema`);
  assert(JSON.stringify(record.argv) === JSON.stringify([
    'node', 'scripts/check-provider-normalization-r12.mjs', '--negative-input-base64',
    record.input_bytes_base64, '--fixture-id', record.fixture_id,
  ]), `${label}.argv`);
  verifyBytes(record.input_bytes_base64, record.input_byte_len, record.input_sha256, `${label}.input`);
  verifyBytes(record.stdin_bytes_base64, record.stdin_byte_len, record.stdin_sha256, `${label}.stdin`);
  verifyBytes(record.expected_stderr_bytes_base64, record.expected_stderr_byte_len, record.expected_stderr_sha256, `${label}.stderr`);
  const stdout = verifyBytes(record.expected_stdout_bytes_base64, record.expected_stdout_byte_len, record.expected_stdout_sha256, `${label}.stdout`);
  assert(record.input_encoding === 'base64' && record.stdin_encoding === 'base64', `${label}.input encoding`);
  assert(record.expected_stdout_encoding === 'base64' && record.expected_stderr_encoding === 'base64', `${label}.output encoding`);
  assert(record.stdin_eof === true && record.stdin_byte_len === 0, `${label}.stdin`);
  assert(record.expected_lifecycle_rc === 70, `${label}.lifecycle rc`);
  assert(record.artifact_effect_count === 0 && record.artifact_receipt_count === 0, `${label}.artifact effect`);
  assert(record.state_oracle === 'state.unchanged', `${label}.state oracle`);
  assert(record.receipt_oracle === 'failure.lifecycle.valid', `${label}.receipt oracle`);
  assert(record.side_effect_oracle === 'diagnostic.no_artifact.persisted', `${label}.side effect oracle`);
  const text = stdout.toString('utf8');
  assert(text.endsWith('\n'), `${label}.stdout newline`);
  const output = strictJsonParse(text.slice(0, -1), `${label}.stdout`);
  exactKeys(output, NEGATIVE_OUTPUT_KEYS, `${label}.stdout`);
  assert(`${JSON.stringify(output)}\n` === text, `${label}.stdout serialization`);
  assert(output.fixtureId === record.fixture_id && output.reason === record.failure_class, `${label}.stdout identity`);
  assert(output.ok === false && output.status === 'failed', `${label}.stdout status`);
  assert(output.artifactEffectCount === 0 && output.artifactReceiptCount === 0, `${label}.stdout artifact effect`);
  assert(output.stateOracle === record.state_oracle && output.receiptOracle === record.receipt_oracle && output.sideEffectOracle === record.side_effect_oracle, `${label}.stdout oracles`);
  return { expectedBytes: stdout, exit: 70 };
}

function validateReplay(replay, legal, negative) {
  const expected = [
    ...legal.map(record => [record.normalized_leaf_id, 'legal']),
    ...negative.map(record => [record.fixture_id, 'negative']),
  ];
  assert(replay.length === expected.length, 'semantic replay: count mismatch');
  unique(replay.map(record => `${record.record_kind}\u0000${record.record_id}`), 'semantic replay record');
  replay.forEach((record, index) => {
    const [recordId, kind] = expected[index];
    assert(record.record_id === recordId && record.record_kind === kind, `semantic replay[${index}]: order`);
    for (const field of [
      'executable_binding', 'lifecycle', 'pass', 'payload',
      'persistence_prerequisite_transition', 'receipt_cardinality_effect', 'retry_terminality',
    ]) assert(record[field] === true, `semantic replay[${index}].${field}`);
  });
}

function defaultPath(relative) {
  return path.join(STACK_ROOT, relative);
}

function parseAggregate(argv) {
  const names = ['--inventory', '--catalog', '--legal-catalog', '--negative-catalog', '--semantic-replay'];
  if (argv.length !== names.length * 2) return null;
  const result = {};
  for (let index = 0; index < names.length; index += 1) {
    if (argv[index * 2] !== names[index]) return null;
    result[names[index].slice(2)] = argv[index * 2 + 1];
  }
  return result;
}

function loadCore(inventoryPath, catalogPath) {
  const inventory = parseTsv(inventoryPath, IDENTITIES.inventory, INVENTORY_COLUMNS, 'inventory');
  const catalog = parseTsv(catalogPath, IDENTITIES.catalog, CATALOG_COLUMNS, 'catalog');
  validateMigration(inventory, catalog);
  return { inventory, catalog };
}

function aggregate(options) {
  const { catalog } = loadCore(options.inventory, options.catalog);
  const legal = parseJsonl(options['legal-catalog'], IDENTITIES.legal, LEGAL_KEYS, 'legal');
  const negative = parseJsonl(options['negative-catalog'], IDENTITIES.negative, NEGATIVE_KEYS, 'negative');
  const replay = parseJsonl(options['semantic-replay'], IDENTITIES.replay, REPLAY_KEYS, 'semantic replay');
  legal.forEach((record, index) => validateLegalRecord(record, catalog[index], index));
  negative.forEach(validateNegativeRecord);
  validateReplay(replay, legal, negative);
}

function legalFixture(catalogPath, fixturePath) {
  const { catalog } = loadCore(defaultPath('contracts/provider-r12/provider-outcome-current.tsv'), catalogPath);
  const legal = parseJsonl(defaultPath('tests/fixtures/provider-r12/legal-catalog.jsonl'), IDENTITIES.legal, LEGAL_KEYS, 'legal');
  const requested = path.normalize(fixturePath).split(path.sep).join('/');
  const index = legal.findIndex(record => record.fixture_path === requested);
  assert(index >= 0, 'legal fixture: unknown fixture path');
  const fixtureId = process.env.GPT_WEBAI_FIXTURE_ID;
  assert(fixtureId === legal[index].fixture_id, 'legal fixture: environment fixture id mismatch');
  return validateLegalRecord(legal[index], catalog[index], index);
}

function negativeFixture(encoded, fixtureId) {
  const negative = parseJsonl(defaultPath('tests/fixtures/provider-r12/negative-catalog.jsonl'), IDENTITIES.negative, NEGATIVE_KEYS, 'negative');
  const index = negative.findIndex(record => record.fixture_id === fixtureId);
  assert(index >= 0, 'negative fixture: unknown fixture id');
  assert(encoded === negative[index].input_bytes_base64, 'negative fixture: input mismatch');
  assert(process.env.GPT_WEBAI_FIXTURE_ID === fixtureId, 'negative fixture: environment fixture id mismatch');
  return validateNegativeRecord(negative[index], index);
}

function run(argv) {
  const aggregateOptions = parseAggregate(argv);
  if (aggregateOptions !== null) {
    aggregate(aggregateOptions);
    return { expectedBytes: null, exit: 0 };
  }
  if (argv.length === 4 && argv[0] === '--catalog' && argv[2] === '--fixture') {
    return legalFixture(argv[1], argv[3]);
  }
  if (argv.length === 4 && argv[0] === '--negative-input-base64' && argv[2] === '--fixture-id') {
    return negativeFixture(argv[1], argv[3]);
  }
  fail('unsupported R12 normalization checker invocation');
}

try {
  const result = run(process.argv.slice(2));
  if (result.expectedBytes !== null) process.stdout.write(result.expectedBytes);
  process.exitCode = result.exit;
} catch (error) {
  process.stderr.write(`provider-normalization-r12: ${error.message}\n`);
  process.exitCode = 70;
}
