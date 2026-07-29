import { readFile } from "node:fs/promises";
import WebSocket, { type RawData } from "ws";
import {
  DAEMON_ERROR_KINDS,
  GwpError,
  type DaemonErrorKind,
  type SendPhase,
} from "../shared/errors.js";
import { SEND_PROGRESS_METHOD, type RpcMethod, type RpcMethods, type SendProgress } from "../shared/types.js";
export interface CallOptions {
  // 절대 상한. 이 시간 안에 결과가 없으면 실패한다.
  timeoutMs?: number;
  // 무진행 상한: daemon의 progress 알림이 이 간격 안에 계속 오는 한 timeoutMs까지
  // 기다린다. 알림이 끊기면(daemon 사망/행) 조기에 실패한다. send처럼 소요 시간이
  // 페이지 상태에 좌우되는 호출에 쓴다.
  inactivityMs?: number;
  onProgress?: (progress: SendProgress) => void;
}
interface PendingCall {
  resolve: (value: unknown) => void;
  reject: (reason: unknown) => void;
  timer: NodeJS.Timeout;
  inactivityMs?: number;
  inactivityTimer?: NodeJS.Timeout;
  onProgress?: (progress: SendProgress) => void;
}
interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: number;
  method?: string;
  params?: { callId?: number; progress?: SendProgress };
  result?: unknown;
  error?: {
    code: number;
    message: string;
    data?: {
      kind?: string;
      phase?: SendPhase;
      detail?: string;
      pendingUserTurnId?: string;
      pendingConversationUrl?: string;
      preClickBaseline?: string[];
    };
  };
}
export class RpcClient {
  private nextId = 1;
  private readonly pending = new Map<number, PendingCall>();
  private constructor(private readonly socket: WebSocket) {
    socket.on("message", (data) => this.onMessage(data));
    socket.on("close", () => this.rejectAll(new Error("daemon connection closed")));
    socket.on("error", (error) => this.rejectAll(error));
  }
  static async connect(port: number, tokenPath: string, timeoutMs = 10_000): Promise<RpcClient> {
    const token = (await readFile(tokenPath, "utf8")).trim();
    if (!/^[0-9a-f]{32}$/.test(token)) {
      throw new Error("daemon token file is missing a valid 32-hex token");
    }
    const endpoint = `ws://127.0.0.1:${port}/`;
    const socket = new WebSocket(endpoint, {
      headers: { Authorization: `Bearer ${token}` },
    });
    await new Promise<void>((resolve, reject) => {
      let settled = false;
      const timer = setTimeout(() => {
        if (settled) return;
        settled = true;
        socket.terminate();
        reject(new Error(`daemon connection timed out on 127.0.0.1:${port}`));
      }, timeoutMs);
      socket.once("open", () => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        resolve();
      });
      socket.once("error", (error) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        reject(error);
      });
      socket.once("unexpected-response", (_request, response) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        response.resume();
        socket.terminate();
        reject(new Error(`daemon rejected WebSocket handshake with HTTP ${response.statusCode}`));
      });
    });
    return new RpcClient(socket);
  }
  call<M extends RpcMethod>(
    method: M,
    params: RpcMethods[M]["params"],
    options: number | CallOptions = 65_000,
  ): Promise<RpcMethods[M]["result"]> {
    const resolved = typeof options === "number" ? { timeoutMs: options } : options;
    const timeoutMs = resolved.timeoutMs ?? 65_000;
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const settle = (reason: Error) => {
        const pending = this.pending.get(id);
        if (!pending) return;
        clearTimeout(pending.timer);
        if (pending.inactivityTimer) clearTimeout(pending.inactivityTimer);
        this.pending.delete(id);
        reject(reason);
      };
      const timer = setTimeout(() => {
        settle(new Error(`RPC ${method} timed out`));
      }, timeoutMs);
      const entry: PendingCall = {
        resolve: resolve as (value: unknown) => void,
        reject,
        timer,
        ...(resolved.inactivityMs ? { inactivityMs: resolved.inactivityMs } : {}),
        ...(resolved.onProgress ? { onProgress: resolved.onProgress } : {}),
      };
      if (resolved.inactivityMs) {
        entry.inactivityTimer = setTimeout(() => {
          settle(new Error(
            `RPC ${method} made no progress for ${resolved.inactivityMs}ms`,
          ));
        }, resolved.inactivityMs);
      }
      this.pending.set(id, entry);
      const payload = params === undefined
        ? { jsonrpc: "2.0", id, method }
        : { jsonrpc: "2.0", id, method, params };
      this.socket.send(JSON.stringify(payload), (error) => {
        if (!error) return;
        settle(error instanceof Error ? error : new Error(String(error)));
      });
    });
  }
  async close(): Promise<void> {
    if (this.socket.readyState === WebSocket.CLOSED) return;
    await new Promise<void>((resolve) => {
      this.socket.once("close", () => resolve());
      this.socket.close();
      setTimeout(() => {
        if (this.socket.readyState !== WebSocket.CLOSED) this.socket.terminate();
        resolve();
      }, 1_000).unref();
    });
  }
  private onMessage(data: RawData): void {
    let response: JsonRpcResponse;
    try {
      response = JSON.parse(data.toString()) as JsonRpcResponse;
    } catch {
      this.rejectAll(new Error("daemon returned invalid JSON"));
      return;
    }
    if (response.id === undefined || response.id === null) {
      if (response.method === SEND_PROGRESS_METHOD) this.onProgressNotification(response);
      return;
    }
    const pending = this.pending.get(response.id);
    if (!pending) return;
    clearTimeout(pending.timer);
    if (pending.inactivityTimer) clearTimeout(pending.inactivityTimer);
    this.pending.delete(response.id);
    if (response.error) {
      const rawKind = response.error.data?.kind ?? "internal";
      const kind = DAEMON_ERROR_KINDS.includes(rawKind as DaemonErrorKind)
        ? rawKind as DaemonErrorKind
        : "internal";
      const detail = response.error.data?.detail ?? response.error.message;
      const phase = response.error.data?.phase;
      const pendingUserTurnId = response.error.data?.pendingUserTurnId;
      const pendingConversationUrl = response.error.data?.pendingConversationUrl;
      const preClickBaseline = response.error.data?.preClickBaseline;
      pending.reject(new GwpError(kind, detail, {
        ...(phase === "pre_click" || phase === "post_click" ? { phase } : {}),
        ...(typeof pendingUserTurnId === "string" && pendingUserTurnId
          ? { pendingUserTurnId }
          : {}),
        ...(typeof pendingConversationUrl === "string" ? { pendingConversationUrl } : {}),
        ...(Array.isArray(preClickBaseline)
          && preClickBaseline.every((item) => typeof item === "string")
          ? { preClickBaseline }
          : {}),
      }));
      return;
    }
    pending.resolve(response.result);
  }
  private onProgressNotification(notification: JsonRpcResponse): void {
    const callId = notification.params?.callId;
    const progress = notification.params?.progress;
    if (typeof callId !== "number" || !progress) return;
    const pending = this.pending.get(callId);
    if (!pending) return;
    if (pending.inactivityTimer && pending.inactivityMs) {
      clearTimeout(pending.inactivityTimer);
      pending.inactivityTimer = setTimeout(() => {
        clearTimeout(pending.timer);
        this.pending.delete(callId);
        pending.reject(new Error(
          `RPC send made no progress for ${pending.inactivityMs}ms`,
        ));
      }, pending.inactivityMs);
    }
    try {
      pending.onProgress?.(progress);
    } catch {
      // A progress observer must never break the call itself.
    }
  }
  private rejectAll(error: unknown): void {
    for (const call of this.pending.values()) {
      clearTimeout(call.timer);
      if (call.inactivityTimer) clearTimeout(call.inactivityTimer);
      call.reject(error);
    }
    this.pending.clear();
  }
}
