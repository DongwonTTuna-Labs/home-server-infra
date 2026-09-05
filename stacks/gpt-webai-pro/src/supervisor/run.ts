import { access, copyFile, readFile, readdir, rm, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { actionEnvelope, completeEnvelope, failedEnvelope, networkFailureEnvelope, recoveringEnvelope, runningEnvelope } from "../cli/envelope.js";
import { GwpError, errorMessage, isDirectNetworkFailure, type PublicErrorKind } from "../shared/errors.js";
import {
  acquireFileLock,
  appendJsonLine,
  atomicWrite,
  fileSize,
  mkdirp,
  moveFile,
  sha256File,
  sha256Text,
  tryAcquireFileLock,
  type FileLock,
} from "../shared/fsx.js";
import { newRequestId } from "../shared/ids.js";
import type { Envelope, PollResult, PublicArtifact, ReadinessResult, ReconcileResult, RequestRow, RpcFile, SendProgress, SendResult, SlotConfig, SlotState, SlotsConfig } from "../shared/types.js";
import { GwpDatabase } from "./db.js";
import { DockerManager, mapContainerOutboxPath } from "./docker.js";
import { RpcClient } from "./rpc-client.js";
import {
  claimSlotForRequest,
  markSlotIdle,
  markSlotNeedsLogin,
  markSlotProviderLimit,
  resolveWeeklyLimits,
  weeklyUsageFor,
} from "./slots.js";
const ACTIVE_STATUSES = new Set(["staged", "sending", "generating", "uncertain"]);
const TERMINAL_STATUSES = new Set(["complete", "needs_user_action", "failed"]);
const SEND_IN_PROGRESS_MESSAGE = "전송 진행 중(소유 프로세스 생존)";
// send RPC는 고정 타임아웃이 아니라 progress 기반으로 기다린다: daemon이 알림으로
// 살아있음을 증명하는 한 절대 상한까지 완주를 기다린다. 94KB 프롬프트가 페이지 jank로
// 180s를 넘겨 앵커가 통째로 유실된 2026-07-29 사건이 근거다.
const SEND_RPC_MAX_MS = envMs("GWP_SEND_MAX_MS", 30 * 60_000);
const SEND_RPC_INACTIVITY_MS = envMs("GWP_SEND_INACTIVITY_MS", 120_000);
function envMs(name: string, fallback: number): number {
  const value = Number(process.env[name]);
  return Number.isFinite(value) && value > 0 ? value : fallback;
}
export async function waitForOwnerLock<Lock>(options: {
  tryLock: () => Promise<Lock | null>;
  isTerminal: () => boolean;
  deadline: number;
  pollMs: number;
  now?: () => number;
  sleep?: (milliseconds: number) => Promise<void>;
}): Promise<{ kind: "acquired"; lock: Lock } | { kind: "terminal" } | { kind: "timeout" }> {
  const now = options.now ?? Date.now;
  const sleep = options.sleep ?? ((milliseconds: number) => new Promise<void>((resolve) => setTimeout(resolve, milliseconds)));
  let firstAttempt = true;
  while (firstAttempt || now() < options.deadline) {
    firstAttempt = false;
    const lock = await options.tryLock();
    if (lock) return { kind: "acquired", lock };
    if (options.isTerminal()) return { kind: "terminal" };
    const remaining = options.deadline - now();
    if (remaining <= 0) break;
    await sleep(Math.min(options.pollMs, remaining));
  }
  return { kind: options.isTerminal() ? "terminal" : "timeout" };
}
interface ValidatedFile {
  source: string;
  name: string;
}
export class InputError extends Error {
  override readonly name = "InputError";
}
export class LoginTimeoutError extends Error {
  override readonly name = "LoginTimeoutError";
}
export class LoginInterruptedError extends Error {
  override readonly name = "LoginInterruptedError";
}
export type KeepaliveProbe = ReadinessResult["state"] | "unreachable";
export interface LoginOptions {
  timeoutMs?: number;
  pollIntervalMs?: number;
  signal?: AbortSignal;
  onUrl?: (url: string) => void;
  onProgress?: (elapsedMs: number, state: ReadinessResult["state"]) => void;
}
export class Supervisor {
  readonly db: GwpDatabase;
  readonly docker: DockerManager;
  private constructor(
    readonly stateDir: string,
    readonly config: SlotsConfig,
    db: GwpDatabase,
  ) {
    this.db = db;
    this.docker = new DockerManager(stateDir, config.image);
  }
  static async open(options: {
    stateDir?: string;
    configPath?: string;
  } = {}): Promise<Supervisor> {
    const stateDir = options.stateDir ?? defaultStateDir();
    const configPath = options.configPath ?? defaultConfigPath();
    const config = validateConfig(JSON.parse(await readFile(configPath, "utf8")) as SlotsConfig);
    await mkdirp(stateDir);
    const db = await GwpDatabase.open(path.join(stateDir, "db.sqlite"));
    db.syncSlots(config.slots);
    return new Supervisor(stateDir, config, db);
  }
  close(): void {
    this.db.close();
  }
  async run(prompt: string, files: string[], timeoutSeconds: number, conversationUrl?: string): Promise<Envelope> {
    if (!prompt.trim()) throw new InputError("prompt must not be empty");
    const validated = await validateFiles(files);
    const id = newRequestId();
    const directory = this.requestDir(id);
    await mkdirp(directory);
    this.db.createRequest(id, sha256Text(prompt.trim()));
    this.log(id, "staged");
    try {
      await atomicWrite(path.join(directory, "prompt.md"), prompt);
      if (conversationUrl) await atomicWrite(path.join(directory, "continue_url"), conversationUrl);
      await this.persistAttachments(id, validated);
    } catch (error) {
      this.db.setRequestStatus(id, "failed", "internal", errorMessage(error));
      return failedEnvelope(id, "internal", errorMessage(error));
    }
    return this.continue(id, timeoutSeconds);
  }
  async resume(id: string, timeoutSeconds: number): Promise<Envelope> {
    if (!this.db.getRequest(id)) throw new InputError(`unknown session: ${id}`);
    return this.continue(id, timeoutSeconds);
  }
  async status(): Promise<{
    ok: true;
    slots: Array<{
      id: string;
      account: string;
      state: string;
      cooldownUntil: number | null;
      lastUsedAt: number | null;
      activeRequests: number;
      weeklyUsed: number;
      weeklyLimit: number | null;
      weeklyResetAt: number | null;
    }>;
    requests: Array<{
      id: string;
      status: string;
      slotId: string | null;
      ageSeconds: number;
      conversationUrl: string | null;
    }>;
  }> {
    const now = Date.now();
    const limits = resolveWeeklyLimits(this.config);
    return {
      ok: true,
      slots: this.db.listSlots().map((slot) => ({
        id: slot.id,
        account: slot.account,
        state: slot.state,
        cooldownUntil: slot.cooldown_until,
        lastUsedAt: slot.last_used_at,
        activeRequests: this.db.countActiveForSlot(slot.id),
        ...weeklyStatus(weeklyUsageFor(this.db, slot.id, limits.get(slot.id) ?? null, now)),
      })),
      requests: this.db.listNonterminalRequests().map((request) => ({
        id: request.id,
        status: request.status,
        slotId: request.slot_id,
        ageSeconds: Math.floor(Math.max(0, now - request.created_at) / 1_000),
        conversationUrl: request.conversation_url,
      })),
    };
  }
  async keepalive(): Promise<{
    ok: true;
    slots: Array<{ id: string; state: SlotState; probe: KeepaliveProbe }>;
  }> {
    const slots: Array<{ id: string; state: SlotState; probe: KeepaliveProbe }> = [];
    for (const slot of this.config.slots) {
      const current = this.db.getSlot(slot.id);
      if (!current) throw new Error(`slot ${slot.id} is absent from the database`);
      if (this.db.countActiveForSlot(slot.id) > 0) {
        slots.push({ id: slot.id, state: current.state, probe: "unknown" });
        continue;
      }
      const activityLock = await tryAcquireFileLock(this.slotActivityLockPath(slot.id));
      if (!activityLock) {
        slots.push({ id: slot.id, state: current.state, probe: "unknown" });
        continue;
      }
      let wasRunning: boolean | null = null;
      let client: RpcClient | null = null;
      let probe: KeepaliveProbe = "unknown";
      try {
        if (slot.unmanaged !== true) wasRunning = (await this.docker.inspect(slot.id)).running;
        client = await this.connectDaemon(slot);
        const readiness = await client.call("readiness", undefined, 40_000);
        probe = readiness.state;
        if (readiness.state === "ready") markSlotIdle(this.db, slot.id);
        else if (readiness.state === "needs_login") markSlotNeedsLogin(this.db, slot.id);
        else if (readiness.state === "provider_limit") markSlotProviderLimit(this.db, slot.id);
      } catch {
        probe = "unreachable";
      } finally {
        if (client) await client.close().catch(() => undefined);
        await activityLock.release();
        // Only stop when inspect proved that this invocation started the runtime.
        // An inspect failure leaves ownership unknown and must preserve the runtime.
        if (slot.unmanaged !== true && wasRunning === false) {
          await this.stopSlotIfNoLiveOwners(slot, undefined, true).catch(() => undefined);
        }
      }
      slots.push({ id: slot.id, state: this.db.getSlot(slot.id)!.state, probe });
    }
    return { ok: true, slots };
  }
  async login(
    slotId: string,
    options: LoginOptions = {},
  ): Promise<{ slotId: string; state: "ready"; url: string }> {
    const slot = this.config.slots.find((item) => item.id === slotId);
    if (!slot) throw new InputError(`unknown slot: ${slotId}`);
    if (this.db.countActiveForSlot(slot.id) > 0) {
      throw new InputError(`slot ${slot.id} has active requests`);
    }
    const timeoutMs = options.timeoutMs ?? 15 * 60 * 1_000;
    const pollIntervalMs = options.pollIntervalMs ?? 5_000;
    if (!Number.isFinite(timeoutMs) || timeoutMs < 0
      || !Number.isFinite(pollIntervalMs) || pollIntervalMs <= 0) {
      throw new InputError("login polling intervals are invalid");
    }
    const activityLock = await tryAcquireFileLock(this.slotActivityLockPath(slot.id));
    if (!activityLock) throw new InputError(`slot ${slot.id} is already in use`);
    if (this.db.countActiveForSlot(slot.id) > 0) {
      await activityLock.release();
      throw new InputError(`slot ${slot.id} has active requests`);
    }
    // Prevent a new request from claiming this slot while its container is being recreated in
    // interactive login mode. Success returns it to idle; every other outcome stays fail-closed.
    markSlotNeedsLogin(this.db, slot.id);
    const url = `http://127.0.0.1:${slot.port + 600}/vnc.html`;
    let client: RpcClient | null = null;
    let readyObserved = false;
    let timedOut = false;
    try {
      throwIfLoginInterrupted(options.signal);
      const endpoint = await this.withSlotControlLock(
        slot,
        () => this.docker.ensure(slot, 60_000, { loginMode: true }),
      );
      throwIfLoginInterrupted(options.signal);
      options.onUrl?.(url);
      client = await RpcClient.connect(endpoint.port, endpoint.tokenPath);
      const startedAt = Date.now();
      const deadline = Date.now() + timeoutMs;
      for (;;) {
        throwIfLoginInterrupted(options.signal);
        const remaining = deadline - Date.now();
        if (remaining <= 0) throw new LoginTimeoutError(`login timed out for ${slot.id}`);
        let readiness: ReadinessResult;
        try {
          readiness = await withLoginAbort(
            client.call("readiness", undefined, Math.max(1, Math.min(40_000, remaining))),
            options.signal,
          );
        } catch (error) {
          if (error instanceof LoginInterruptedError) throw error;
          if (Date.now() >= deadline) throw new LoginTimeoutError(`login timed out for ${slot.id}`);
          throw error;
        }
        if (readiness.state === "ready") {
          readyObserved = true;
          return { slotId: slot.id, state: "ready", url };
        }
        options.onProgress?.(Date.now() - startedAt, readiness.state);
        const waitMs = Math.min(pollIntervalMs, Math.max(0, deadline - Date.now()));
        if (waitMs === 0) throw new LoginTimeoutError(`login timed out for ${slot.id}`);
        await loginDelay(waitMs, options.signal);
      }
    } catch (error) {
      if (error instanceof LoginTimeoutError) timedOut = true;
      throw error;
    } finally {
      try {
        if (client) await client.close().catch(() => undefined);
        await this.stopSlot(slot);
        if (readyObserved) markSlotIdle(this.db, slot.id);
        else if (timedOut) markSlotNeedsLogin(this.db, slot.id);
      } finally {
        await activityLock.release();
      }
    }
  }
  async cleanup(apply: boolean): Promise<{
    ok: true;
    dryRun: boolean;
    actions: Array<{ kind: string; target: string; detail: string }>;
  }> {
    const actions: Array<{ kind: string; target: string; detail: string }> = [];
    for (const row of this.db.listSlots().filter((slot) => slot.state === "needs_login")) {
      const slot = this.requireSlotConfig(row.id);
      if (this.db.countActiveForSlot(row.id) > 0) {
        actions.push({
          kind: "skip_login_recheck",
          target: row.id,
          detail: "slot has active requests",
        });
        continue;
      }
      if (!apply) {
        actions.push({
          kind: "recheck_login",
          target: row.id,
          detail: "would start the slot and recheck daemon readiness",
        });
        continue;
      }
      const activityLock = await tryAcquireFileLock(this.slotActivityLockPath(slot.id));
      if (!activityLock) {
        actions.push({
          kind: "skip_login_recheck",
          target: row.id,
          detail: "slot has a live maintenance owner",
        });
        continue;
      }
      let client: RpcClient | null = null;
      let readinessReady = false;
      let runtimeStopped = false;
      try {
        client = await this.connectDaemon(slot);
        const readiness = await client.call("readiness", undefined, 40_000);
        if (readiness.state === "ready") {
          readinessReady = true;
        } else {
          actions.push({
            kind: "login_still_required",
            target: row.id,
            detail: `readiness remained ${readiness.state}`,
          });
        }
      } catch (error) {
        actions.push({ kind: "login_recheck_failed", target: row.id, detail: errorMessage(error) });
      } finally {
        if (client) await client.close().catch(() => undefined);
        try {
          await this.stopSlot(slot);
          runtimeStopped = true;
        } catch (error) {
          actions.push({
            kind: "login_recheck_failed",
            target: row.id,
            detail: `runtime stop failed: ${errorMessage(error)}`,
          });
        } finally {
          await activityLock.release();
        }
      }
      if (readinessReady && runtimeStopped) {
        markSlotIdle(this.db, row.id);
        actions.push({ kind: "recover_login_slot", target: row.id, detail: "readiness is ready; set idle" });
      }
    }
    for (const slot of this.config.slots) {
      if (slot.unmanaged === true) continue;
      const stopped = await this.stopSlotIfNoLiveOwners(slot, undefined, apply);
      if (stopped === "stopped" || stopped === "would_stop") {
        actions.push({
          kind: "stop_ownerless_container",
          target: this.docker.containerName(slot.id),
          detail: apply
            ? "stopped managed runtime with no live CLI owner"
            : "would stop managed runtime with no live CLI owner",
        });
      }
    }
    return { ok: true, dryRun: !apply, actions };
  }
  // 비종결 요청은 supervisor 프로세스가 돌 때만 상태가 전진한다. 소유 세션이 resume을
  // 다시 돌리지 않으면 generating이 무한 방치된다(2026-07-30 R6 13시간 사례). reap은
  // 이미-전송된 요청(sending/generating/uncertain)을 resume으로 전진시키는 시스템
  // 안전망이다 — 전체 요청 owner.lock과 guarded update가 동시 실행을 보호하므로 살아 있는
  // 소유자와 경합해도 안전하다(소유자 생존 시 running envelope로 비켜난다).
  // staged는 전송이 한 번도 arm되지 않은 요청이라 개시 결정이 소유 세션 몫 — 건드리지
  // 않는다 (reaper의 역할은 "보낸 것을 끝내기"이지 "안 보낸 것을 보내기"가 아니다).
  async reap(timeoutSeconds: number): Promise<{
    ok: true;
    actions: Array<{ session: string; before: string; after: string; detail?: string }>;
    runtimeActions: Array<{ slot: string; action: "stopped_ownerless_runtime" }>;
  }> {
    if (!Number.isFinite(timeoutSeconds) || timeoutSeconds < 0) {
      throw new InputError("timeout-seconds must be a non-negative number");
    }
    const actions: Array<{ session: string; before: string; after: string; detail?: string }> = [];
    const runtimeActions: Array<{ slot: string; action: "stopped_ownerless_runtime" }> = [];
    // One candidate per invocation is the global execution budget. updated_at is touched after
    // each attempt, so repeated timer ticks round-robin instead of pinning the oldest request.
    for (const request of this.db.listReapCandidates(1)) {
      const before = request.status;
      try {
        const envelope = await this.resume(request.id, timeoutSeconds);
        actions.push({
          session: request.id,
          before,
          after: envelope.status,
          ...(envelope.message ? { detail: envelope.message } : {}),
        });
      } catch (error) {
        actions.push({ session: request.id, before, after: "error", detail: errorMessage(error) });
      } finally {
        this.db.touchNonterminalRequest(request.id);
      }
    }
    for (const slot of this.config.slots) {
      if (await this.stopSlotIfNoLiveOwners(slot, undefined, true) === "stopped") {
        runtimeActions.push({ slot: slot.id, action: "stopped_ownerless_runtime" });
      }
    }
    return { ok: true, actions, runtimeActions };
  }
  async release(id: string): Promise<Envelope> {
    if (!this.db.getRequest(id)) throw new InputError(`unknown session: ${id}`);
    const ownerLock = await tryAcquireFileLock(this.ownerLockPath(id));
    if (!ownerLock) return runningEnvelope(id, SEND_IN_PROGRESS_MESSAGE);
    try {
      const sendLock = await tryAcquireFileLock(this.sendLockPath(id));
      if (!sendLock) return runningEnvelope(id, SEND_IN_PROGRESS_MESSAGE);
      try {
        this.db.setRequestStatus(id, "failed", "internal", "released by operator");
        this.log(id, "failed", "released by operator");
        await this.releaseRuntime(this.db.getRequest(id)!);
        return failedEnvelope(id, "internal", "released by operator");
      } finally {
        await sendLock.release();
      }
    } finally {
      await ownerLock.release();
    }
  }
  private async continue(
    id: string,
    timeoutSeconds: number,
  ): Promise<Envelope> {
    if (!Number.isFinite(timeoutSeconds) || timeoutSeconds < 0) {
      throw new InputError("timeout-seconds must be a non-negative number");
    }
    const initial = this.requireRequest(id);
    if (TERMINAL_STATUSES.has(initial.status)) return this.envelopeFor(initial);
    const deadline = Date.now() + Math.floor(timeoutSeconds * 1_000);
    const attachment = await waitForOwnerLock({
      tryLock: () => tryAcquireFileLock(this.ownerLockPath(id)),
      isTerminal: () => TERMINAL_STATUSES.has(this.requireRequest(id).status),
      deadline,
      pollMs: envMs("GWP_OWNER_ATTACH_POLL_MS", 2_000),
    });
    if (attachment.kind === "terminal") return this.envelopeFor(this.requireRequest(id));
    if (attachment.kind === "timeout") return runningEnvelope(id, SEND_IN_PROGRESS_MESSAGE);
    const ownerLock = attachment.lock;
    try {
      return await this.continueOwned(id, deadline);
    } finally {
      const current = this.db.getRequest(id);
      if (current?.slot_id) {
        await this.stopSlotIfNoLiveOwners(
          this.requireSlotConfig(current.slot_id),
          current.id,
          true,
        ).catch(() => undefined);
      }
      await ownerLock.release();
    }
  }
  private async continueOwned(
    id: string,
    deadline: number,
  ): Promise<Envelope> {
    const triedSlots = new Set<string>();
    // GWP_ONLY_SLOT: 특정 슬롯에만 고정한다 (그 슬롯 외 전부 후보에서 제외).
    // 그 슬롯이 사용 불가면 다른 슬롯으로 넘어가지 않고 pool_busy/recovering으로 대기한다.
    const onlySlot = process.env.GWP_ONLY_SLOT?.trim();
    if (onlySlot) {
      for (const s of this.config.slots) if (s.id !== onlySlot) triedSlots.add(s.id);
    }
    let sawLogin = false;
    let sawProviderLimit = false;
    let inlineReconcileTries = 0;
    const configuredRetries = Number(process.env.GWP_INLINE_RECONCILE_TRIES ?? "3");
    const maxInlineRetries = Number.isSafeInteger(configuredRetries) && configuredRetries >= 0
      ? configuredRetries
      : 3;
    const inlineBackoffMs = envMs("GWP_INLINE_RECONCILE_BACKOFF_MS", 20_000);
    const retryUncertainInline = async (outcome: Envelope): Promise<boolean> => {
      if (outcome.errorKind !== "send_uncertain"
        || inlineReconcileTries >= maxInlineRetries
        || deadline - Date.now() < inlineBackoffMs * 2
        || this.requireRequest(id).status !== "uncertain") return false;
      inlineReconcileTries += 1;
      this.log(id, "uncertain", `inline reconcile retry ${inlineReconcileTries}/${maxInlineRetries}`);
      await new Promise<void>((resolve) => setTimeout(resolve, inlineBackoffMs));
      return Date.now() < deadline;
    };
    for (;;) {
      let request = this.requireRequest(id);
      if (TERMINAL_STATUSES.has(request.status)) return this.envelopeFor(request);
      if (request.status === "sending" || request.status === "uncertain") {
        const outcome = await this.recoverSendState(request, deadline);
        if (outcome && !await retryUncertainInline(outcome)) return outcome;
        continue;
      }
      if (request.status === "generating") {
        return this.pollUntilDeadline(request, deadline);
      }
      if (request.status !== "staged") {
        this.db.setRequestStatus(id, "failed", "internal", `unsupported request state ${request.status}`);
        continue;
      }
      if (!request.slot_id) {
        const weeklyLimits = resolveWeeklyLimits(this.config);
        const claim = claimSlotForRequest(
          this.db,
          this.config.slots,
          this.config.maxConcurrent,
          request.id,
          Date.now(),
          triedSlots,
          weeklyLimits,
        );
        request = claim.request;
        if (request.status !== "staged") continue;
        if (!request.slot_id) {
          const slots = this.db.listSlots();
          const now = Date.now();
          const hasCapacityBlocked = slots.some((item) => (
            item.state === "idle"
              || (item.state === "provider_limit" && item.cooldown_until !== null
                && item.cooldown_until <= now)
          ) && this.db.countActiveForSlot(item.id) >= this.config.maxConcurrent);
          const hasProviderLimit = slots.some((item) => item.state === "provider_limit");
          // 상태로는 쓸 수 있는데 주간 한도 때문에 빠진 슬롯만 남았으면 weekly_limit로 대기시킨다.
          const usable = slots.filter((item) => !triedSlots.has(item.id) && (item.state === "idle"
            || (item.state === "provider_limit" && item.cooldown_until !== null && item.cooldown_until <= now)));
          const usages = usable.map((item) => weeklyUsageFor(this.db, item.id, weeklyLimits.get(item.id) ?? null, now));
          if (usable.length > 0 && usages.every((usage) => usage.exhausted) && !hasCapacityBlocked) {
            const resetAt = Math.min(...usages.map((usage) => usage.resetAt ?? Number.POSITIVE_INFINITY));
            const when = Number.isFinite(resetAt) ? new Date(resetAt).toISOString() : "unknown";
            return recoveringEnvelope(id, "weekly_limit", `all usable accounts reached the weekly limit; earliest reset at ${when}`);
          }
          if ((sawProviderLimit || hasProviderLimit) && !hasCapacityBlocked) {
            return recoveringEnvelope(id, "provider_limit", "all currently usable accounts are cooling down");
          }
          if (sawLogin && slots.every((item) => item.state === "needs_login")) {
            this.db.setRequestStatus(id, "needs_user_action", "needs_login", "all slots require login");
            continue;
          }
          return recoveringEnvelope(id, "pool_busy", "no idle slot is currently available");
        }
      }
      const slot = this.requireSlotConfig(request.slot_id!);
      await this.stageFiles(request);
      let client: RpcClient | null = null;
      try {
        client = await this.connectDaemon(slot);
        const readiness = await client.call("readiness", undefined, 40_000);
        if (readiness.state === "needs_login") {
          sawLogin = true;
          triedSlots.add(slot.id);
          markSlotNeedsLogin(this.db, slot.id);
          this.db.updateRequest(id, { slot_id: null });
          await client.close();
          client = null;
          await this.stopSlotIfNoLiveOwners(slot, undefined, true);
          continue;
        }
        if (readiness.state === "provider_limit") {
          sawProviderLimit = true;
          triedSlots.add(slot.id);
          markSlotProviderLimit(this.db, slot.id);
          this.db.updateRequest(id, { slot_id: null });
          await client.close();
          client = null;
          await this.stopSlotIfNoLiveOwners(slot, undefined, true);
          continue;
        }
        if (readiness.state !== "ready") {
          this.db.setRequestStatus(id, "failed", "daemon_unreachable", "daemon readiness is unknown");
          await this.releaseRuntime(this.requireRequest(id));
          continue;
        }
        markSlotIdle(this.db, slot.id);
        const outcome = await this.send(request, slot, client);
        client = null;
        if (outcome && !await retryUncertainInline(outcome)) return outcome;
      } catch (error) {
        if (client) await client.close().catch(() => undefined);
        const current = this.requireRequest(id);
        if (current.status === "sending" || current.status === "uncertain") {
          continue;
        }
        if (isDirectNetworkFailure(error)) {
          this.db.setRequestStatus(id, "failed", "network_disconnected", errorMessage(error));
          await this.releaseRuntime(this.requireRequest(id));
          return networkFailureEnvelope(id, errorMessage(error));
        }
        this.db.setRequestStatus(id, "failed", "daemon_unreachable", errorMessage(error));
        await this.releaseRuntime(this.requireRequest(id));
        return failedEnvelope(id, "daemon_unreachable", errorMessage(error));
      }
    }
  }
  private async send(
    request: RequestRow,
    slot: SlotConfig,
    client: RpcClient,
  ): Promise<Envelope | null> {
    const prompt = await readFile(path.join(this.requestDir(request.id), "prompt.md"), "utf8");
    const files = await this.rpcFiles(request.id, slot);
    const conversationUrl = await readFile(path.join(this.requestDir(request.id), "continue_url"), "utf8").catch(() => "");
    let sendLock;
    try {
      sendLock = await tryAcquireFileLock(this.sendLockPath(request.id));
    } catch (error) {
      const message = errorMessage(error);
      this.db.setRequestStatus(request.id, "failed", "internal", message);
      await client.close().catch(() => undefined);
      await this.releaseRuntime(this.requireRequest(request.id));
      return failedEnvelope(request.id, "internal", message);
    }
    if (!sendLock) {
      await client.close().catch(() => undefined);
      return runningEnvelope(request.id, SEND_IN_PROGRESS_MESSAGE);
    }
    try {
      const current = this.requireRequest(request.id);
      if (current.status !== "staged" || current.slot_id !== slot.id) {
        await client.close().catch(() => undefined);
        return null;
      }
      const attemptNo = armSendAttempt(this.db, request.id);
      this.log(request.id, "sending", `attempt ${attemptNo} armed`);
      let lastProgress: SendProgress | undefined;
      let lastLoggedStep: string | undefined;
      try {
        const result = await client.call("send", { prompt, files, ...(conversationUrl ? { conversationUrl } : {}) }, {
          timeoutMs: SEND_RPC_MAX_MS,
          inactivityMs: SEND_RPC_INACTIVITY_MS,
          onProgress: (progress) => {
            lastProgress = progress;
            if (progress.step !== lastLoggedStep) {
              lastLoggedStep = progress.step;
              this.log(
                request.id,
                "sending",
                `send ${progress.step} +${Math.round(progress.elapsedMs / 1_000)}s`
                  + (progress.matchDebug ? ` (${progress.matchDebug})` : ""),
              );
            }
          },
        });
        const finalState = confirmSendAttempt(this.db, request.id, result);
        this.log(
          request.id,
          "generating",
          `attempt ${attemptNo} ${finalState}`
            + (result.matchedBy && result.matchedBy !== "strict" ? ` (matched: ${result.matchedBy})` : ""),
        );
        await client.close();
        return null;
      } catch (rawError) {
        const error = withProgressAnchors(rawError, lastProgress);
        const phase = error instanceof GwpError ? error.phase : undefined;
        if (phase === "pre_click") {
          const finalState = markPreClickNoSend(this.db, request.id);
          if (finalState === "confirmed" || finalState === "reconciled") {
            await client.close().catch(() => undefined);
            return null;
          }
          if (error instanceof GwpError && error.kind === "model_unavailable") {
            this.db.setRequestStatus(request.id, "needs_user_action", "model_unavailable", error.detail);
            await client.close().catch(() => undefined);
            await this.releaseRuntime(this.requireRequest(request.id));
            return actionEnvelope(request.id, "model_unavailable", error.detail);
          }
          if (error instanceof GwpError && error.kind === "needs_login") {
            markSlotNeedsLogin(this.db, slot.id);
            this.db.updateRequest(request.id, { status: "staged", slot_id: null });
            await client.close().catch(() => undefined);
            await this.stopSlotIfNoLiveOwners(slot, undefined, true);
            return null;
          }
          if (error instanceof GwpError && error.kind === "provider_limit") {
            markSlotProviderLimit(this.db, slot.id);
            this.db.updateRequest(request.id, { status: "staged", slot_id: null });
            await client.close().catch(() => undefined);
            await this.stopSlotIfNoLiveOwners(slot, undefined, true);
            return null;
          }
          const message = errorMessage(error);
          const network = isDirectNetworkFailure(error);
          this.db.setRequestStatus(
            request.id,
            "failed",
            network ? "network_disconnected" : "internal",
            message,
          );
          await client.close().catch(() => undefined);
          await this.releaseRuntime(this.requireRequest(request.id));
          return network
            ? networkFailureEnvelope(request.id, message)
            : failedEnvelope(request.id, "internal", message);
        }
        const finalState = markSendUncertain(
          this.db,
          request.id,
          errorMessage(error),
          error instanceof GwpError
            ? {
                ...(error.pendingUserTurnId
                  ? { pendingUserTurnId: error.pendingUserTurnId }
                  : {}),
                ...(error.pendingConversationUrl
                  ? { pendingConversationUrl: error.pendingConversationUrl }
                  : {}),
                ...(error.preClickBaseline
                  ? { preClickBaseline: error.preClickBaseline }
                  : {}),
              }
            : undefined,
        );
        await client.close().catch(() => undefined);
        if (finalState === "confirmed" || finalState === "reconciled") return null;
        this.log(request.id, "uncertain", errorMessage(error));
        return actionEnvelope(request.id, "send_uncertain", errorMessage(error), true);
      }
    } finally {
      await sendLock.release();
    }
  }
  private async recoverSendState(
    request: RequestRow,
    deadline: number,
  ): Promise<Envelope | null> {
    const sendLock = await tryAcquireFileLock(this.sendLockPath(request.id));
    if (!sendLock) return runningEnvelope(request.id, SEND_IN_PROGRESS_MESSAGE);
    try {
      let current = this.requireRequest(request.id);
      if (current.status === "sending") {
        const attempt = this.db.latestAttempt(request.id);
        if (!attempt || attempt.state !== "armed") {
          if (attempt?.state === "confirmed" || attempt?.state === "reconciled") return null;
          this.db.setRequestStatus(request.id, "failed", "internal", "sending request has no armed attempt");
          return null;
        }
        const finalState = markSendUncertain(
          this.db,
          request.id,
          "send owner exited after the attempt was armed",
        );
        if (finalState === "confirmed" || finalState === "reconciled") return null;
        this.log(request.id, "uncertain", "armed attempt recovered after send owner exit");
        current = this.requireRequest(request.id);
      }
      if (current.status !== "uncertain") return null;
      return this.reconcile(current, deadline);
    } finally {
      await sendLock.release();
    }
  }
  private async reconcile(request: RequestRow, deadline: number): Promise<Envelope | null> {
    const attempt = this.db.latestAttempt(request.id);
    if (!attempt || attempt.state !== "uncertain") {
      this.db.setRequestStatus(request.id, "failed", "internal", "uncertain request has no uncertain attempt");
      return null;
    }
    if (!request.slot_id) {
      return actionEnvelope(request.id, "send_uncertain", "the original slot is unknown", true);
    }
    let client: RpcClient | null = null;
    try {
      const slot = this.requireSlotConfig(request.slot_id);
      client = await this.connectDaemon(slot);
      const prompt = await readFile(path.join(this.requestDir(request.id), "prompt.md"), "utf8");
      const promptSha256 = this.requireRequest(request.id).prompt_sha256;
      const anchor = decodeSendAnchor(request.error_detail);
      const pendingConversationUrl = anchor?.pendingConversationUrl
        ?? request.conversation_url
        ?? undefined;
      const result = await client.call("reconcile", {
        prompt,
        promptSha256,
        ...(attempt.user_turn_id ? { pendingUserTurnId: attempt.user_turn_id } : {}),
        ...(request.conversation_url && isValidConversationPointer(request.conversation_url)
          ? { conversationUrl: request.conversation_url }
          : {}),
        ...(pendingConversationUrl ? { pendingConversationUrl } : {}),
        ...(anchor?.preClickBaseline ? { preClickBaseline: anchor.preClickBaseline } : {}),
        // 대형 user 턴이 있는 대화의 네비게이션+관측은 65s를 넘길 수 있다.
      }, Math.max(5_000, Math.min(120_000, deadline - Date.now() + 5_000)));
      await client.close();
      client = null;
      if (result.matchedBy && result.matchedBy !== "strict") {
        this.log(request.id, "uncertain", `reconcile matched by ${result.matchedBy}`);
      }
      if (result.evidence) {
        this.log(request.id, "uncertain", `reconcile mismatch evidence: ${result.evidence}`);
      }
      const decision = applyReconcileResult(this.db, request.id, result);
      if (decision === "found") {
        this.log(request.id, "generating", "uncertain send reconciled as found");
        return null;
      }
      if (decision === "retry_send") {
        this.log(request.id, "staged", "reconcile proved no send; attempt 2 allowed");
        return null;
      }
      if (decision === "exhausted") {
        await this.releaseRuntime(this.requireRequest(request.id));
        return actionEnvelope(request.id, "send_uncertain", "two attempts ended without a confirmed send");
      }
      return actionEnvelope(
        request.id,
        "send_uncertain",
        "the open ChatGPT tabs cannot prove whether the send occurred",
        true,
      );
    } catch (error) {
      if (client) await client.close().catch(() => undefined);
      return actionEnvelope(
        request.id,
        "send_uncertain",
        `reconcile could not prove the send state: ${errorMessage(error)}`,
        true,
      );
    }
  }
  private async pollUntilDeadline(request: RequestRow, deadline: number): Promise<Envelope> {
    if (!request.slot_id || !request.conversation_url) {
      this.db.setRequestStatus(request.id, "failed", "internal", "generating request is missing slot or conversation URL");
      return failedEnvelope(request.id, "internal", "generating request is missing slot or conversation URL");
    }
    let client: RpcClient | null = null;
    try {
      const slot = this.requireSlotConfig(request.slot_id);
      let attempt = this.db.latestAttempt(request.id);
      if (!attempt || (attempt.state !== "confirmed" && attempt.state !== "reconciled")) {
        throw new Error("generating request is missing a confirmed or reconciled send attempt");
      }
      let current = request;
      client = await this.connectDaemon(slot);
      for (;;) {
        const remaining = deadline - Date.now();
        if (remaining <= 0) {
          await client.close();
          return runningEnvelope(request.id);
        }
        const result = await client.call("poll", {
          conversationUrl: current.conversation_url!,
          promptSha256: current.prompt_sha256,
          ...(attempt.user_turn_id ? { userTurnId: attempt.user_turn_id } : {}),
          ...(attempt.assistant_turn_id ? { assistantTurnId: attempt.assistant_turn_id } : {}),
          waitMs: Math.min(60_000, Math.max(0, remaining)),
        }, Math.min(65_000, Math.max(5_000, remaining + 5_000)));
        current = this.updateConversationPointer(current, result.currentUrl);
        if (result.assistantTurnId && attempt.user_turn_id) {
          const rebound = this.db.rebindAssistantTurnId(
            request.id,
            attempt.attempt_no,
            attempt.user_turn_id,
            attempt.assistant_turn_id,
            result.assistantTurnId,
          );
          attempt = rebound.row;
          if (rebound.changed) this.log(request.id, "generating", "assistant turn id updated");
        }
        if (current.status !== "generating") {
          await client.close();
          return this.envelopeFor(current);
        }
        if (result.state === "generating") {
          if (Date.now() >= deadline) {
            await client.close();
            return runningEnvelope(request.id);
          }
          await new Promise((resolve) => setTimeout(resolve, 25));
          continue;
        }
        const envelope = await this.finalizeComplete(current, slot, client, result);
        client = null;
        return envelope;
      }
    } catch (error) {
      if (client) await client.close().catch(() => undefined);
      const message = errorMessage(error);
      return this.finishGeneratingFailure(
        request.id,
        isDirectNetworkFailure(error) ? "network_disconnected" : "internal",
        message,
      );
    }
  }
  private updateConversationPointer(request: RequestRow, currentUrl: string): RequestRow {
    let changed = false;
    const result = this.db.immediate(() => {
      const persisted = this.requireRequest(request.id);
      if (currentUrl === persisted.conversation_url || !isValidConversationPointer(currentUrl)) {
        return persisted;
      }
      if (isTemporaryConversationPointer(currentUrl)
        && persisted.conversation_url
        && !isTemporaryConversationPointer(persisted.conversation_url)) return persisted;
      const updated = this.db.connection.prepare(`
        UPDATE requests
        SET conversation_url = ?, updated_at = ?
        WHERE id = ? AND status = 'generating'
      `).run(currentUrl, Date.now(), request.id);
      changed = updated.changes === 1;
      return this.requireRequest(request.id);
    });
    if (changed) this.log(request.id, "generating", `conversation URL updated to ${currentUrl}`);
    return result;
  }
  private async finalizeComplete(
    request: RequestRow,
    slot: SlotConfig,
    client: RpcClient,
    result: PollResult,
  ): Promise<Envelope> {
    const finalizationLock = await tryAcquireFileLock(this.sendLockPath(request.id));
    if (!finalizationLock) {
      await client.close().catch(() => undefined);
      return runningEnvelope(request.id);
    }
    try {
      const current = this.requireRequest(request.id);
      if (current.status !== "generating") {
        await client.close().catch(() => undefined);
        return this.envelopeFor(current);
      }
      if (result.answerMarkdown === undefined || !result.answerSha256
        || sha256Text(result.answerMarkdown) !== result.answerSha256 || (!result.answerMarkdown && !result.artifactControls?.length)) {
        throw new Error("daemon returned an invalid complete answer");
      }
      const answerPath = path.join(this.requestDir(request.id), "answer.md");
      await atomicWrite(answerPath, result.answerMarkdown);
      const failedControls: string[] = [];
      for (const control of result.artifactControls ?? []) {
        let stored = false;
        let lastFailure = "";
        for (let attempt = 1; attempt <= 2 && !stored; attempt += 1) {
          try {
            const downloaded = await client.call("download", {
              conversationUrl: this.requireRequest(request.id).conversation_url!,
              controlIndex: control.index,
            }, 45_000);
            await this.storeArtifact(request, slot, downloaded);
            stored = true;
          } catch (error) {
            lastFailure = errorMessage(error);
          }
        }
        if (!stored) failedControls.push(`${control.label}: ${lastFailure}`);
      }
      const artifactMessage = failedControls.length > 0
        ? `${failedControls.length} artifact control(s) failed after two attempts: ${failedControls.join("; ")}`
        : null;
      const completed = this.db.connection.prepare(`
        UPDATE requests
        SET status = 'complete', answer_sha256 = ?, error_kind = NULL,
            error_detail = ?, updated_at = ?
        WHERE id = ? AND status = 'generating'
      `).run(result.answerSha256, artifactMessage, Date.now(), request.id);
      if (completed.changes !== 1) {
        await client.close();
        return this.envelopeFor(this.requireRequest(request.id));
      }
      this.log(request.id, "complete");
      await this.releaseRuntime(this.requireRequest(request.id), client);
      return this.envelopeFor(this.requireRequest(request.id));
    } finally {
      await finalizationLock.release();
    }
  }
  private async finishGeneratingFailure(
    requestId: string,
    errorKind: PublicErrorKind,
    detail: string,
  ): Promise<Envelope> {
    const failureLock = await tryAcquireFileLock(this.sendLockPath(requestId));
    if (!failureLock) return runningEnvelope(requestId);
    try {
      const current = this.requireRequest(requestId);
      if (current.status !== "generating") return this.envelopeFor(current);
      this.db.connection.prepare(`
        UPDATE requests
        SET status = 'failed', error_kind = ?, error_detail = ?, updated_at = ?
        WHERE id = ? AND status = 'generating'
      `).run(errorKind, detail, Date.now(), requestId);
      await this.releaseRuntime(this.requireRequest(requestId));
      return errorKind === "network_disconnected"
        ? networkFailureEnvelope(requestId, detail)
        : failedEnvelope(requestId, "internal", detail);
    } finally {
      await failureLock.release();
    }
  }
  private async storeArtifact(
    request: RequestRow,
    slot: SlotConfig,
    downloaded: { filename: string; outboxPath: string; sha256: string; sizeBytes: number },
  ): Promise<void> {
    const managed = slot.unmanaged !== true;
    const source = managed
      ? mapContainerOutboxPath(downloaded.outboxPath, this.docker.paths(slot.id).outbox)
      : downloaded.outboxPath;
    try {
      const filename = path.basename(downloaded.filename);
      if (!filename || filename !== downloaded.filename || filename === "." || filename === "..") {
        throw new Error("daemon returned an unsafe artifact filename");
      }
      if (await sha256File(source) !== downloaded.sha256
        || await fileSize(source) !== downloaded.sizeBytes) {
        throw new Error("artifact metadata does not match outbox bytes");
      }
      const target = path.join(this.requestDir(request.id), "artifacts", filename);
      try {
        await access(target);
        const targetSha256 = await sha256File(target);
        const targetSize = await fileSize(target);
        if (targetSha256 !== downloaded.sha256 || targetSize !== downloaded.sizeBytes) {
          throw new Error(`duplicate artifact filename: ${filename}`);
        }
        if (path.resolve(source) !== path.resolve(target)) await rm(source, { force: true });
        this.db.addArtifact({
          request_id: request.id,
          filename,
          path: target,
          sha256: targetSha256,
          size_bytes: targetSize,
          created_at: Date.now(),
        });
        return;
      } catch (error) {
        if (error instanceof Error && !("code" in error && error.code === "ENOENT")) throw error;
      }
      await moveFile(source, target);
      const sha256 = await sha256File(target);
      const sizeBytes = await fileSize(target);
      if (sha256 !== downloaded.sha256 || sizeBytes !== downloaded.sizeBytes) {
        throw new Error("stored artifact bytes changed during transfer");
      }
      this.db.addArtifact({
        request_id: request.id,
        filename,
        path: target,
        sha256,
        size_bytes: sizeBytes,
        created_at: Date.now(),
      });
    } finally {
      if (managed) await rm(source, { force: true });
    }
  }
  private async persistAttachments(requestId: string, files: ValidatedFile[]): Promise<void> {
    const directory = path.join(this.requestDir(requestId), "attachments");
    await mkdirp(directory);
    for (const file of files) await copyFile(file.source, path.join(directory, file.name));
  }
  private async stageFiles(request: RequestRow): Promise<void> {
    if (!request.slot_id) return;
    const attachments = path.join(this.requestDir(request.id), "attachments");
    const names = (await readdir(attachments)).sort();
    if (names.length === 0) return;
    const inbox = path.join(this.docker.paths(request.slot_id).inbox, request.id);
    await mkdirp(inbox);
    for (const name of names) {
      await copyFile(path.join(attachments, name), path.join(inbox, name));
    }
  }
  private async rpcFiles(requestId: string, slot: SlotConfig): Promise<RpcFile[]> {
    const inbox = path.join(this.docker.paths(slot.id).inbox, requestId);
    let names: string[] = [];
    try {
      names = (await readdir(inbox)).sort();
    } catch (error) {
      if (!(error instanceof Error) || !("code" in error) || error.code !== "ENOENT") throw error;
    }
    return names.map((name) => ({
      name,
      containerPath: slot.unmanaged === true
        ? path.join(inbox, name)
        : path.posix.join("/inbox", requestId, name),
    }));
  }
  private async envelopeFor(request: RequestRow): Promise<Envelope> {
    if (request.status === "complete") {
      const answerPath = path.join(this.requestDir(request.id), "answer.md");
      const answer = await readFile(answerPath, "utf8");
      const artifacts: PublicArtifact[] = this.db.listArtifacts(request.id).map((item) => ({
        filename: item.filename,
        path: item.path,
        sha256: item.sha256,
        sizeBytes: item.size_bytes,
      }));
      return completeEnvelope({
        sessionId: request.id,
        answer,
        answerPath,
        answerSha256: request.answer_sha256!,
        artifacts,
        message: request.error_detail,
      });
    }
    if (request.status === "needs_user_action") {
      return actionEnvelope(
        request.id,
        request.error_kind,
        request.error_detail ?? "user action is required",
        request.error_kind === "send_uncertain",
      );
    }
    if (request.status === "failed") {
      if (request.error_kind === "network_disconnected") {
        return networkFailureEnvelope(request.id, request.error_detail ?? "network disconnected");
      }
      return failedEnvelope(request.id, request.error_kind ?? "internal", request.error_detail ?? "request failed");
    }
    if (request.status === "generating") return runningEnvelope(request.id);
    if (request.status === "uncertain") {
      return actionEnvelope(request.id, "send_uncertain", request.error_detail ?? "send state is uncertain", true);
    }
    return recoveringEnvelope(request.id, "pool_busy", "request is waiting for a slot");
  }
  private async releaseRuntime(request: RequestRow, client: RpcClient | null = null): Promise<void> {
    if (!request.slot_id || ACTIVE_STATUSES.has(request.status)) return;
    await this.closeConversationBestEffort(request, client);
    if (client) await client.close().catch(() => undefined);
    await this.stopSlotIfNoLiveOwners(this.requireSlotConfig(request.slot_id), undefined, true);
  }
  private async closeConversationBestEffort(
    request: RequestRow,
    existingClient: RpcClient | null,
  ): Promise<void> {
    if (!request.slot_id || !request.conversation_url) return;
    const slot = this.requireSlotConfig(request.slot_id);
    let client = existingClient;
    let ownsClient = false;
    try {
      if (!client) {
        if (slot.unmanaged !== true && !(await this.docker.inspect(slot.id)).running) return;
        const paths = this.docker.paths(slot.id);
        client = await RpcClient.connect(slot.port, paths.tokenPath, 2_000);
        ownsClient = true;
      }
      await client.call("closeConversation", {
        conversationUrl: request.conversation_url,
      }, 5_000);
    } catch {
      // Terminalization is authoritative in SQLite; tab cleanup is intentionally
      // best-effort and must never turn a completed request back into a failure.
    } finally {
      if (ownsClient && client) await client.close().catch(() => undefined);
    }
  }
  private async stopSlot(slot: SlotConfig): Promise<void> {
    if (slot.unmanaged === true) return;
    await this.withSlotControlLock(slot, () => this.docker.stop(slot.id));
  }
  private async connectDaemon(slot: SlotConfig): Promise<RpcClient> {
    const endpoint = await this.withSlotControlLock(slot, () => this.docker.ensure(slot));
    return RpcClient.connect(endpoint.port, endpoint.tokenPath);
  }
  private async stopSlotIfNoLiveOwners(
    slot: SlotConfig,
    excludingRequestId: string | undefined,
    apply: boolean,
  ): Promise<"not_running" | "in_use" | "would_stop" | "stopped"> {
    if (slot.unmanaged === true) return "not_running";
    const activityProbe = await tryAcquireFileLock(this.slotActivityLockPath(slot.id));
    if (!activityProbe) return "in_use";
    const probes: FileLock[] = [];
    try {
      return await this.withSlotControlLock(slot, async () => {
        for (const request of this.db.listActiveForSlot(slot.id)) {
          if (request.id === excludingRequestId) continue;
          const probe = await tryAcquireFileLock(this.ownerLockPath(request.id));
          if (!probe) return "in_use";
          probes.push(probe);
        }
        const inspected = await this.docker.inspect(slot.id);
        if (!inspected.running) return "not_running";
        if (!apply) return "would_stop";
        await this.docker.stop(slot.id);
        return "stopped";
      });
    } finally {
      await Promise.all(probes.map((probe) => probe.release().catch(() => undefined)));
      await activityProbe.release().catch(() => undefined);
    }
  }
  private async withSlotControlLock<T>(
    slot: SlotConfig,
    operation: () => Promise<T>,
  ): Promise<T> {
    if (slot.unmanaged === true) return operation();
    const lock = await acquireFileLock(this.slotControlLockPath(slot.id));
    try {
      return await operation();
    } finally {
      await lock.release();
    }
  }
  private requestDir(id: string): string {
    return path.join(this.stateDir, "requests", id);
  }
  private sendLockPath(id: string): string {
    return path.join(this.requestDir(id), "send.lock");
  }
  private ownerLockPath(id: string): string {
    return path.join(this.requestDir(id), "owner.lock");
  }
  private slotControlLockPath(slotId: string): string {
    return path.join(this.stateDir, "slots", slotId, "runtime-control.lock");
  }
  private slotActivityLockPath(slotId: string): string {
    return path.join(this.stateDir, "slots", slotId, "runtime-activity.lock");
  }
  private requireRequest(id: string): RequestRow {
    const request = this.db.getRequest(id);
    if (!request) throw new InputError(`unknown session: ${id}`);
    return request;
  }
  private requireSlotConfig(id: string): SlotConfig {
    const slot = this.config.slots.find((item) => item.id === id);
    if (!slot) throw new Error(`slot ${id} is absent from config`);
    return slot;
  }
  private log(id: string, status: string, detail?: string): void {
    void appendJsonLine(path.join(this.requestDir(id), "log.jsonl"), {
      at: Date.now(),
      status,
      ...(detail ? { detail } : {}),
    }).catch(() => undefined);
  }
}
// 로컬에서 send 대기를 포기(절대 상한/무진행/소켓 오류)해도 daemon이 progress로 복제해 준
// 마지막 앵커를 잃지 않는다. 클릭 가능성을 배제할 수 없으므로 phase는 무조건 post_click
// (fail-closed, §5.2). daemon이 직접 던진 GwpError는 이미 자체 앵커를 실어 온다.
export function withProgressAnchors(error: unknown, progress: SendProgress | undefined): unknown {
  if (error instanceof GwpError || !progress) return error;
  return new GwpError(
    "click_uncertain",
    `${errorMessage(error)} (last step: ${progress.step} +${Math.round(progress.elapsedMs / 1_000)}s)`,
    {
      phase: "post_click",
      cause: error,
      ...(progress.pendingUserTurnId ? { pendingUserTurnId: progress.pendingUserTurnId } : {}),
      ...(progress.pendingConversationUrl
        ? { pendingConversationUrl: progress.pendingConversationUrl }
        : {}),
      ...(progress.preClickBaseline ? { preClickBaseline: progress.preClickBaseline } : {}),
    },
  );
}
export function armSendAttempt(db: GwpDatabase, requestId: string): number {
  return db.immediate(() => {
    const previous = db.latestAttempt(requestId);
    if (previous && previous.attempt_no >= 2) throw new Error("send attempt limit exceeded");
    const request = db.getRequest(requestId);
    if (!request || request.status !== "staged") {
      throw new Error("send attempt requires a staged request");
    }
    let attemptNo: number;
    if (!previous) attemptNo = 1;
    else if (previous.attempt_no === 1 && previous.state === "no_send_proven") attemptNo = 2;
    else throw new Error("attempt 2 requires attempt 1 to be no_send_proven");
    db.createAttempt(requestId, attemptNo);
    const updated = db.connection.prepare(`
      UPDATE requests
      SET status = 'sending', error_kind = NULL, error_detail = NULL, updated_at = ?
      WHERE id = ? AND status = 'staged'
    `).run(Date.now(), requestId);
    if (updated.changes !== 1) throw new Error("send attempt lost its staged request");
    return attemptNo;
  });
}
export function confirmSendAttempt(
  db: GwpDatabase,
  requestId: string,
  result: SendResult,
): "confirmed" | "reconciled" {
  return db.immediate(() => {
    const attempt = db.latestAttempt(requestId);
    if (!attempt) throw new Error("confirmed send requires an armed attempt");
    const transition = db.transitionAttempt(requestId, attempt.attempt_no, ["armed"], "confirmed", {
      userTurnId: result.userTurnId,
      ...(result.assistantTurnId !== undefined ? { assistantTurnId: result.assistantTurnId } : {}),
    });
    if (!transition.changed) {
      if (transition.row.state === "confirmed" || transition.row.state === "reconciled") {
        return transition.row.state;
      }
      throw new Error("confirmed send requires an armed attempt");
    }
    const now = Date.now();
    db.connection.prepare(`
      UPDATE requests
      SET status = 'generating', conversation_url = ?, error_kind = NULL,
          error_detail = NULL, updated_at = ?
      WHERE id = ? AND status = 'sending'
    `).run(result.conversationUrl, now, requestId);
    recordUsageFor(db, requestId, result.modelLabel ?? null, now);
    return "confirmed";
  });
}
export function markPreClickNoSend(db: GwpDatabase, requestId: string):
  "no_send_proven" | "confirmed" | "reconciled" {
  const attempt = db.latestAttempt(requestId);
  if (!attempt) throw new Error("pre-click failure requires an armed attempt");
  const transition = db.transitionAttempt(
    requestId,
    attempt.attempt_no,
    ["armed"],
    "no_send_proven",
  );
  if (transition.changed || transition.row.state === "no_send_proven") return "no_send_proven";
  if (transition.row.state === "confirmed" || transition.row.state === "reconciled") {
    return transition.row.state;
  }
  throw new Error("pre-click failure requires an armed attempt");
}
export function markSendUncertain(
  db: GwpDatabase,
  requestId: string,
  detail: string,
  anchor?: {
    pendingUserTurnId?: string;
    pendingConversationUrl?: string;
    preClickBaseline?: string[];
  },
): "uncertain" | "confirmed" | "reconciled" {
  return db.immediate(() => {
    const attempt = db.latestAttempt(requestId);
    if (!attempt) throw new Error("uncertain send requires an armed attempt");
    const transition = db.transitionAttempt(
      requestId,
      attempt.attempt_no,
      ["armed"],
      "uncertain",
      anchor?.pendingUserTurnId ? { userTurnId: anchor.pendingUserTurnId } : {},
    );
    if (!transition.changed) {
      if (transition.row.state === "confirmed" || transition.row.state === "reconciled") {
        return transition.row.state;
      }
      if (transition.row.state === "uncertain") return "uncertain";
      throw new Error("uncertain send requires an armed attempt");
    }
    const storedDetail = encodeSendAnchor(detail, anchor);
    const requestUpdate = db.connection.prepare(`
      UPDATE requests
      SET status = 'uncertain', error_kind = 'send_uncertain', error_detail = ?,
          conversation_url = CASE
            WHEN (conversation_url IS NULL OR conversation_url = '') AND ? IS NOT NULL THEN ?
            ELSE conversation_url
          END,
          updated_at = ?
      WHERE id = ? AND status = 'sending'
    `).run(
      storedDetail,
      anchor?.pendingConversationUrl ?? null,
      anchor?.pendingConversationUrl ?? null,
      Date.now(),
      requestId,
    );
    if (requestUpdate.changes !== 1) throw new Error("uncertain send lost its sending request");
    return "uncertain";
  });
}
interface StoredSendAnchor {
  detail: string;
  pendingConversationUrl?: string;
  preClickBaseline?: string[];
}
function encodeSendAnchor(
  detail: string,
  anchor: { pendingConversationUrl?: string; preClickBaseline?: string[] } | undefined,
): string {
  if (!anchor?.pendingConversationUrl && !anchor?.preClickBaseline) return detail;
  return JSON.stringify({
    detail,
    ...(anchor.pendingConversationUrl
      ? { pendingConversationUrl: anchor.pendingConversationUrl }
      : {}),
    ...(anchor.preClickBaseline ? { preClickBaseline: anchor.preClickBaseline } : {}),
  } satisfies StoredSendAnchor);
}
function decodeSendAnchor(value: string | null): StoredSendAnchor | null {
  if (!value?.startsWith("{")) return null;
  try {
    const parsed = JSON.parse(value) as Partial<StoredSendAnchor>;
    if (typeof parsed.detail !== "string") return null;
    if (parsed.pendingConversationUrl !== undefined
      && typeof parsed.pendingConversationUrl !== "string") return null;
    if (parsed.preClickBaseline !== undefined
      && (!Array.isArray(parsed.preClickBaseline)
        || !parsed.preClickBaseline.every((item) => typeof item === "string"))) return null;
    return parsed as StoredSendAnchor;
  } catch {
    return null;
  }
}
export function applyReconcileResult(
  db: GwpDatabase,
  requestId: string,
  result: ReconcileResult,
): "found" | "retry_send" | "exhausted" | "unproven" {
  const attempt = db.latestAttempt(requestId);
  if (!attempt) {
    throw new Error("reconcile requires an uncertain attempt");
  }
  if (attempt.user_turn_id
    && (!result.found || result.userTurnId !== attempt.user_turn_id)) return "unproven";
  if (result.found
    && (!result.conversationUrl || !result.userTurnId
      || !isValidConversationPointer(result.conversationUrl))) return "unproven";
  if (result.found && result.conversationUrl && result.userTurnId) {
    return db.immediate(() => {
      const transition = db.transitionAttempt(requestId, attempt.attempt_no, ["uncertain"], "reconciled", {
        userTurnId: result.userTurnId ?? null,
        assistantTurnId: result.assistantTurnId ?? null,
      });
      if (!transition.changed) {
        if (transition.row.state === "confirmed" || transition.row.state === "reconciled") {
          return "found";
        }
        throw new Error("reconcile requires an uncertain attempt");
      }
      const now = Date.now();
      db.connection.prepare(`
        UPDATE requests
        SET status = 'generating', conversation_url = ?, error_kind = NULL,
            error_detail = NULL, updated_at = ?
        WHERE id = ? AND status = 'uncertain'
      `).run(result.conversationUrl, now, requestId);
      recordUsageFor(db, requestId, null, now);
      return "found";
    });
  }
  if (!result.proven) {
    if (attempt.state === "confirmed" || attempt.state === "reconciled") return "found";
    if (attempt.state !== "uncertain") throw new Error("reconcile requires an uncertain attempt");
    return "unproven";
  }
  return db.immediate(() => {
    const transition = db.transitionAttempt(
      requestId,
      attempt.attempt_no,
      ["uncertain"],
      "no_send_proven",
    );
    if (!transition.changed
      && (transition.row.state === "confirmed" || transition.row.state === "reconciled")) {
      return "found";
    }
    if (!transition.changed && transition.row.state !== "no_send_proven") {
      throw new Error("reconcile requires an uncertain attempt");
    }
    if (attempt.attempt_no < 2) {
      db.connection.prepare(`
        UPDATE requests
        SET status = 'staged', error_kind = NULL, error_detail = NULL, updated_at = ?
        WHERE id = ? AND status = 'uncertain'
      `).run(Date.now(), requestId);
      return "retry_send";
    }
    db.connection.prepare(`
      UPDATE requests
      SET status = 'needs_user_action', error_kind = 'send_uncertain',
          error_detail = 'two attempts ended without a confirmed send', updated_at = ?
      WHERE id = ? AND status = 'uncertain'
    `).run(Date.now(), requestId);
    return "exhausted";
  });
}
function defaultStateDir(): string {
  if (process.env.GPT_WEBAI_PRO_STATE_DIR) return path.resolve(process.env.GPT_WEBAI_PRO_STATE_DIR);
  const base = process.env.XDG_STATE_HOME
    ? path.resolve(process.env.XDG_STATE_HOME)
    : path.join(process.env.HOME ?? ".", ".local", "state");
  return path.join(base, "gpt-webai-pro");
}
function defaultConfigPath(): string {
  return path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../config/slots.json");
}
function isValidConversationPointer(value: string): boolean {
  try {
    const baseOrigin = new URL(process.env.GWP_BASE_URL ?? "https://chatgpt.com").origin;
    const url = new URL(value);
    return url.origin === baseOrigin && /^\/c\/[^/?#]+\/?$/.test(url.pathname);
  } catch {
    return false;
  }
}
function isTemporaryConversationPointer(value: string): boolean {
  try {
    return /^\/c\/WEB:/u.test(new URL(value).pathname);
  } catch {
    return false;
  }
}
function validateConfig(config: SlotsConfig): SlotsConfig {
  if (!config || typeof config.image !== "string" || !Array.isArray(config.slots)
    || !Number.isInteger(config.maxConcurrent) || config.maxConcurrent < 1) {
    throw new InputError("invalid slots config");
  }
  const ids = new Set<string>();
  const accounts = new Set<string>();
  const ports = new Set<number>();
  for (const slot of config.slots) {
    if (!/^slot-[a-z0-9]+$/.test(slot.id) || !slot.account || ids.has(slot.id)
      || accounts.has(slot.account)) {
      throw new InputError(`invalid slot config entry: ${JSON.stringify(slot)}`);
    }
    ids.add(slot.id);
    accounts.add(slot.account);
    if (!Number.isInteger(slot.port) || slot.port < 1 || slot.port > 65_535 || ports.has(slot.port)) {
      throw new InputError(`invalid or duplicate port for slot ${slot.id}`);
    }
    ports.add(slot.port);
    if (slot.weeklyLimit !== undefined && !isWeeklyLimit(slot.weeklyLimit)) {
      throw new InputError(`invalid weeklyLimit for slot ${slot.id}`);
    }
  }
  if (config.weeklyLimit !== undefined && !isWeeklyLimit(config.weeklyLimit)) {
    throw new InputError("invalid weeklyLimit");
  }
  return config;
}
function isWeeklyLimit(value: unknown): value is number {
  return Number.isInteger(value) && (value as number) >= 1;
}
/** 확정된 전송을 주간 사용량 원장에 남긴다. 슬롯이 없는 요청(있을 수 없음)은 건너뛴다. */
function recordUsageFor(db: GwpDatabase, requestId: string, modelLabel: string | null, now: number): void {
  const slotId = db.getRequest(requestId)?.slot_id;
  if (slotId) db.recordUsage(requestId, slotId, modelLabel, now);
}
function weeklyStatus(usage: ReturnType<typeof weeklyUsageFor>): {
  weeklyUsed: number;
  weeklyLimit: number | null;
  weeklyResetAt: number | null;
} {
  return { weeklyUsed: usage.used, weeklyLimit: usage.limit, weeklyResetAt: usage.resetAt };
}
async function validateFiles(files: string[]): Promise<ValidatedFile[]> {
  const result: ValidatedFile[] = [];
  const sourceCounts = new Map<string, number>();
  const stagedNames = new Set<string>();
  for (const value of files) {
    const source = path.resolve(value);
    const info = await stat(source).catch(() => null);
    if (!info?.isFile()) throw new InputError(`attachment is not a regular file: ${value}`);
    const basename = path.basename(source);
    let ordinal = (sourceCounts.get(basename) ?? 0) + 1;
    sourceCounts.set(basename, ordinal);
    let name = ordinal === 1 ? basename : suffixBeforeFirstDot(basename, ordinal);
    while (stagedNames.has(name)) {
      ordinal += 1;
      name = suffixBeforeFirstDot(basename, ordinal);
    }
    stagedNames.add(name);
    result.push({ source, name });
  }
  return result;
}
function suffixBeforeFirstDot(filename: string, ordinal: number): string {
  const dot = filename.indexOf(".");
  return dot > 0
    ? `${filename.slice(0, dot)}-${ordinal}${filename.slice(dot)}`
    : `${filename}-${ordinal}`;
}
function throwIfLoginInterrupted(signal: AbortSignal | undefined): void {
  if (signal?.aborted) throw new LoginInterruptedError("login interrupted");
}
async function withLoginAbort<T>(promise: Promise<T>, signal: AbortSignal | undefined): Promise<T> {
  if (!signal) return promise;
  throwIfLoginInterrupted(signal);
  return new Promise<T>((resolve, reject) => {
    const abort = () => reject(new LoginInterruptedError("login interrupted"));
    signal.addEventListener("abort", abort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener("abort", abort);
        resolve(value);
      },
      (error: unknown) => {
        signal.removeEventListener("abort", abort);
        reject(error);
      },
    );
  });
}
async function loginDelay(milliseconds: number, signal: AbortSignal | undefined): Promise<void> {
  if (!signal) {
    await new Promise((resolve) => setTimeout(resolve, milliseconds));
    return;
  }
  throwIfLoginInterrupted(signal);
  await new Promise<void>((resolve, reject) => {
    let timer: NodeJS.Timeout;
    const abort = () => {
      clearTimeout(timer);
      signal.removeEventListener("abort", abort);
      reject(new LoginInterruptedError("login interrupted"));
    };
    const finish = () => {
      signal.removeEventListener("abort", abort);
      resolve();
    };
    timer = setTimeout(finish, milliseconds);
    signal.addEventListener("abort", abort, { once: true });
  });
}
export { defaultConfigPath, defaultStateDir, validateConfig };
