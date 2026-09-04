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
export type SlotState = "idle" | "needs_login" | "provider_limit";
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
  // 계정별 주간(7일 이동창) 전송 한도. 없으면 SlotsConfig.weeklyLimit, 그것도 없으면 무제한.
  weeklyLimit?: number;
}
export interface SlotsConfig {
  image: string;
  maxConcurrent: number;
  slots: SlotConfig[];
  // 모든 슬롯의 기본 주간 전송 한도 (ChatGPT Pro 계정 주간 200회 등).
  weeklyLimit?: number;
}
export interface UsageEventRow {
  request_id: string;
  slot_id: string;
  model_label: string | null;
  sent_at: number;
}
export interface LabelConfig {
  // 허용되는 생각 강도(power) 라벨. 알약(pill)이 이 중 하나면 already_exact.
  target: string[];
  // 알약 후보 식별에 쓰는 power 라벨 전집합 (버전 토큰 "6", "5.5"는 정규화에서 제거된다).
  intelligence: string[];
  // 새 UI(2026-09 GPT-6)의 "Select model" 라디오에서 선택할 모델 버전. 없으면 검사하지 않는다.
  modelVersion?: string;
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
  // 지정 시 새 채팅 대신 이 기존 대화에 이어서 후속 턴을 보낸다 (RouteFork 연속 제안용).
  conversationUrl?: string;
}
export type SendStep =
  | "navigate"
  | "ensure_model"
  | "compose"
  | "attach"
  | "verify_chips"
  | "baseline"
  | "wait_send_button"
  | "click"
  | "confirm";
export interface SendProgress {
  step: SendStep;
  phase: import("./errors.js").SendPhase;
  elapsedMs: number;
  stepElapsedMs: number;
  pendingUserTurnId?: string;
  pendingConversationUrl?: string;
  preClickBaseline?: string[];
  matchDebug?: string;
}
export const SEND_PROGRESS_METHOD = "gwp.sendProgress";
export interface SendProgressNotification {
  jsonrpc: "2.0";
  method: typeof SEND_PROGRESS_METHOD;
  params: { callId: number; progress: SendProgress };
}
export interface SendResult {
  conversationUrl: string;
  userTurnId: string;
  // 전송 직전 보장된 모델 라벨(예: "6 Pro"). 주간 사용량 기록의 증거로 supervisor가 저장한다.
  modelLabel?: string;
  // assistant 턴은 전송 착지 확정에 필수가 아니다 — user 턴 + 비루트 대화 URL이면 착지로 본다.
  // (reconcile의 turn_anchor와 동일 기준; 생성 완료는 poll이 별도로 판정한다.)
  assistantTurnId?: string;
  matchedBy?: "strict" | "loose" | "single_turn";
}
export interface ReconcileParams {
  prompt: string;
  promptSha256: string;
  conversationUrl?: string;
  pendingConversationUrl?: string;
  pendingUserTurnId?: string;
  preClickBaseline?: string[];
}
export interface ReconcileResult {
  found: boolean;
  conversationUrl?: string;
  userTurnId?: string;
  assistantTurnId?: string;
  proven: boolean;
  matchedBy?: "cache" | "turn_anchor" | "strict" | "loose" | "single_turn";
  evidence?: string;
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
export interface CloseConversationParams {
  conversationUrl: string;
}
export interface RpcMethods {
  health: { params: undefined; result: HealthResult };
  readiness: { params: undefined; result: ReadinessResult };
  send: { params: SendParams; result: SendResult };
  reconcile: { params: ReconcileParams; result: ReconcileResult };
  poll: { params: PollParams; result: PollResult };
  download: { params: DownloadParams; result: DownloadResult };
  closeConversation: { params: CloseConversationParams; result: { ok: boolean } };
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
