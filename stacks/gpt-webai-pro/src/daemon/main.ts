import { createServer, type Server } from "node:http";
import { readFile } from "node:fs/promises";
import type { AddressInfo } from "node:net";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { WebSocketServer, type RawData, type WebSocket } from "ws";
import { GwpError, errorMessage } from "../shared/errors.js";
import { sha256Text } from "../shared/fsx.js";
import {
  SEND_PROGRESS_METHOD,
  type CloseConversationParams,
  type DownloadParams,
  type InspectParams,
  type LabelConfig,
  type PollParams,
  type ReconcileParams,
  type RpcMethod,
  type SendParams,
  type SendProgress,
  type SendResult,
} from "../shared/types.js";
import { ArtifactDownloader } from "./actions/download.js";
import { inspectConversation } from "./actions/inspect.js";
import { pollConversation } from "./actions/poll.js";
import { reconcileSend } from "./actions/reconcile.js";
import { sendMessage } from "./actions/send.js";
import { BrowserSession } from "./browser.js";
import { readinessObservation } from "./selectors.js";
interface JsonRpcRequest {
  jsonrpc: "2.0";
  id: number;
  method: RpcMethod;
  params?: unknown;
}
type EnqueueMutation = <T>(operation: () => Promise<T>) => Promise<T>;
// stderr JSON 라인 = docker logs. 이전엔 daemon이 아무것도 남기지 않아 send가 어디서
// 시간을 쓰는지 supervisor 밖에서는 알 수 없었다 (2026-07-29 send 스톨 진단의 교훈).
export function dlog(event: string, fields: Record<string, unknown> = {}): void {
  process.stderr.write(`${JSON.stringify({ at: Date.now(), event, ...fields })}\n`);
}
// 완료된 send의 진실은 daemon 메모리에 캐시한다. supervisor가 대기를 포기(타임아웃/
// 소켓 단절)한 뒤 daemon이 send를 완결한 경우, 같은 컨테이너가 살아있는 한 reconcile이
// 탭 상태와 무관하게 이 캐시로 결정적으로 회수한다 (§5.3 A0).
const SEND_CACHE_LIMIT = 16;
export class SendResultCache {
  private readonly entries = new Map<string, SendResult>();
  private readonly attempts = new Map<string, number>();
  // 같은 promptSha로 두 번째 send가 "시작"되는 순간부터 이 sha는 영구 모호하다 —
  // 캐시를 지우고 다시는 채우지 않는다. reconcile은 탭 스캔의 보수적 모호성 처리로
  // 떨어진다 (동일 텍스트 요청 2건이 서로의 대화에 바인딩되는 오류 방지).
  beginAttempt(promptSha256: string): void {
    const count = (this.attempts.get(promptSha256) ?? 0) + 1;
    this.attempts.set(promptSha256, count);
    if (count > 1) this.entries.delete(promptSha256);
    while (this.attempts.size > SEND_CACHE_LIMIT * 4) {
      const oldest = this.attempts.keys().next().value;
      if (oldest === undefined) break;
      this.attempts.delete(oldest);
      this.entries.delete(oldest);
    }
  }
  set(promptSha256: string, result: SendResult): void {
    if ((this.attempts.get(promptSha256) ?? 0) !== 1) return;
    this.entries.delete(promptSha256);
    this.entries.set(promptSha256, result);
    while (this.entries.size > SEND_CACHE_LIMIT) {
      const oldest = this.entries.keys().next().value;
      if (oldest === undefined) break;
      this.entries.delete(oldest);
    }
  }
  get(promptSha256: string): SendResult | undefined {
    return this.entries.get(promptSha256);
  }
}
// supervisor의 requests.prompt_sha256 계산(sha256Text(prompt.trim()))과 반드시 일치해야 한다.
export function promptCacheKey(prompt: string): string {
  return sha256Text(prompt.trim());
}
export interface DaemonHandle {
  port: number;
  close(): Promise<void>;
}
export async function startDaemon(options: {
  session: BrowserSession;
  port: number;
  token: string;
  outboxDir: string;
  labels: LabelConfig;
}): Promise<DaemonHandle> {
  if (!Number.isInteger(options.port) || options.port < 0 || options.port > 65_535) {
    throw new Error("daemon port must be an integer between 0 and 65535");
  }
  if (!/^[0-9a-f]{32}$/.test(options.token)) {
    throw new Error("GWP_DAEMON_TOKEN must be exactly 32 lower-hex characters");
  }
  const server = createServer((_request, response) => {
    response.writeHead(426).end();
  });
  const webSockets = new WebSocketServer({ noServer: true });
  server.on("upgrade", (request, socket, head) => {
    if (request.headers.authorization !== `Bearer ${options.token}`) {
      socket.write("HTTP/1.1 401 Unauthorized\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
      socket.destroy();
      return;
    }
    webSockets.handleUpgrade(request, socket, head, (client) => {
      webSockets.emit("connection", client, request);
    });
  });
  const downloader = new ArtifactDownloader(options.outboxDir);
  const enqueueMutation = createMutationQueue();
  const sendCache = new SendResultCache();
  webSockets.on("connection", (socket) => {
    socket.on("message", (data) => {
      void handleMessage(socket, data, options, downloader, enqueueMutation, sendCache);
    });
  });
  const port = await listen(server, options.port);
  return {
    port,
    async close() {
      for (const client of webSockets.clients) client.terminate();
      await new Promise<void>((resolve) => webSockets.close(() => resolve()));
      await new Promise<void>((resolve) => server.close(() => resolve()));
    },
  };
}
async function handleMessage(
  socket: WebSocket,
  raw: RawData,
  options: {
    session: BrowserSession;
    labels: LabelConfig;
    outboxDir: string;
  },
  downloader: ArtifactDownloader,
  enqueueMutation: EnqueueMutation,
  sendCache: SendResultCache,
): Promise<void> {
  let request: JsonRpcRequest | undefined;
  const startedAt = Date.now();
  try {
    request = JSON.parse(raw.toString()) as JsonRpcRequest;
    if (request.jsonrpc !== "2.0" || !Number.isInteger(request.id)) {
      throw new Error("invalid JSON-RPC request");
    }
    const result = await dispatch(request, options, downloader, enqueueMutation, socket, sendCache);
    dlog("rpc", { method: request.method, ms: Date.now() - startedAt, ok: true });
    socket.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, result }));
  } catch (error) {
    const gwp = error instanceof GwpError
      ? error
      : new GwpError("internal", errorMessage(error), { cause: error });
    dlog("rpc", {
      method: request?.method ?? "invalid",
      ms: Date.now() - startedAt,
      ok: false,
      kind: gwp.kind,
      detail: gwp.detail,
    });
    const phase = request?.method === "send"
      ? gwp.phase ?? "pre_click"
      : gwp.phase;
    socket.send(JSON.stringify({
      jsonrpc: "2.0",
      id: request?.id ?? null,
      error: {
        code: -32000,
        message: gwp.message,
        data: {
          kind: gwp.kind,
          ...(phase ? { phase } : {}),
          detail: gwp.detail,
          ...(gwp.pendingUserTurnId
            ? { pendingUserTurnId: gwp.pendingUserTurnId }
            : {}),
          ...(gwp.pendingConversationUrl
            ? { pendingConversationUrl: gwp.pendingConversationUrl }
            : {}),
          ...(gwp.preClickBaseline
            ? { preClickBaseline: gwp.preClickBaseline }
            : {}),
        },
      },
    }));
  }
}
async function dispatch(
  request: JsonRpcRequest,
  options: {
    session: BrowserSession;
    labels: LabelConfig;
    outboxDir: string;
  },
  downloader: ArtifactDownloader,
  enqueueMutation: EnqueueMutation,
  socket: WebSocket,
  sendCache: SendResultCache,
): Promise<unknown> {
  switch (request.method) {
    case "health": {
      const page = await options.session.inspectionPage();
      return {
        ok: options.session.connected(),
        chromeConnected: options.session.connected(),
        currentUrl: page?.url() ?? "",
      };
    }
    case "readiness": {
      const page = await options.session.inspectionPage();
      return page
        ? readinessObservation(page, options.labels.intelligence)
        : { state: "unknown", modelLabel: "" };
    }
    case "send": {
      const params = request.params as SendParams;
      return enqueueMutation(async () => {
        sendCache.beginAttempt(promptCacheKey(params.prompt));
        let lastStep = "";
        const result = await sendMessage(options.session, params, options.labels, (progress) => {
          if (progress.step !== lastStep) {
            lastStep = progress.step;
            dlog("send_step", {
              step: progress.step,
              phase: progress.phase,
              elapsedMs: progress.elapsedMs,
              ...(progress.pendingUserTurnId ? { pendingUserTurnId: progress.pendingUserTurnId } : {}),
              ...(progress.matchDebug ? { matchDebug: progress.matchDebug } : {}),
            });
          }
          notifySendProgress(socket, request.id, progress);
        }, options.outboxDir);
        sendCache.set(promptCacheKey(params.prompt), result);
        return result;
      });
    }
    case "reconcile": {
      const params = request.params as ReconcileParams;
      // The whole verdict stays behind the mutation queue: a URL-less reconcile must
      // never inspect the new tab between send's navigation/click and confirmed result.
      // 큐 순서가 곧 정합성이다: 진행 중 send가 있으면 그 send가 끝난 뒤에야 이 클로저가
      // 돌므로, 캐시 조회는 "완결된 send의 진실"만 본다.
      return enqueueMutation(async () => {
        const cached = sendCache.get(params.promptSha256);
        if (cached) {
          dlog("reconcile_cache_hit", { userTurnId: cached.userTurnId });
          return {
            found: true,
            conversationUrl: cached.conversationUrl,
            userTurnId: cached.userTurnId,
            assistantTurnId: cached.assistantTurnId,
            proven: true,
            matchedBy: "cache",
          };
        }
        const verdict = await reconcileSend(options.session, params);
        dlog("reconcile", {
          found: verdict.found,
          proven: verdict.proven,
          ...(verdict.matchedBy ? { matchedBy: verdict.matchedBy } : {}),
          ...(verdict.evidence ? { evidence: verdict.evidence } : {}),
        });
        return verdict;
      });
    }
    case "poll":
      return pollConversation(
        options.session,
        request.params as PollParams,
        (conversationUrl) => enqueueMutation(() => options.session.open(conversationUrl)),
      );
    case "download":
      return enqueueMutation(() => (
        downloader.download(options.session, request.params as DownloadParams)
      ));
    case "inspect":
      return enqueueMutation(() => inspectConversation(
        options.session, request.params as InspectParams, options.outboxDir,
      ));
    case "closeConversation": {
      await enqueueMutation(() => (
        options.session.closeConversation(
          (request.params as CloseConversationParams).conversationUrl,
        )
      ));
      return { ok: true };
    }
    default:
      throw new GwpError("internal", `unknown RPC method: ${String(request.method)}`);
  }
}
function notifySendProgress(socket: WebSocket, callId: number, progress: SendProgress): void {
  if (socket.readyState !== socket.OPEN) return;
  socket.send(JSON.stringify({
    jsonrpc: "2.0",
    method: SEND_PROGRESS_METHOD,
    params: { callId, progress },
  }));
}
function createMutationQueue(): EnqueueMutation {
  let tail: Promise<void> = Promise.resolve();
  return <T>(operation: () => Promise<T>): Promise<T> => {
    const result = tail.then(operation);
    tail = result.then(() => undefined, () => undefined);
    return result;
  };
}
function listen(server: Server, port: number): Promise<number> {
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "0.0.0.0", () => {
      server.off("error", reject);
      resolve((server.address() as AddressInfo).port);
    });
  });
}
async function main(): Promise<void> {
  const sourceRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
  const labelsPath = process.env.GWP_LABELS_PATH ?? path.join(sourceRoot, "config", "labels.json");
  const labels = JSON.parse(await readFile(labelsPath, "utf8")) as LabelConfig;
  const port = Number(process.env.GWP_DAEMON_PORT);
  if (!Number.isInteger(port) || port < 1 || port > 65_535) {
    throw new Error("GWP_DAEMON_PORT must be an integer between 1 and 65535");
  }
  const session = await BrowserSession.connect(
    process.env.GWP_CDP_URL ?? "http://127.0.0.1:9222",
    process.env.GWP_BASE_URL ?? "https://chatgpt.com",
  );
  const handle = await startDaemon({
    session,
    port,
    token: process.env.GWP_DAEMON_TOKEN ?? "",
    outboxDir: process.env.GWP_OUTBOX_DIR ?? "/outbox",
    labels,
  });
  const shutdown = async () => {
    // Chrome을 먼저 CDP Browser.close로 클린 종료(쿠키 flush) — browser.ts 주석 참조.
    await session.closeBrowserGracefully().catch(() => undefined);
    await handle.close();
    process.exit(0);
  };
  process.once("SIGTERM", () => void shutdown());
  process.once("SIGINT", () => void shutdown());
}
if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  main().catch((error) => {
    process.stderr.write(`${errorMessage(error)}\n`);
    process.exitCode = 1;
  });
}
