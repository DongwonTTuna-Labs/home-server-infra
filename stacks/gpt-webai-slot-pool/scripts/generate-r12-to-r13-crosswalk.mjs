#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const STACK_ROOT = path.dirname(SCRIPT_DIR);

const HEADER = Object.freeze([
  'normalizedLeafId',
  'r13ResponseDiscriminant',
  'requiredProofOrReceipt',
  'r13EventSequence',
  'lifecycleResultKind',
  'exit',
  'failClosedResultKind',
]);

const CATALOG_COLUMNS = Object.freeze([
  'current_operation',
  'lifecycle_result_kind',
  'lifecycle_reason',
  'normalized_leaf_id',
  'ok',
  'persistence_event_sequence',
  'receipt_sequence',
  'request_artifact_expectation',
]);

const FAIL_CLOSED_BY_OPERATION = Object.freeze({
  'capture.root': 'run.model_failed',
  'capture.session': 'show.content_unavailable',
  download: 'download.content_unavailable',
  poll: 'run.poll_failed',
  send: 'run.send_failed',
  'session.resume': 'resume.content_unavailable',
  'session.show': 'show.content_unavailable',
  status: 'status.runtime_probe_failed',
});

class GenerationError extends Error {}

function fail(message) {
  throw new GenerationError(message);
}

function assert(condition, message) {
  if (!condition) fail(message);
}

function parseArgs(argv) {
  const options = {
    catalog: path.join(STACK_ROOT, 'contracts/provider-r12/provider-outcome-normalized.tsv'),
    eventsSource: path.join(
      STACK_ROOT,
      'crates/gpt-webai-lifecycle/src/provider_normalization_r12/events.rs',
    ),
    output: path.join(STACK_ROOT, 'contracts/provider-r12/r12-to-r13-crosswalk.tsv'),
    resultMatrix: path.join(
      STACK_ROOT,
      'crates/gpt-webai-lifecycle/src/contracts/cli.rs',
    ),
  };
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index];
    const value = argv[index + 1];
    if (!['--catalog', '--events-source', '--output', '--result-matrix'].includes(option)) {
      fail(`unknown argument: ${option}`);
    }
    assert(value !== undefined && !value.startsWith('--'), `${option} requires a path`);
    const key = {
      '--catalog': 'catalog',
      '--events-source': 'eventsSource',
      '--output': 'output',
      '--result-matrix': 'resultMatrix',
    }[option];
    options[key] = path.resolve(value);
    index += 1;
  }
  return options;
}

function canonicalText(file, label) {
  const bytes = fs.readFileSync(file);
  assert(bytes.length > 0, `${label}: empty input`);
  assert(!bytes.subarray(0, 3).equals(Buffer.from([0xef, 0xbb, 0xbf])), `${label}: BOM`);
  assert(bytes[bytes.length - 1] === 0x0a, `${label}: missing final LF`);
  assert(!bytes.includes(0x0d) && !bytes.includes(0x00), `${label}: forbidden byte`);
  const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  for (const line of text.slice(0, -1).split('\n')) {
    assert(!/[ \t]$/.test(line), `${label}: trailing whitespace`);
  }
  return text;
}

function parseCatalog(file) {
  const lines = canonicalText(file, 'normalized catalog').slice(0, -1).split('\n');
  const header = lines.shift().split('\t');
  assert(new Set(header).size === header.length, 'normalized catalog: duplicate column');
  for (const column of CATALOG_COLUMNS) {
    assert(header.includes(column), `normalized catalog: missing ${column}`);
  }
  assert(lines.length === 315, `normalized catalog: expected 315 leaves, got ${lines.length}`);
  const rows = lines.map((line, rowIndex) => {
    const fields = line.split('\t');
    assert(fields.length === header.length, `normalized catalog row ${rowIndex + 2}: field count`);
    return Object.fromEntries(fields.map((value, index) => [header[index], value]));
  });
  const leaves = rows.map((row) => row.normalized_leaf_id);
  assert(leaves.every((leaf) => leaf.length > 0), 'normalized catalog: empty leaf id');
  assert(new Set(leaves).size === 315, 'normalized catalog: duplicate leaf id');
  return rows;
}

function parseRustResultMatrix(file) {
  const source = fs.readFileSync(file, 'utf8');
  const universe = new Set();
  const constants = source.matchAll(
    /const\s+[A-Z_]+_RESULTS:\s*&str\s*=\s*"([\s\S]*?)";/g,
  );
  for (const match of constants) {
    const value = match[1].replace(/\\\s*/g, ' ');
    for (const resultKind of value.split(/\s+/).filter(Boolean)) universe.add(resultKind);
  }
  assert(universe.size > 0, 'result matrix: no result kinds');

  const start = source.indexOf('pub fn result_spec');
  const end = source.indexOf('impl LifecycleEnvelope', start);
  assert(start >= 0 && end > start, 'result matrix: result_spec function missing');
  const body = source.slice(start, end);
  const overrides = new Map();
  const blocks = body.matchAll(
    /if matches!\(\s*result_kind,([\s\S]*?)\)\s*\{\s*return Some\(ResultSpec \{([\s\S]*?)\}\);\s*\}/g,
  );
  for (const block of blocks) {
    const kinds = [...block[1].matchAll(/"([^"]+)"/g)].map((match) => match[1]);
    const spec = parseResultSpecFields(block[2]);
    for (const resultKind of kinds) {
      assert(universe.has(resultKind), `result matrix: override outside universe: ${resultKind}`);
      assert(!overrides.has(resultKind), `result matrix: duplicate override: ${resultKind}`);
      overrides.set(resultKind, spec);
    }
  }
  assert(
    /Some\(ResultSpec \{\s*exit_code:\s*70,\s*ok:\s*false,\s*reason_required:\s*true,\s*terminal:\s*true,\s*\}\)\s*\}/s.test(body),
    'result matrix: closed default failure spec changed',
  );
  return {
    get(resultKind) {
      assert(universe.has(resultKind), `result matrix: unknown result kind: ${resultKind}`);
      return overrides.get(resultKind) ?? {
        exitCode: 70,
        ok: false,
        reasonRequired: true,
        terminal: true,
      };
    },
  };
}

function parseResultSpecFields(source) {
  const integer = (name) => {
    const match = source.match(new RegExp(`${name}:\\s*(\\d+)`));
    assert(match !== null, `result matrix: missing ${name}`);
    return Number(match[1]);
  };
  const boolean = (name) => {
    const match = source.match(new RegExp(`${name}:\\s*(true|false)`));
    assert(match !== null, `result matrix: missing ${name}`);
    return match[1] === 'true';
  };
  return {
    exitCode: integer('exit_code'),
    ok: boolean('ok'),
    reasonRequired: boolean('reason_required'),
    terminal: boolean('terminal'),
  };
}

function parseRustEventMap(file) {
  const source = fs.readFileSync(file, 'utf8');
  const map = new Map();
  for (const match of source.matchAll(/"([A-Za-z0-9_]+R12)"\s*=>\s*EventType::([A-Za-z0-9_]+),/g)) {
    assert(!map.has(match[1]), `event map: duplicate token ${match[1]}`);
    map.set(match[1], match[2]);
  }
  assert(map.size === 23, `event map: expected 23 direct mappings, got ${map.size}`);
  assert(
    source.includes('"ProviderSessionUrlRejectedR12" => match stage'),
    'event map: stage-dependent URL mapping missing',
  );
  return map;
}

function tokens(cell, label) {
  if (cell === 'none') return [];
  assert(cell.length > 0, `${label}: empty token list`);
  const values = cell.split(',');
  assert(values.every((value) => value.length > 0), `${label}: empty token`);
  assert(new Set(values).size === values.length, `${label}: duplicate token`);
  return values;
}

function translateEvents(row, eventMap) {
  const translated = [];
  for (const token of tokens(row.persistence_event_sequence, `${row.normalized_leaf_id} events`)) {
    if (token.startsWith('event.prior.')) continue;
    assert(token.startsWith('event.'), `${row.normalized_leaf_id}: invalid event token ${token}`);
    const retained = token.slice('event.'.length);
    if (retained === 'ProviderSessionUrlRejectedR12') {
      if (row.current_operation === 'poll') translated.push('PollFailed');
      else if (
        ['capture.session', 'download', 'session.resume', 'session.show'].includes(
          row.current_operation,
        )
      ) translated.push('SessionOperationFailed');
      else fail(`${row.normalized_leaf_id}: URL rejection has no operation-stage resolution`);
      continue;
    }
    const event = eventMap.get(retained);
    assert(event !== undefined, `${row.normalized_leaf_id}: unmapped event ${retained}`);
    translated.push(event);
  }
  return translated;
}

function contractClass(reason) {
  return (
    reason === 'visual_failure_reason' ||
    reason.startsWith('provider.contract.') ||
    [
      'provider.invalid_json',
      'provider.io_error',
      'provider.stderr_too_large',
      'provider.stdout_too_large',
      'provider.timeout',
    ].includes(reason)
  );
}

function responseOperation(row, receipts) {
  switch (row.current_operation) {
    case 'capture.root':
      return 'capture.root';
    case 'capture.session':
    case 'session.show':
      return 'session-rebind';
    case 'send':
      if (receipts.includes('receipt.send.post_click')) return 'send-click';
      if (receipts.includes('receipt.send.reconciled_turn_start')) return 'send-reconcile';
      return 'send-click';
    case 'session.resume':
      return receipts.some((receipt) => receipt.startsWith('receipt.poll.'))
        ? 'poll'
        : 'session-rebind';
    case 'download':
      if (receipts.includes('receipt.artifact.download')) return 'artifact-click-save';
      if (receipts.includes('receipt.claim.zero')) return 'artifact-discover';
      return 'session-rebind';
    case 'poll':
      return 'poll';
    case 'status':
      return 'status';
    default:
      fail(`${row.normalized_leaf_id}: unknown retained operation ${row.current_operation}`);
  }
}

function responsePolarity(row) {
  if (
    row.current_operation === 'status' &&
    row.lifecycle_result_kind !== 'terminal.invocation_failure'
  ) return 'success';
  if (
    row.current_operation === 'poll' &&
    row.lifecycle_result_kind === 'nonterminal.poll_running_unverified'
  ) return 'success';
  return row.ok === 'true' ? 'success' : 'failure';
}

function requiredProof(row, operation, receipts) {
  switch (operation) {
    case 'capture.root':
      return receipts.includes('receipt.capture.root') ? 'root_binding_candidate' : null;
    case 'session-rebind':
      return receipts.some((receipt) =>
        ['receipt.capture.session', 'receipt.session.resume', 'receipt.session.show'].includes(
          receipt,
        ))
        ? 'session_echo'
        : null;
    case 'send-click':
      return receipts.includes('receipt.send.pre_click') && receipts.includes('receipt.send.post_click')
        ? 'send_receipt.post_click'
        : null;
    case 'send-reconcile':
      return receipts.includes('receipt.send.pre_click') &&
        receipts.includes('receipt.send.reconciled_turn_start')
        ? 'send_receipt.reconciled_turn_start'
        : null;
    case 'poll':
      return receipts.some((receipt) =>
        [
          'receipt.answer.poll_terminal',
          'receipt.artifact.poll_terminal',
          'receipt.poll.failure',
          'receipt.poll.progress',
        ].includes(receipt))
        ? 'poll_receipt'
        : null;
    case 'artifact-click-save':
      return receipts.includes('receipt.artifact.download')
        ? 'playwright_download_receipt'
        : null;
    case 'artifact-discover':
      return receipts.includes('receipt.claim.zero') ? 'zero_control_proof' : null;
    case 'status':
      return receipts.includes('receipt.status') ? 'status_probe' : null;
    default:
      fail(`${row.normalized_leaf_id}: no proof rule for ${operation}`);
  }
}

function resultForRow(row, events) {
  const kind = row.lifecycle_result_kind;
  const reason = row.lifecycle_reason;
  switch (row.current_operation) {
    case 'capture.root':
      if (kind === 'terminal.capture_success') return 'run.running';
      return 'run.model_failed';
    case 'capture.session':
      if (kind === 'terminal.capture_success') return 'show.running';
      return 'show.content_unavailable';
    case 'send':
      if (kind === 'nonterminal.send_success') return 'run.running';
      if (events.includes('UploadFailed')) return 'run.upload_failed';
      if (events.includes('ModelSelectionFailed')) return 'run.model_failed';
      if (events.includes('SendUncertain')) return 'run.send_uncertain';
      if (events.includes('SlotHealthObserved')) return 'run.slot_readiness_failed';
      return 'run.send_failed';
    case 'poll':
      if (kind === 'nonterminal.poll_running' || kind === 'nonterminal.poll_running_unverified') {
        return 'run.running';
      }
      if (kind === 'terminal.poll_terminal_success') return 'run.terminal_success';
      if (kind === 'terminal.poll_terminal_artifact_failure') return 'run.artifact_required_failed';
      return 'run.poll_failed';
    case 'session.resume':
      if (kind === 'nonterminal.resume_running' || kind === 'terminal.resume_idle') {
        return 'resume.running';
      }
      if (kind === 'terminal.resume_terminal_success') return 'resume.terminal_success';
      if (events.includes('ArtifactClaimFailed')) return 'resume.artifact_required_failed';
      if (reason === 'session.url_mismatch') return 'resume.url_rejected';
      if (['provider.limit', 'provider.schema_drift'].includes(reason)) {
        return 'resume.provider_blocked';
      }
      return 'resume.content_unavailable';
    case 'session.show':
      if (kind === 'nonterminal.show_running') return 'show.running';
      if (kind === 'terminal.show_idle') return 'show.idle';
      if (kind === 'terminal.show_terminal') return 'show.terminal';
      if (reason === 'session.url_mismatch') return 'show.url_rejected';
      if (['provider.limit', 'provider.schema_drift'].includes(reason)) {
        return 'show.provider_blocked';
      }
      return 'show.content_unavailable';
    case 'download':
      if (kind === 'terminal.download_success') {
        return row.artifact_claim_result.includes('zero')
          ? 'download.optional_zero'
          : 'download.completed';
      }
      if (kind === 'terminal.download_artifact_failure') {
        if (reason === 'artifact.controls_absent') return 'download.controls_absent_required';
        if (['artifact.download_timeout', 'artifact.recovery_failed'].includes(reason)) {
          return 'download.event_timeout';
        }
        if (reason === 'provider.limit') return 'download.provider_blocked';
        return 'download.ambiguous_controls';
      }
      if (reason === 'session.url_mismatch') return 'download.url_rejected';
      if (['provider.limit', 'provider.schema_drift'].includes(reason)) {
        return 'download.provider_blocked';
      }
      return 'download.content_unavailable';
    case 'status':
      if (kind === 'terminal.status_ready') return 'status.ready';
      if (kind === 'nonterminal.status_unknown' || reason === 'provider.limit') {
        return 'status.degraded';
      }
      if (kind === 'terminal.status_blocked') return 'status.blocked';
      return 'status.runtime_probe_failed';
    default:
      fail(`${row.normalized_leaf_id}: no lifecycle mapping for ${row.current_operation}`);
  }
}

function generate(rows, eventMap, resultMatrix) {
  const generated = rows.map((row) => {
    const receipts = tokens(row.receipt_sequence, `${row.normalized_leaf_id} receipts`);
    const events = translateEvents(row, eventMap);
    const operation = responseOperation(row, receipts);
    const polarity = responsePolarity(row);
    const failureVariant = row.ok !== 'true';
    const failClosed = FAIL_CLOSED_BY_OPERATION[row.current_operation];
    assert(failClosed !== undefined, `${row.normalized_leaf_id}: missing fail-closed mapping`);

    let lifecycleResultKind = resultForRow(row, events);
    let required = failureVariant ? null : requiredProof(row, operation, receipts);
    if (contractClass(row.lifecycle_reason) || (!failureVariant && required === null)) {
      lifecycleResultKind = failClosed;
      required = null;
    }

    const spec = resultMatrix.get(lifecycleResultKind);
    const failSpec = resultMatrix.get(failClosed);
    assert(
      !failSpec.ok && failSpec.terminal && failSpec.exitCode === 70,
      `${row.normalized_leaf_id}: fail-closed result is not a terminal exit-70 failure`,
    );
    return [
      row.normalized_leaf_id,
      `${operation}.${polarity}`,
      required ?? 'none',
      events.length === 0 ? '-' : events.join(','),
      lifecycleResultKind,
      String(spec.exitCode),
      failClosed,
    ];
  });
  generated.sort((left, right) => Buffer.from(left[0]).compare(Buffer.from(right[0])));
  assert(generated.length === 315, 'generated crosswalk: leaf count');
  assert(new Set(generated.map((row) => row[0])).size === 315, 'generated crosswalk: leaf parity');
  for (const row of generated) {
    assert(row.length === HEADER.length, `${row[0]}: output field count`);
    for (const cell of row) {
      assert(cell.length > 0 && !/[\t\n\r]/.test(cell), `${row[0]}: invalid output cell`);
    }
    assert(
      /^[A-Za-z0-9_.,|-]+$/.test(row[3]),
      `${row[0]}: invalid event sequence serialization`,
    );
  }
  return `${HEADER.join('\t')}\n${generated.map((row) => row.join('\t')).join('\n')}\n`;
}

try {
  const options = parseArgs(process.argv.slice(2));
  const rows = parseCatalog(options.catalog);
  const resultMatrix = parseRustResultMatrix(options.resultMatrix);
  const eventMap = parseRustEventMap(options.eventsSource);
  const output = generate(rows, eventMap, resultMatrix);
  fs.writeFileSync(options.output, output, { encoding: 'utf8', mode: 0o644 });
} catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  process.stderr.write(`crosswalk generation failed: ${message}\n`);
  process.exitCode = 1;
}
