import { randomBytes } from "node:crypto";
import { createServer, type Server } from "node:http";
import { readFile } from "node:fs/promises";
import type { AddressInfo } from "node:net";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import type { Page } from "playwright-core";
import { WebSocketServer, type RawData, type WebSocket } from "ws";
import { GwpError, errorMessage } from "../shared/errors.js";
import { atomicWrite, mkdirp } from "../shared/fsx.js";
import type {
  CaptureFailureParams,
  CloseConversationParams,
  DownloadParams,
  LabelConfig,
  OpenParams,
  PollParams,
  ReconcileParams,
  RpcMethod,
  SendParams,
} from "../shared/types.js";
import { ArtifactDownloader } from "./actions/download.js";
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
  webSockets.on("connection", (socket) => {
    socket.on("message", (data) => {
      void handleMessage(socket, data, options, downloader, enqueueMutation);
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
    outboxDir: string;
    labels: LabelConfig;
  },
  downloader: ArtifactDownloader,
  enqueueMutation: EnqueueMutation,
): Promise<void> {
  let request: JsonRpcRequest | undefined;
  try {
    request = JSON.parse(raw.toString()) as JsonRpcRequest;
    if (request.jsonrpc !== "2.0" || !Number.isInteger(request.id)) {
      throw new Error("invalid JSON-RPC request");
    }
    const result = await dispatch(request, options, downloader, enqueueMutation);
    socket.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, result }));
  } catch (error) {
    const gwp = error instanceof GwpError
      ? error
      : new GwpError("internal", errorMessage(error), { cause: error });
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
    outboxDir: string;
    labels: LabelConfig;
  },
  downloader: ArtifactDownloader,
  enqueueMutation: EnqueueMutation,
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
    case "send":
      return enqueueMutation(() => (
        sendMessage(options.session, request.params as SendParams, options.labels)
      ));
    case "reconcile":
      // The whole verdict stays behind the mutation queue: a URL-less reconcile must
      // never inspect the new tab between send's navigation/click and confirmed result.
      return enqueueMutation(() => (
        reconcileSend(options.session, request.params as ReconcileParams)
      ));
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
    case "open": {
      await enqueueMutation(() => (
        options.session.open((request.params as OpenParams).conversationUrl)
      ));
      return { ok: true };
    }
    case "closeConversation": {
      await enqueueMutation(() => (
        options.session.closeConversation(
          (request.params as CloseConversationParams).conversationUrl,
        )
      ));
      return { ok: true };
    }
    case "captureFailure": {
      const page = await options.session.inspectionPage();
      if (!page) throw new GwpError("internal", "no browser page is available to capture");
      return captureFailure(page, options.outboxDir, request.params as CaptureFailureParams);
    }
    default:
      throw new GwpError("internal", `unknown RPC method: ${String(request.method)}`);
  }
}
async function captureFailure(
  page: Page,
  outboxDir: string,
  params: CaptureFailureParams,
): Promise<{ screenshotPath: string; htmlPath: string }> {
  const tag = params.tag.replace(/[^a-z0-9._-]+/giu, "-").slice(0, 80) || "failure";
  const stamp = `${Date.now()}-${process.pid}-${randomBytes(6).toString("hex")}`;
  await mkdirp(outboxDir);
  const screenshotPath = path.join(outboxDir, `${stamp}-${tag}.png`);
  const htmlPath = path.join(outboxDir, `${stamp}-${tag}.html`);
  await page.screenshot({ path: screenshotPath, fullPage: true });
  await atomicWrite(htmlPath, await page.content());
  return { screenshotPath, htmlPath };
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
