export const PROVIDER_SCHEMA = 'gpt-webai.provider.envelope.v2';

const KNOWN_STATUSES = new Set([
  'ready',
  'login_required',
  'provider_limit',
  'subscription_required',
  'unknown',
  'unreachable',
  'sent',
  'session.start_unconfirmed',
  'session.content_unavailable',
  'session.running_unverified',
  'attachment_unavailable',
  'model.selection_mismatch',
  'running',
  'done',
  'artifact.download_timeout',
  'artifact.controls_absent',
  'artifact.recovery_failed',
  'captured',
  'capture_failed',
  'scroll.bottom_unverified',
  'resumed',
  'show',
  'provider.schema_drift',
]);

export function validateProviderEnvelope(envelope) {
  const errors = [];
  if (!envelope || typeof envelope !== 'object' || Array.isArray(envelope)) {
    return ['envelope must be an object'];
  }
  if (envelope.schema !== PROVIDER_SCHEMA) errors.push('schema must be gpt-webai.provider.envelope.v2');
  if (typeof envelope.ok !== 'boolean') errors.push('ok must be boolean');
  if (envelope.vendor !== 'chatgpt') errors.push('vendor must be chatgpt');
  if (typeof envelope.status !== 'string' || envelope.status.length === 0) errors.push('status must be non-empty string');
  else if (!KNOWN_STATUSES.has(envelope.status)) errors.push(`status is unknown: ${envelope.status}`);
  if (envelope.reason !== undefined && typeof envelope.reason !== 'string') errors.push('reason must be string when present');
  if (envelope.artifactExpectation !== undefined
    && !['none', 'optional', 'required', 'claimed'].includes(envelope.artifactExpectation)) {
    errors.push('artifactExpectation must be none, optional, required, or claimed');
  }

  if (envelope.status === 'sent') {
    if (!nonEmptyString(envelope.sessionId)) errors.push('sent requires sessionId');
    if (!validConversationUrl(envelope.conversationUrl)) errors.push('sent requires non-root /c conversationUrl');
  }
  if (envelope.status === 'done' && nonEmptyString(envelope.answerText) && !validConversationUrl(envelope.conversationUrl)) {
    errors.push('done with answerText requires non-root /c conversationUrl');
  }
  if (['artifact.download_timeout', 'artifact.controls_absent', 'artifact.recovery_failed'].includes(envelope.status)
    && !nonEmptyString(envelope.sessionId)) {
    errors.push(`${envelope.status} requires sessionId`);
  }
  if (envelope.status === 'session.content_unavailable' && !nonEmptyString(envelope.sessionId)) {
    errors.push('session.content_unavailable requires sessionId');
  }
  if (['session.running_unverified', 'scroll.bottom_unverified'].includes(envelope.status)
    && !nonEmptyString(envelope.sessionId)) {
    errors.push(`${envelope.status} requires sessionId`);
  }

  validateArtifactArray(envelope.artifacts, 'artifacts', errors);
  validateArtifactArray(envelope.artifactCandidates, 'artifactCandidates', errors);
  return errors;
}

export function validateArtifactObject(item, pathForError = 'artifact') {
  const errors = [];
  if (!item || typeof item !== 'object' || Array.isArray(item)) {
    return [`${pathForError} must be object`];
  }
  if (!nonEmptyString(item.buttonText)) errors.push(`${pathForError}.buttonText must be visible non-empty text`);
  if (!sha256(item.buttonTextSha256)) errors.push(`${pathForError}.buttonTextSha256 must be sha256`);
  if (!item.clickedElement || typeof item.clickedElement !== 'object' || Array.isArray(item.clickedElement)) {
    errors.push(`${pathForError}.clickedElement must be object`);
  }
  if (!item.artifact || typeof item.artifact !== 'object' || Array.isArray(item.artifact)) {
    errors.push(`${pathForError}.artifact must be object`);
  } else if (!['saved', 'failed'].includes(item.artifact.status)) {
    errors.push(`${pathForError}.artifact.status must be saved or failed`);
  }
  return errors;
}

function validateArtifactArray(value, name, errors) {
  if (value === undefined) return;
  if (!Array.isArray(value)) {
    errors.push(`${name} must be array`);
    return;
  }
  value.forEach((item, index) => {
    errors.push(...validateArtifactObject(item, `${name}[${index}]`));
  });
}

function nonEmptyString(value) {
  return typeof value === 'string' && value.length > 0;
}

function validConversationUrl(value) {
  return /^https:\/\/chatgpt\.com\/c\/[^/?#]+/.test(String(value || ''));
}

function sha256(value) {
  return /^[a-f0-9]{64}$/i.test(String(value || ''));
}
