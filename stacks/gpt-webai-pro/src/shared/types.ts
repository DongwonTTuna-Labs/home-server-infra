export type RequestStatus =
  | "staged"
  | "sending"
  | "generating"
  | "complete"
  | "uncertain"
  | "needs_user_action"
  | "failed";

export type SendAttemptState =
  | "armed"
  | "confirmed"
  | "reconciled"
  | "no_send_proven"
  | "uncertain";

export type SlotState = "idle" | "busy" | "needs_login" | "provider_limit";

export interface RequestRow {
  id: string;
  prompt_sha256: string;
  status: RequestStatus;
  slot_id: string | null;
  conversation_url: string | null;
  answer_sha256: string | null;
  error_kind: string | null;
  error_detail: string | null;
  created_at: number;
  updated_at: number;
}

export interface SendAttemptRow {
  request_id: string;
  attempt_no: number;
  state: SendAttemptState;
  user_turn_id: string | null;
  assistant_turn_id: string | null;
  created_at: number;
  updated_at: number;
}

export interface ArtifactRow {
  request_id: string;
  filename: string;
  path: string;
  sha256: string;
  size_bytes: number;
  created_at: number;
}

export interface SlotRow {
  id: string;
  account: string;
  state: SlotState;
  cooldown_until: number | null;
  last_used_at: number | null;
}

export interface SlotConfig {
  id: string;
  account: string;
  port: number;
  unmanaged?: boolean;
}

export interface SlotsConfig {
  image: string;
  slots: SlotConfig[];
}

export interface LabelConfig {
  target: string[];
  intelligence: string[];
}

export interface RpcFile {
  name: string;
  containerPath: string;
}

export interface HealthResult {
  ok: boolean;
  chromeConnected: boolean;
  currentUrl: string;
}

export interface ReadinessResult {
  state: "ready" | "needs_login" | "provider_limit" | "unknown";
  modelLabel: string;
}

export interface SendParams {
  prompt: string;
  files: RpcFile[];
  newConversation: true;
}

export interface SendResult {
  conversationUrl: string;
  userTurnId: string;
  assistantTurnId: string;
}

export interface ReconcileParams {
  promptSha256: string;
  conversationUrl?: string;
}

export interface ReconcileResult {
  found: boolean;
  conversationUrl?: string;
  userTurnId?: string;
  assistantTurnId?: string;
  proven: boolean;
}

export interface ArtifactControl {
  index: number;
  label: string;
}

export interface PollParams {
  conversationUrl: string;
  promptSha256: string;
  userTurnId?: string;
  assistantTurnId?: string;
  waitMs: number;
}

export interface PollResult {
  state: "generating" | "complete";
  currentUrl: string;
  assistantTurnId?: string;
  answerMarkdown?: string;
  answerSha256?: string;
  artifactControls?: ArtifactControl[];
}

export interface DownloadParams {
  conversationUrl: string;
  controlIndex: number;
}

export interface DownloadResult {
  filename: string;
  outboxPath: string;
  sha256: string;
  sizeBytes: number;
}

export interface OpenParams {
  conversationUrl: string;
}

export interface CaptureFailureParams {
  tag: string;
}

export interface CaptureFailureResult {
  screenshotPath: string;
  htmlPath: string;
}

export interface RpcMethods {
  health: { params: undefined; result: HealthResult };
  readiness: { params: undefined; result: ReadinessResult };
  send: { params: SendParams; result: SendResult };
  reconcile: { params: ReconcileParams; result: ReconcileResult };
  poll: { params: PollParams; result: PollResult };
  download: { params: DownloadParams; result: DownloadResult };
  open: { params: OpenParams; result: { ok: boolean } };
  captureFailure: { params: CaptureFailureParams; result: CaptureFailureResult };
}

export type RpcMethod = keyof RpcMethods;

export interface PublicArtifact {
  filename: string;
  path: string;
  sha256: string;
  sizeBytes: number;
}

export type EnvelopeStatus =
  | "complete"
  | "running"
  | "recovering"
  | "needs_user_action"
  | "failed";

export interface Envelope {
  ok: boolean;
  hardFailure: boolean;
  networkDisconnected: boolean;
  usageError: boolean;
  status: EnvelopeStatus;
  sessionId: string | null;
  resumeCommand: string | null;
  nextCommand: string | null;
  answer: string | null;
  answerPath: string | null;
  answerSha256: string | null;
  artifacts: PublicArtifact[];
  errorKind: string | null;
  message: string | null;
}
