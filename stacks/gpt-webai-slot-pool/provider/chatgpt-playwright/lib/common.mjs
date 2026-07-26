import { createHash } from 'node:crypto';
import { createReadStream, writeSync } from 'node:fs';
import { stat } from 'node:fs/promises';
import path from 'node:path';

import { PROVIDER_SCHEMA, validateProviderEnvelope } from './schemas.mjs';

export { PROVIDER_SCHEMA };
export const CHATGPT_ROOT = 'https://chatgpt.com/';
export const DEFAULT_DOWNLOAD_TIMEOUT_MS = 30_000;
export const DEFAULT_ATTACHMENT_TIMEOUT_MS = 60_000;
export const DEFAULT_STABLE_MS = 2_500;
export const DEFAULT_SESSION_HYDRATION_TIMEOUT_MS = 60_000;

export function hitFailpoint(name) {
  if (process.env.GPT_WEBAI_FAILPOINT !== name) return;
  // Synchronous fd write is required because this path exits without cleanup.
  writeSync(2, `failpoint:${name}\n`);
  process.exit(99);
}

export function jsonOut(payload) {
  const envelope = { schema: PROVIDER_SCHEMA, ...payload };
  const validationErrors = validateProviderEnvelope(envelope);
  if (validationErrors.length === 0) {
    process.stdout.write(`${JSON.stringify(envelope)}\n`);
    return;
  }
  const drift = {
    schema: PROVIDER_SCHEMA,
    ok: true,
    vendor: envelope.vendor || 'chatgpt',
    status: 'provider.schema_drift',
    reason: 'provider.schema_drift',
    message: 'provider envelope failed self-validation',
    validationErrors,
  };
  process.stdout.write(`${JSON.stringify(drift)}\n`);
}

export function sha256Text(text) {
  return createHash('sha256').update(text).digest('hex');
}

export async function sha256File(filePath) {
  const hash = createHash('sha256');
  await new Promise((resolve, reject) => {
    createReadStream(filePath).on('data', chunk => hash.update(chunk)).on('end', resolve).on('error', reject);
  });
  return hash.digest('hex');
}

export async function fileSize(filePath) {
  const info = await stat(filePath);
  return info.size;
}

export function valueAfter(args, name) {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] || '' : '';
}

export function valuesAfter(args, name) {
  const values = [];
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === name && args[index + 1]) {
      values.push(args[index + 1]);
      index += 1;
    }
  }
  return values;
}

export function numberArg(args, name, fallback) {
  const raw = valueAfter(args, name);
  const value = Number.parseInt(raw, 10);
  return Number.isFinite(value) && value > 0 ? value : fallback;
}

// ChatGPT briefly shows an optimistic client-side placeholder conversation id
// ("WEB:<uuid>") before the server assigns the real one. Treat it as not yet a
// valid conversation: confirmation must wait for the server id.
function isPlaceholderConversationId(id) {
  return /^WEB(:|%3A)/i.test(id || '');
}

export function conversationIdFromUrl(url) {
  const match = /^https:\/\/chatgpt\.com\/c\/([^/?#]+)/.exec(url || '');
  if (!match) return '';
  return isPlaceholderConversationId(match[1]) ? '' : match[1];
}

export function validConversationUrl(url) {
  return conversationIdFromUrl(url) !== '';
}

export function conversationUrlMatchesSession(url, sessionId) {
  return conversationIdFromUrl(url) === String(sessionId || '');
}

export function sanitizeFilename(name, fallback) {
  const base = path.basename(String(name || fallback || 'artifact.bin')).replace(/[^A-Za-z0-9._-]+/g, '_');
  const trimmed = base.replace(/^_+|_+$/g, '');
  if (!trimmed || trimmed === '.' || trimmed === '..') return 'artifact.bin';
  return trimmed.slice(0, 160);
}

const ARTIFACT_FILENAME_RE = /[A-Za-z0-9][A-Za-z0-9._-]{0,180}\.(?:tar\.gz|zip|tgz|gz|diff|patch|txt|md|manifest|sha256|json|csv)\b/gi;

export function filenameFromText(text) {
  ARTIFACT_FILENAME_RE.lastIndex = 0;
  const match = String(text || '').match(ARTIFACT_FILENAME_RE);
  return match ? match[0].trim() : '';
}

function artifactSignalText(text) {
  const lines = String(text || '').split(/\r?\n/);
  const kept = [];
  let fenced = false;
  for (const line of lines) {
    if (/^\s*```/.test(line)) {
      fenced = !fenced;
      continue;
    }
    if (fenced) continue;
    if (/^\s*diff --git\b/i.test(line)) break;
    kept.push(line);
  }
  return kept.join('\n');
}

export function artifactFilenamesFromText(text) {
  const signalText = artifactSignalText(text);
  ARTIFACT_FILENAME_RE.lastIndex = 0;
  return Array.from(new Set(
    Array.from(signalText.matchAll(ARTIFACT_FILENAME_RE), match => match[0].trim())
      .filter(Boolean),
  ));
}

export function artifactExpectedFromText(text) {
  const signalText = artifactSignalText(text);
  const artifactReady = /\bARTIFACT_(?:READY|DONE)\b/i.test(signalText);
  if (!artifactReady
    && terminalVerdictMarkerAnswer(signalText)
    && artifactFilenamesFromText(signalText).length === 0
    && /\b(?:no|not)\b.{0,80}\b(?:downloadable|artifact|download controls?|downloadable files?)\b/i.test(signalText)) {
    return false;
  }
  return artifactReady
    || /(?:download|다운로드)\s*:?\s+[^\n]+\.(?:tar\.gz|zip|tgz|gz|diff|patch|txt|md|manifest|sha256|json|csv)\b|downloadable (?:artifact|archive|file|zip|tar)|download controls?|(?:artifact(?:\s+filename)?|manifest)\s*:?\s*[^\n]+\.(?:tar\.gz|zip|tgz|gz|diff|patch|txt|md|manifest|sha256|json|csv)\b|artifact filename\(s\)|zip or tar artifact|source-tree artifact|\.zip\.sha256\b/i.test(signalText)
    || patchOnlyTerminalAnswer(signalText);
}

export function normalizeArtifactExpectation(value, fallback = 'optional') {
  const normalized = String(value || fallback || 'optional').trim().toLowerCase();
  return ['none', 'optional', 'required', 'claimed'].includes(normalized) ? normalized : fallback;
}

export function artifactExpectationFromArgs(args, fallback = 'optional') {
  return normalizeArtifactExpectation(valueAfter(args, '--artifact-expectation'), fallback);
}

export function artifactExpectationRequiresControls(expectation, answerText = '') {
  const normalized = normalizeArtifactExpectation(expectation, 'optional');
  if (normalized === 'required' || normalized === 'claimed') return true;
  if (normalized === 'none') return false;
  return artifactExpectedFromText(answerText);
}

export function artifactDownloadFailureStatus({
  artifactExpected = false,
  artifacts = [],
  warnings = [],
  downloadCandidateCount = 0,
} = {}) {
  const warningCount = Array.isArray(warnings) ? warnings.length : 0;
  if (warningCount > 0) return 'artifact.download_timeout';

  if (Array.isArray(artifacts) && artifacts.length > 0) return '';

  const candidateCount = Number(downloadCandidateCount || 0);
  if (Number.isFinite(candidateCount) && candidateCount > 0) return 'artifact.download_timeout';
  if (artifactExpected) return 'artifact.controls_absent';
  return '';
}

export function terminalVerdictMarkerAnswer(text) {
  const value = String(text || '').trim();
  if (!value) return false;
  const verdict = '(?:ARTIFACT_READY|PATCH_READY|DESIGN_READY|CHANGES_REQUIRED|LGTM_NO_BLOCKING|LGTM|ATTACHMENT_MISSING)';
  const lines = value
    .split(/\r?\n/)
    .map(line => line.trim().replace(/^`+|`+$/g, ''))
    .filter(Boolean)
    .slice(0, 5);
  return lines.some(line => (
    new RegExp(`^${verdict}(?:\\s|$|[:.;,\\[(\\-])`, 'i').test(line)
    || new RegExp(`^(?:VERDICT|Required Verdict|Final Verdict)\\s*:\\s*${verdict}\\b`, 'i').test(line)
    || /^BLOCKING_FINDINGS\s*:/i.test(line)
  ));
}

function patchOnlyTerminalAnswer(text) {
  const value = String(text || '').trim();
  if (!value) return false;
  return /^\s*(?:VERDICT\s*:\s*)?PATCH_READY\b/i.test(value)
    || /^\s*diff --git\b/im.test(value)
    || /\b(?:unified diff|body patch|inline patch|apply-ready diff|patch file)\b/i.test(value);
}

export function progressPrologueAnswer(text) {
  const value = String(text || '').trim();
  if (!value) return false;
  if (terminalVerdictMarkerAnswer(value)) {
    return false;
  }
  if (/^(?:pro\s+)?thinking[. …]*$/i.test(value) || /^(?:프로\s*)?생각\s*중[. …]*$/i.test(value)) {
    return true;
  }
  return [
    /\bI(?:['’]m| am)\s+(?:(?:specifically|now|currently|also|still)\s+)?(?:focusing|reviewing|checking|inspecting|reading|analy[sz]ing|working|validating|verifying|testing|re-?running|cross-checking)\b/i,
    /\bI(?:['’]m| am)\s+(?:(?:specifically|now|currently|also|still)\s+)?(?:in|doing|performing)\s+.{0,120}\b(?:review|analysis|inspection|verification|validation|testing|check|checks)\b/i,
    /\bI(?:['’]ve| have)\s+(?:found|verified|validated|confirmed|checked|read|opened|unpacked)\b.{0,240}\band\s+(?:am\s+)?(?:checking|reviewing|inspecting|validating|verifying|testing|cross-checking|analy[sz]ing|working)\b/is,
    /\bI(?:['’]ll| will)\s+(?:(?:first|now|next)\s+)?(?:inspect|verify|review|check|read|look|analy[sz]e|start|build|locate|open|examine)\b/i,
    /\bI(?:['’]ll| will)\s+.*\bthen\s+(?:provide|build|write|return|produce|decide)\b/i,
    /\b(?:let me|I am going to)\s+(?:inspect|verify|review|check|read|look|analy[sz]e|start|build|locate|open|examine)\b/i,
  ].some(pattern => pattern.test(value));
}

export function mimeFromName(name) {
  const lower = name.toLowerCase();
  if (lower.endsWith('.zip')) return 'application/zip';
  if (lower.endsWith('.tar.gz') || lower.endsWith('.tgz') || lower.endsWith('.gz')) return 'application/gzip';
  if (lower.endsWith('.json')) return 'application/json';
  if (lower.endsWith('.manifest') || lower.endsWith('.md') || lower.endsWith('.txt') || lower.endsWith('.diff') || lower.endsWith('.patch') || lower.endsWith('.sha256')) return 'text/plain';
  return 'application/octet-stream';
}
