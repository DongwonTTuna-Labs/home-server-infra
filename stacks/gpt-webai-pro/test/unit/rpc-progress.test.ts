import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import type { AddressInfo } from "node:net";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { WebSocketServer, type WebSocket } from "ws";
import { SEND_PROGRESS_METHOD, type SendProgress } from "../../src/shared/types.js";
import { RpcClient } from "../../src/supervisor/rpc-client.js";
function progressPayload(callId: number, step: SendProgress["step"], extra: Partial<SendProgress> = {}): string {
  return JSON.stringify({
    jsonrpc: "2.0",
    method: SEND_PROGRESS_METHOD,
    params: {
      callId,
      progress: { step, phase: "pre_click", elapsedMs: 1, stepElapsedMs: 1, ...extra },
    },
  });
}
async function withFakeDaemon(
  behavior: (socket: WebSocket, request: { id: number; method: string }) => void,
  run: (client: RpcClient) => Promise<void>,
): Promise<void> {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-rpc-progress-"));
  const tokenPath = path.join(directory, "daemon.token");
  await writeFile(tokenPath, `${randomBytes(16).toString("hex")}\n`, { mode: 0o600 });
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 });
  await new Promise<void>((resolve) => server.once("listening", resolve));
  server.on("connection", (socket) => {
    socket.on("message", (raw) => {
      const request = JSON.parse(String(raw)) as { id: number; method: string };
      behavior(socket, request);
    });
  });
  const client = await RpcClient.connect((server.address() as AddressInfo).port, tokenPath);
  try {
    await run(client);
  } finally {
    await client.close().catch(() => undefined);
    await new Promise<void>((resolve) => server.close(() => resolve()));
    await rm(directory, { recursive: true, force: true });
  }
}
test("progress notifications keep a slow send alive past the inactivity window", async () => {
  await withFakeDaemon(
    (socket, request) => {
      for (const [at, step] of [[100, "compose"], [250, "click"], [400, "confirm"]] as const) {
        setTimeout(() => socket.send(progressPayload(request.id, step)), at);
      }
      setTimeout(() => socket.send(JSON.stringify({
        jsonrpc: "2.0",
        id: request.id,
        result: { conversationUrl: "https://x/c/1", userTurnId: "u", assistantTurnId: "a" },
      })), 550);
    },
    async (client) => {
      const steps: string[] = [];
      const result = await client.call("send", { prompt: "p", files: [] }, {
        timeoutMs: 5_000,
        inactivityMs: 400,
        onProgress: (progress) => steps.push(progress.step),
      });
      assert.equal(result.userTurnId, "u");
      assert.deepEqual(steps, ["compose", "click", "confirm"]);
    },
  );
});
test("a stalled send rejects on inactivity while anchors stay observable", async () => {
  await withFakeDaemon(
    (socket, request) => {
      socket.send(progressPayload(request.id, "confirm", {
        phase: "post_click",
        pendingUserTurnId: "turn-1",
        pendingConversationUrl: "https://x/c/9",
      }));
      // 이후 침묵 — daemon 행 시뮬레이션.
    },
    async (client) => {
      let lastProgress: SendProgress | undefined;
      await assert.rejects(
        client.call("send", { prompt: "p", files: [] }, {
          timeoutMs: 5_000,
          inactivityMs: 200,
          onProgress: (progress) => { lastProgress = progress; },
        }),
        /no progress for 200ms/,
      );
      assert.equal(lastProgress?.pendingUserTurnId, "turn-1");
      assert.equal(lastProgress?.pendingConversationUrl, "https://x/c/9");
    },
  );
});
test("the absolute cap still bounds a send that keeps reporting progress", async () => {
  await withFakeDaemon(
    (socket, request) => {
      const interval = setInterval(() => {
        if (socket.readyState === socket.OPEN) {
          socket.send(progressPayload(request.id, "confirm"));
        } else {
          clearInterval(interval);
        }
      }, 50);
      socket.once("close", () => clearInterval(interval));
    },
    async (client) => {
      await assert.rejects(
        client.call("send", { prompt: "p", files: [] }, { timeoutMs: 400, inactivityMs: 200 }),
        /RPC send timed out/,
      );
    },
  );
});
