export const PUBLIC_ERROR_KINDS = [
  "needs_login",
  "provider_limit",
  "model_unavailable",
  "send_uncertain",
  "pool_busy",
  "daemon_unreachable",
  "network_disconnected",
  "internal",
] as const;
export type PublicErrorKind = (typeof PUBLIC_ERROR_KINDS)[number];
export const DAEMON_ERROR_KINDS = [
  "needs_login",
  "provider_limit",
  "model_unavailable",
  "nav_failed",
  "compose_failed",
  "chip_mismatch",
  "click_uncertain",
  "turn_not_found",
  "artifact_failed",
  "internal",
] as const;
export type DaemonErrorKind = (typeof DAEMON_ERROR_KINDS)[number];
export type SendPhase = "pre_click" | "post_click";
export class GwpError extends Error {
  readonly kind: PublicErrorKind | DaemonErrorKind;
  readonly phase: SendPhase | undefined;
  readonly detail: string;
  readonly networkEvidence: boolean;
  readonly pendingUserTurnId: string | undefined;
  readonly pendingConversationUrl: string | undefined;
  readonly preClickBaseline: string[] | undefined;
  constructor(
    kind: PublicErrorKind | DaemonErrorKind,
    detail: string,
    options: {
      phase?: SendPhase;
      cause?: unknown;
      networkEvidence?: boolean;
      pendingUserTurnId?: string;
      pendingConversationUrl?: string;
      preClickBaseline?: string[];
    } = {},
  ) {
    super(detail, { cause: options.cause });
    this.name = "GwpError";
    this.kind = kind;
    this.phase = options.phase;
    this.detail = detail;
    this.networkEvidence = options.networkEvidence === true;
    this.pendingUserTurnId = options.pendingUserTurnId;
    this.pendingConversationUrl = options.pendingConversationUrl;
    this.preClickBaseline = options.preClickBaseline;
  }
}
export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
export function isDirectNetworkFailure(error: unknown): boolean {
  const message = errorMessage(error);
  return /ERR_(?:INTERNET_DISCONNECTED|NAME_NOT_RESOLVED|NETWORK_CHANGED)|ENETUNREACH|EHOSTUNREACH|DNS_PROBE_FINISHED_NO_INTERNET/i.test(message);
}
