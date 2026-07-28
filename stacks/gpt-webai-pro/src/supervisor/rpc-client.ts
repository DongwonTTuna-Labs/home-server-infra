import { readFile } from "node:fs/promises";
import WebSocket, { type RawData } from "ws";
import {
  DAEMON_ERROR_KINDS,
  GwpError,
  type DaemonErrorKind,
  type SendPhase,
} from "../shared/errors.js";
import type { RpcMethod, RpcMethods } from "../shared/types.js";
interface PendingCall {
  resolve: (value: unknown) => void;
  reject: (reason: unknown) => void;
  timer: NodeJS.Timeout;
}
interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: number;
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
    timeoutMs = 65_000,
  ): Promise<RpcMethods[M]["result"]> {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`RPC ${method} timed out`));
      }, timeoutMs);
      this.pending.set(id, {
        resolve: resolve as (value: unknown) => void,
        reject,
        timer,
      });
      const payload = params === undefined
        ? { jsonrpc: "2.0", id, method }
        : { jsonrpc: "2.0", id, method, params };
      this.socket.send(JSON.stringify(payload), (error) => {
        if (!error) return;
        clearTimeout(timer);
        this.pending.delete(id);
        reject(error);
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
    const pending = this.pending.get(response.id);
    if (!pending) return;
    clearTimeout(pending.timer);
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
  private rejectAll(error: unknown): void {
    for (const call of this.pending.values()) {
      clearTimeout(call.timer);
      call.reject(error);
    }
    this.pending.clear();
  }
}
