import type { Envelope, EnvelopeStatus, PublicArtifact } from "../shared/types.js";

export function makeEnvelope(
  status: EnvelopeStatus,
  values: Partial<Envelope> = {},
): Envelope {
  return {
    ok: true,
    hardFailure: false,
    networkDisconnected: false,
    usageError: false,
    status,
    sessionId: null,
    resumeCommand: null,
    nextCommand: null,
    answer: null,
    answerPath: null,
    answerSha256: null,
    artifacts: [],
    errorKind: null,
    message: null,
    ...values,
  };
}

export function resumeCommand(sessionId: string): string {
  return `gpt-webai-pro resume --session ${sessionId}`;
}

export function runningEnvelope(sessionId: string, message: string | null = null): Envelope {
  return makeEnvelope("running", {
    sessionId,
    resumeCommand: resumeCommand(sessionId),
    message,
  });
}

export function recoveringEnvelope(
  sessionId: string,
  errorKind: "provider_limit" | "pool_busy",
  message: string,
): Envelope {
  const command = resumeCommand(sessionId);
  return makeEnvelope("recovering", {
    sessionId,
    resumeCommand: command,
    nextCommand: command,
    errorKind,
    message,
  });
}

export function actionEnvelope(
  sessionId: string | null,
  errorKind: string | null,
  message: string,
  resumable = false,
): Envelope {
  return makeEnvelope("needs_user_action", {
    sessionId,
    resumeCommand: resumable && sessionId ? resumeCommand(sessionId) : null,
    errorKind,
    message,
  });
}

export function failedEnvelope(
  sessionId: string | null,
  errorKind: string,
  message: string,
): Envelope {
  return makeEnvelope("failed", { sessionId, errorKind, message });
}

export function networkFailureEnvelope(
  sessionId: string | null,
  message: string,
): Envelope {
  return makeEnvelope("failed", {
    ok: false,
    hardFailure: true,
    networkDisconnected: true,
    sessionId,
    errorKind: "network_disconnected",
    message,
  });
}

export function completeEnvelope(values: {
  sessionId: string;
  answer: string;
  answerPath: string;
  answerSha256: string;
  artifacts?: PublicArtifact[];
  message?: string | null;
}): Envelope {
  return makeEnvelope("complete", {
    sessionId: values.sessionId,
    answer: values.answer,
    answerPath: values.answerPath,
    answerSha256: values.answerSha256,
    artifacts: values.artifacts ?? [],
    message: values.message ?? null,
  });
}

export function emptyPromptEnvelope(): Envelope {
  return makeEnvelope("needs_user_action", {
    usageError: true,
    message: "prompt must not be empty",
  });
}

export function writeEnvelope(envelope: Envelope): void {
  process.stdout.write(`${JSON.stringify(envelope)}\n`);
}
