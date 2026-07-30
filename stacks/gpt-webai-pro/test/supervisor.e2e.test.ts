import assert from "node:assert/strict";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomBytes } from "node:crypto";
import { createServer, type Server } from "node:http";
import { mkdir, mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import type { AddressInfo } from "node:net";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import test, { type TestContext } from "node:test";
import { WebSocketServer, type WebSocket } from "ws";
import { sha256File, sha256Text } from "../src/shared/fsx.js";
import type { SlotConfig } from "../src/shared/types.js";
import { CONTAINER_OUTBOX } from "../src/supervisor/docker.js";
import { InputError, LoginInterruptedError, LoginTimeoutError, Supervisor } from "../src/supervisor/run.js";
import { markSlotNeedsLogin, markSlotProviderLimit } from "../src/supervisor/slots.js";
const DROP = Symbol("drop connection");
class MockRpcError extends Error {
  constructor(
    readonly kind: string,
    message: string,
    readonly phase?: "pre_click" | "post_click",
    readonly pendingConversationUrl?: string,
    readonly preClickBaseline?: string[],
    readonly pendingUserTurnId?: string,
  ) {
    super(message);
  }
}
type Handler = (
  method: string,
  params: Record<string, unknown> | undefined,
  socket: WebSocket,
) => unknown | typeof DROP | Promise<unknown | typeof DROP>;
class MockDaemon {
  private constructor(
    readonly port: number,
    private readonly server: Server,
    private readonly webSockets: WebSocketServer,
    private readonly metrics: { healthCalls: number },
  ) {}
  get healthCalls(): number {
    return this.metrics.healthCalls;
  }
  static async start(token: string, handler: Handler): Promise<MockDaemon> {
    const server = createServer();
    const metrics = { healthCalls: 0 };
    const webSockets = new WebSocketServer({
      server,
      verifyClient(info, done) {
        if (info.req.headers.authorization === `Bearer ${token}`) done(true);
        else done(false, 401, "Unauthorized");
      },
    });
    let rpcQueue: Promise<void> = Promise.resolve();
    webSockets.on("connection", (socket) => {
      socket.on("message", (raw) => {
        const task = rpcQueue.then(async () => {
          const request = JSON.parse(raw.toString()) as {
            id: number;
            method: string;
            params?: Record<string, unknown>;
          };
          try {
            let result: unknown;
            if (request.method === "health") {
              metrics.healthCalls += 1;
              result = { ok: true, chromeConnected: true, currentUrl: "https://chatgpt.com/" };
            } else {
              result = await handler(request.method, request.params, socket);
            }
            if (result === DROP) {
              socket.terminate();
              return;
            }
            socket.send(JSON.stringify({ jsonrpc: "2.0", id: request.id, result }));
          } catch (error) {
            const rpc = error instanceof MockRpcError
              ? error
              : new MockRpcError("internal", String(error));
            socket.send(JSON.stringify({
              jsonrpc: "2.0",
              id: request.id,
              error: {
                code: -32000,
                message: rpc.message,
                data: {
                  kind: rpc.kind,
                  ...(rpc.phase ? { phase: rpc.phase } : {}),
                  ...(rpc.pendingUserTurnId
                    ? { pendingUserTurnId: rpc.pendingUserTurnId }
                    : {}),
                  ...(rpc.pendingConversationUrl
                    ? { pendingConversationUrl: rpc.pendingConversationUrl }
                    : {}),
                  ...(rpc.preClickBaseline
                    ? { preClickBaseline: rpc.preClickBaseline }
                    : {}),
                  detail: rpc.message,
                },
              },
            }));
          }
        });
        rpcQueue = task.catch(() => undefined);
      });
    });
    await new Promise<void>((resolve, reject) => {
      server.once("error", reject);
      server.listen(0, "127.0.0.1", () => resolve());
    });
    return new MockDaemon((server.address() as AddressInfo).port, server, webSockets, metrics);
  }
  async close(): Promise<void> {
    for (const socket of this.webSockets.clients) socket.terminate();
    await new Promise<void>((resolve) => this.webSockets.close(() => resolve()));
    await new Promise<void>((resolve) => this.server.close(() => resolve()));
  }
}
async function fixture(
  t: TestContext,
  definitions: Array<{ id: string; account: string; handler: Handler }>,
  options: { maxConcurrent?: number } = {},
): Promise<{ supervisor: Supervisor; directory: string; daemons: MockDaemon[] }> {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-supervisor-"));
  const daemons: MockDaemon[] = [];
  const slots: SlotConfig[] = [];
  for (const definition of definitions) {
    const token = randomBytes(16).toString("hex");
    const tokenPath = path.join(directory, "state", "slots", definition.id, "daemon.token");
    await mkdir(path.dirname(tokenPath), { recursive: true });
    await writeFile(tokenPath, `${token}\n`, { mode: 0o600 });
    const daemon = await MockDaemon.start(token, definition.handler);
    daemons.push(daemon);
    slots.push({
      id: definition.id,
      account: definition.account,
      port: daemon.port,
      unmanaged: true,
    });
  }
  const configPath = path.join(directory, "slots.json");
  await writeFile(configPath, JSON.stringify({
    image: "unused-in-tests",
    maxConcurrent: options.maxConcurrent ?? 3,
    slots,
  }));
  const supervisor = await Supervisor.open({
    stateDir: path.join(directory, "state"),
    configPath,
  });
  t.after(async () => {
    supervisor.close();
    await Promise.all(daemons.map((daemon) => daemon.close()));
    await rm(directory, { recursive: true, force: true });
  });
  return { supervisor, directory, daemons };
}
function complete(answer = "mock answer", currentUrl = "https://chatgpt.com/c/mock") {
  return {
    state: "complete",
    currentUrl,
    answerMarkdown: answer,
    answerSha256: sha256Text(answer),
    artifactControls: [],
  };
}
function standardHandler(overrides: Handler): Handler {
  return async (method, params, socket) => {
    if (method === "readiness") return { state: "ready", modelLabel: "Pro" };
    if (method === "poll") return complete("mock answer", pollUrl(params));
    if (method === "closeConversation") return { ok: true };
    return overrides(method, params, socket);
  };
}
function pollUrl(params: Record<string, unknown> | undefined): string {
  const value = params?.conversationUrl;
  if (typeof value !== "string") throw new Error("mock poll is missing conversationUrl");
  return value;
}
function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T | PromiseLike<T>) => void;
  reject: (reason?: unknown) => void;
} {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}
async function seedStagedRequest(
  supervisor: Supervisor,
  directory: string,
  id: string,
  prompt: string,
): Promise<void> {
  const requestDir = path.join(directory, "state", "requests", id);
  await mkdir(path.join(requestDir, "attachments"), { recursive: true });
  await writeFile(path.join(requestDir, "prompt.md"), prompt);
  supervisor.db.createRequest(id, sha256Text(prompt));
}
function spawnResume(directory: string, id: string): {
  child: ChildProcessWithoutNullStreams;
  completion: Promise<{
    code: number | null;
    signal: NodeJS.Signals | null;
    stdout: string;
    stderr: string;
  }>;
} {
  const supervisorUrl = pathToFileURL(path.resolve("src/supervisor/run.ts")).href;
  const source = `
    import { Supervisor } from ${JSON.stringify(supervisorUrl)};
    const supervisor = await Supervisor.open({
      stateDir: ${JSON.stringify(path.join(directory, "state"))},
      configPath: ${JSON.stringify(path.join(directory, "slots.json"))}
    });
    try {
      const result = await supervisor.resume(${JSON.stringify(id)}, 10);
      process.stdout.write(JSON.stringify(result) + "\\n");
    } finally {
      supervisor.close();
    }
  `;
  const child = spawn(
    process.execPath,
    ["--import", "tsx", "--input-type=module", "--eval", source],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk: string) => { stdout += chunk; });
  child.stderr.on("data", (chunk: string) => { stderr += chunk; });
  const completion = new Promise<{
    code: number | null;
    signal: NodeJS.Signals | null;
    stdout: string;
    stderr: string;
  }>((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code, signal) => resolve({ code, signal, stdout, stderr }));
  });
  return { child, completion };
}
test("live send owner makes concurrent resume return running without changing state", async (t) => {
  const sendStarted = deferred<void>();
  const sendResult = deferred<{
    conversationUrl: string;
    userTurnId: string;
    assistantTurnId: string;
  }>();
  let sendCalls = 0;
  let reconcileCalls = 0;
  const { supervisor, directory } = await fixture(t, [{
    id: "slot-01",
    account: "a",
    handler: async (method, params) => {
      if (method === "readiness") return { state: "ready", modelLabel: "Pro" };
      if (method === "send") {
        sendCalls += 1;
        sendStarted.resolve();
        return sendResult.promise;
      }
      if (method === "reconcile") {
        reconcileCalls += 1;
        throw new Error("concurrent resume must not reconcile a live sender");
      }
      if (method === "poll") return complete("owner answer", pollUrl(params));
      throw new Error(`unexpected ${method}`);
    },
  }]);
  const id = "req_5000000000000001";
  await seedStagedRequest(supervisor, directory, id, "live owner");
  const owner = spawnResume(directory, id);
  t.after(() => {
    if (owner.child.exitCode === null && owner.child.signalCode === null) owner.child.kill("SIGKILL");
  });
  await sendStarted.promise;
  const requestBefore = supervisor.db.getRequest(id);
  const attemptBefore = supervisor.db.latestAttempt(id);
  const slotsBefore = supervisor.db.listSlots();
  const concurrent = await supervisor.resume(id, 2);
  assert.equal(concurrent.status, "running");
  assert.equal(concurrent.message, "전송 진행 중(소유 프로세스 생존)");
  assert.match(concurrent.resumeCommand ?? "", new RegExp(id));
  assert.deepEqual(supervisor.db.getRequest(id), requestBefore);
  assert.deepEqual(supervisor.db.latestAttempt(id), attemptBefore);
  assert.deepEqual(supervisor.db.listSlots(), slotsBefore);
  assert.equal(sendCalls, 1);
  assert.equal(reconcileCalls, 0);
  const concurrentRelease = await supervisor.release(id);
  assert.equal(concurrentRelease.status, "running");
  assert.equal(concurrentRelease.message, "전송 진행 중(소유 프로세스 생존)");
  assert.deepEqual(supervisor.db.getRequest(id), requestBefore);
  assert.deepEqual(supervisor.db.latestAttempt(id), attemptBefore);
  sendResult.resolve({
    conversationUrl: "https://chatgpt.com/c/live-owner",
    userTurnId: "user-live-owner",
    assistantTurnId: "assistant-live-owner",
  });
  const finished = await owner.completion;
  assert.equal(finished.code, 0, finished.stderr);
  assert.equal(JSON.parse(finished.stdout.trim()).status, "complete");
  assert.equal(supervisor.db.latestAttempt(id)?.state, "confirmed");
  assert.equal(sendCalls, 1);
});
test("SIGKILL releases send flock and resume reconciles the orphaned armed attempt", async (t) => {
  const sendStarted = deferred<void>();
  const sendResult = deferred<{
    conversationUrl: string;
    userTurnId: string;
    assistantTurnId: string;
  }>();
  let sendCalls = 0;
  let reconcileCalls = 0;
  const { supervisor, directory } = await fixture(t, [{
    id: "slot-01",
    account: "a",
    handler: async (method, params) => {
      if (method === "readiness") return { state: "ready", modelLabel: "Pro" };
      if (method === "send") {
        sendCalls += 1;
        sendStarted.resolve();
        return sendResult.promise;
      }
      if (method === "reconcile") {
        reconcileCalls += 1;
        return {
          found: true,
          proven: true,
          conversationUrl: "https://chatgpt.com/c/orphan-reconciled",
          userTurnId: "user-orphan",
          assistantTurnId: "assistant-orphan",
        };
      }
      if (method === "poll") return complete("recovered answer", pollUrl(params));
      throw new Error(`unexpected ${method}`);
    },
  }]);
  const id = "req_5000000000000002";
  await seedStagedRequest(supervisor, directory, id, "orphan owner");
  const owner = spawnResume(directory, id);
  await sendStarted.promise;
  assert.equal(supervisor.db.getRequest(id)?.status, "sending");
  assert.equal(supervisor.db.latestAttempt(id)?.state, "armed");
  assert.equal(owner.child.kill("SIGKILL"), true);
  const killed = await owner.completion;
  assert.equal(killed.signal, "SIGKILL");
  sendResult.resolve({
    conversationUrl: "https://chatgpt.com/c/lost-rpc-response",
    userTurnId: "lost-user",
    assistantTurnId: "lost-assistant",
  });
  const recovered = await supervisor.resume(id, 2);
  assert.equal(recovered.status, "complete");
  assert.equal(recovered.answer, "recovered answer");
  assert.equal(sendCalls, 1);
  assert.equal(reconcileCalls, 1);
  assert.equal(supervisor.db.latestAttempt(id)?.state, "reconciled");
  assert.equal(supervisor.db.getRequest(id)?.conversation_url, "https://chatgpt.com/c/orphan-reconciled");
});
test("send WebSocket loss -> uncertain -> reconcile found -> complete without re-click", async (t) => {
  let sendCalls = 0;
  const { supervisor } = await fixture(t, [{
    id: "slot-01",
    account: "a",
    handler: standardHandler((method) => {
      if (method === "send") {
        sendCalls += 1;
        return DROP;
      }
      if (method === "reconcile") {
        return {
          found: true,
          proven: true,
          conversationUrl: "https://chatgpt.com/c/reconciled",
          userTurnId: "user-reconciled",
          assistantTurnId: "assistant-reconciled",
        };
      }
      throw new Error(`unexpected ${method}`);
    }),
  }]);
  const first = await supervisor.run("socket loss", [], 2);
  assert.equal(first.status, "needs_user_action");
  assert.equal(first.errorKind, "send_uncertain");
  const resumed = await supervisor.resume(first.sessionId!, 2);
  assert.equal(resumed.status, "complete");
  assert.equal(sendCalls, 1);
  assert.equal(supervisor.db.latestAttempt(first.sessionId!)?.state, "reconciled");
});
test("post-click confirmation miss persists its pending tab and reconciles without re-click", async (t) => {
  const pendingUrl = "https://chatgpt.com/c/WEB:landed-before-confirmation";
  const pendingUserTurnId = "user-landed-before-confirmation";
  const baseline = ["existing-user", "existing-assistant"];
  let sendCalls = 0;
  let reconcileCalls = 0;
  const { supervisor } = await fixture(t, [{
    id: "slot-01",
    account: "a",
    handler: standardHandler((method, params) => {
      if (method === "send") {
        sendCalls += 1;
        throw new MockRpcError(
          "click_uncertain",
          "turn confirmation window expired after the send landed",
          "post_click",
          pendingUrl,
          baseline,
          pendingUserTurnId,
        );
      }
      if (method === "reconcile") {
        reconcileCalls += 1;
        assert.equal(params?.prompt, "actually landed");
        assert.equal(params?.pendingUserTurnId, pendingUserTurnId);
        assert.equal(params?.pendingConversationUrl, pendingUrl);
        assert.deepEqual(params?.preClickBaseline, baseline);
        return {
          found: true,
          proven: true,
          conversationUrl: "https://chatgpt.com/c/landed-final",
          userTurnId: pendingUserTurnId,
          assistantTurnId: "assistant-landed",
        };
      }
      throw new Error(`unexpected ${method}`);
    }),
  }]);
  const first = await supervisor.run("actually landed", [], 2);
  assert.equal(first.status, "needs_user_action");
  assert.equal(supervisor.db.getRequest(first.sessionId!)?.conversation_url, pendingUrl);
  assert.equal(supervisor.db.latestAttempt(first.sessionId!)?.user_turn_id, pendingUserTurnId);
  const resumed = await supervisor.resume(first.sessionId!, 2);
  assert.equal(resumed.status, "complete");
  assert.equal(sendCalls, 1);
  assert.equal(reconcileCalls, 1);
  assert.equal(supervisor.db.latestAttempt(first.sessionId!)?.state, "reconciled");
});
test("an inaccessible pending tab stays uncertain and can never authorize attempt 2", async (t) => {
  const pendingUrl = "https://chatgpt.com/c/WEB:inaccessible";
  const pendingUserTurnId = "user-anchor-lost";
  let sendCalls = 0;
  const { supervisor } = await fixture(t, [{
    id: "slot-01",
    account: "a",
    handler: standardHandler((method, params) => {
      if (method === "send") {
        sendCalls += 1;
        throw new MockRpcError(
          "click_uncertain",
          "confirmation unavailable",
          "post_click",
          pendingUrl,
          [],
          pendingUserTurnId,
        );
      }
      if (method === "reconcile") {
        assert.equal(params?.pendingUserTurnId, pendingUserTurnId);
        assert.equal(params?.pendingConversationUrl, pendingUrl);
        return { found: false, proven: true };
      }
      throw new Error(`unexpected ${method}`);
    }),
  }]);
  const first = await supervisor.run("must not retry", [], 2);
  const resumed = await supervisor.resume(first.sessionId!, 2);
  assert.equal(resumed.status, "needs_user_action");
  assert.equal(sendCalls, 1);
  assert.deepEqual(
    supervisor.db.listAttempts(first.sessionId!).map((attempt) => attempt.state),
    ["uncertain"],
  );
  assert.equal(supervisor.db.getRequest(first.sessionId!)?.conversation_url, pendingUrl);
});
test("reconcile proven-not-found permits attempt 2 and attempt 2 exhaustion stops", async (t) => {
  await t.test("attempt 2 confirms", async (subtest) => {
    let sendCalls = 0;
    const { supervisor } = await fixture(subtest, [{
      id: "slot-01",
      account: "a",
      handler: standardHandler((method) => {
        if (method === "send") {
          sendCalls += 1;
          return sendCalls === 1
            ? DROP
            : {
              conversationUrl: "https://chatgpt.com/c/attempt-2",
              userTurnId: "user-2",
              assistantTurnId: "assistant-2",
            };
        }
        if (method === "reconcile") return { found: false, proven: true };
        throw new Error(`unexpected ${method}`);
      }),
    }]);
    const first = await supervisor.run("retry once", [], 2);
    const resumed = await supervisor.resume(first.sessionId!, 2);
    assert.equal(resumed.status, "complete");
    assert.equal(sendCalls, 2);
    assert.deepEqual(
      supervisor.db.listAttempts(first.sessionId!).map((attempt) => attempt.state),
      ["no_send_proven", "confirmed"],
    );
  });
  await t.test("attempt 2 also proven absent", async (subtest) => {
    let sendCalls = 0;
    const { supervisor } = await fixture(subtest, [{
      id: "slot-01",
      account: "a",
      handler: standardHandler((method) => {
        if (method === "send") {
          sendCalls += 1;
          return DROP;
        }
        if (method === "reconcile") return { found: false, proven: true };
        throw new Error(`unexpected ${method}`);
      }),
    }]);
    const first = await supervisor.run("retry exhausted", [], 2);
    const second = await supervisor.resume(first.sessionId!, 2);
    assert.equal(second.status, "needs_user_action");
    const third = await supervisor.resume(first.sessionId!, 2);
    assert.equal(third.status, "needs_user_action");
    assert.equal(supervisor.db.getRequest(first.sessionId!)?.status, "needs_user_action");
    assert.equal(sendCalls, 2);
    assert.equal(supervisor.db.listAttempts(first.sessionId!).length, 2);
  });
});
test("one slot multiplexes two requests with independent conversation identities", async (t) => {
  const sends = new Map<string, {
    conversationUrl: string;
    userTurnId: string;
    assistantTurnId: string;
  }>();
  const closed: string[] = [];
  const { supervisor } = await fixture(t, [{
    id: "slot-a",
    account: "a",
    handler: async (method, params) => {
      if (method === "readiness") return { state: "ready", modelLabel: "Pro" };
      if (method === "send") {
        const prompt = String(params?.prompt);
        const identity = {
          conversationUrl: `https://chatgpt.com/c/${prompt}`,
          userTurnId: `user-${prompt}`,
          assistantTurnId: `assistant-${prompt}`,
        };
        sends.set(prompt, identity);
        return identity;
      }
      if (method === "poll") {
        const conversationUrl = pollUrl(params);
        const prompt = conversationUrl.split("/").at(-1)!;
        const identity = sends.get(prompt);
        assert.ok(identity, `poll used an unknown conversation ${conversationUrl}`);
        assert.equal(params?.userTurnId, identity.userTurnId);
        assert.equal(params?.assistantTurnId, identity.assistantTurnId);
        return complete(`answer-${prompt}`, conversationUrl);
      }
      if (method === "closeConversation") {
        closed.push(pollUrl(params));
        return { ok: true };
      }
      throw new Error(`unexpected ${method}`);
    },
  }]);
  const [left, right] = await Promise.all([
    supervisor.run("left", [], 0),
    supervisor.run("right", [], 0),
  ]);
  assert.equal(left.status, "running");
  assert.equal(right.status, "running");
  assert.equal(supervisor.db.getRequest(left.sessionId!)?.slot_id, "slot-a");
  assert.equal(supervisor.db.getRequest(right.sessionId!)?.slot_id, "slot-a");
  assert.deepEqual([...sends.keys()].sort(), ["left", "right"]);
  const active = await supervisor.status();
  assert.equal(active.slots[0]?.activeRequests, 2);
  assert.deepEqual(
    active.requests.map((request) => request.id).sort(),
    [left.sessionId!, right.sessionId!].sort(),
  );
  const leftComplete = await supervisor.resume(left.sessionId!, 2);
  assert.equal(leftComplete.status, "complete");
  assert.equal(leftComplete.answer, "answer-left");
  assert.deepEqual(closed, ["https://chatgpt.com/c/left"]);
  assert.equal(supervisor.db.getRequest(right.sessionId!)?.status, "generating");
  assert.equal((await supervisor.status()).slots[0]?.activeRequests, 1);
  const rightComplete = await supervisor.resume(right.sessionId!, 2);
  assert.equal(rightComplete.status, "complete");
  assert.equal(rightComplete.answer, "answer-right");
  assert.deepEqual(closed, [
    "https://chatgpt.com/c/left",
    "https://chatgpt.com/c/right",
  ]);
  assert.equal((await supervisor.status()).slots[0]?.activeRequests, 0);
});
test("maxConcurrent exhaustion leaves the extra request recovering with pool_busy", async (t) => {
  const { supervisor } = await fixture(t, [{
    id: "slot-a",
    account: "a",
    handler: standardHandler((method, params) => {
      if (method === "send") {
        const prompt = String(params?.prompt);
        return {
          conversationUrl: `https://chatgpt.com/c/${prompt}`,
          userTurnId: `user-${prompt}`,
          assistantTurnId: `assistant-${prompt}`,
        };
      }
      throw new Error(`unexpected ${method}`);
    }),
  }], { maxConcurrent: 2 });
  const first = await supervisor.run("capacity-1", [], 0);
  const second = await supervisor.run("capacity-2", [], 0);
  assert.equal(first.status, "running");
  assert.equal(second.status, "running");
  const overflow = await supervisor.run("capacity-3", [], 0);
  assert.equal(overflow.status, "recovering");
  assert.equal(overflow.errorKind, "pool_busy");
  assert.match(overflow.resumeCommand ?? "", new RegExp(overflow.sessionId!));
  assert.equal(supervisor.db.getRequest(overflow.sessionId!)?.status, "staged");
  assert.equal(supervisor.db.getRequest(overflow.sessionId!)?.slot_id, null);
  assert.equal((await supervisor.status()).slots[0]?.activeRequests, 2);
});
test("provider-limit slot is skipped and receives a three-minute cooldown", async (t) => {
  let limitedSends = 0;
  let healthySends = 0;
  const { supervisor } = await fixture(t, [
    {
      id: "slot-01",
      account: "a",
      handler: (method) => {
        if (method === "readiness") return { state: "provider_limit", modelLabel: "Pro" };
        if (method === "send") limitedSends += 1;
        throw new Error(`unexpected ${method}`);
      },
    },
    {
      id: "slot-02",
      account: "b",
      handler: standardHandler((method) => {
        if (method === "send") {
          healthySends += 1;
          return {
            conversationUrl: "https://chatgpt.com/c/healthy",
            userTurnId: "user-healthy",
            assistantTurnId: "assistant-healthy",
          };
        }
        throw new Error(`unexpected ${method}`);
      }),
    },
  ]);
  const before = Date.now();
  const result = await supervisor.run("rotate", [], 2);
  assert.equal(result.status, "complete");
  assert.equal(limitedSends, 0);
  assert.equal(healthySends, 1);
  const limited = supervisor.db.getSlot("slot-01")!;
  assert.equal(limited.state, "provider_limit");
  assert.ok((limited.cooldown_until ?? 0) >= before + 179_000);
});
test("reap advances an abandoned generating request but never initiates staged sends", async (t) => {
  let readyToComplete = false;
  const { supervisor } = await fixture(t, [{
    id: "slot-01",
    account: "a",
    handler: (method, params) => {
      if (method === "readiness") return { state: "ready", modelLabel: "Pro" };
      if (method === "send") {
        return {
          conversationUrl: "https://chatgpt.com/c/reap-mock",
          userTurnId: "user-reap",
          assistantTurnId: "assistant-reap",
        };
      }
      if (method === "poll") return readyToComplete
        ? complete("reaped answer", pollUrl(params))
        : { state: "generating", currentUrl: pollUrl(params) };
      if (method === "closeConversation") return { ok: true };
      throw new Error(`unexpected ${method}`);
    },
  }]);
  // 소유 세션이 running envelope을 받고 사라진 상황 재현: generating으로 방치된다.
  const abandoned = await supervisor.run("abandoned by owner", [], 0);
  assert.equal(abandoned.status, "running");
  // 전송이 arm되지 않은 staged 요청은 reap이 절대 개시하지 않는다.
  supervisor.db.createRequest("req_00staged0000dead", sha256Text("never armed"));
  const still = await supervisor.reap(2);
  assert.deepEqual(
    still.actions.map((action) => [action.session, action.before, action.after]),
    [
      [abandoned.sessionId, "generating", "running"],
      ["req_00staged0000dead", "staged", "staged"],
    ],
  );
  readyToComplete = true;
  const reaped = await supervisor.reap(5);
  const advanced = reaped.actions.find((action) => action.session === abandoned.sessionId);
  assert.deepEqual(advanced && [advanced.before, advanced.after], ["generating", "complete"]);
  assert.equal(supervisor.db.getRequest("req_00staged0000dead")?.status, "staged");
  const settled = await supervisor.resume(abandoned.sessionId!, 0);
  assert.equal(settled.status, "complete");
  assert.equal(settled.answer, "reaped answer");
});
test("timeout returns running and resume completes the same session", async (t) => {
  let sendCalls = 0;
  let readyToComplete = false;
  const { supervisor, daemons } = await fixture(t, [{
    id: "slot-01",
    account: "a",
    handler: async (method, params) => {
      if (method === "readiness") return { state: "ready", modelLabel: "Pro" };
      if (method === "send") {
        sendCalls += 1;
        return {
          conversationUrl: "https://chatgpt.com/c/slow-mock",
          userTurnId: "user-slow",
          assistantTurnId: "assistant-slow",
        };
      }
      if (method === "poll") return readyToComplete
        ? complete("eventual answer", pollUrl(params))
        : { state: "generating", currentUrl: pollUrl(params) };
      throw new Error(`unexpected ${method}`);
    },
  }]);
  const first = await supervisor.run("eventual", [], 0);
  assert.equal(first.status, "running");
  assert.ok((daemons[0]?.healthCalls ?? 0) >= 1);
  assert.match(first.resumeCommand ?? "", new RegExp(first.sessionId!));
  readyToComplete = true;
  const resumed = await supervisor.resume(first.sessionId!, 2);
  assert.equal(resumed.status, "complete");
  assert.equal(resumed.sessionId, first.sessionId);
  assert.equal(resumed.answer, "eventual answer");
  assert.equal(sendCalls, 1);
});
test("poll currentUrl promotes a temporary WEB URL without stale downgrade", async (t) => {
  const prompt = "promote temporary conversation URL";
  const temporaryUrl = "https://chatgpt.com/c/WEB:temporary";
  const finalUrl = "https://chatgpt.com/c/final-conversation";
  const pollUrls: string[] = [];
  const downloadUrls: string[] = [];
  let artifactPath = "";
  const { supervisor, directory } = await fixture(t, [{
    id: "slot-01",
    account: "a",
    handler: async (method, params) => {
      if (method === "readiness") return { state: "ready", modelLabel: "Pro" };
      if (method === "send") return {
        conversationUrl: temporaryUrl,
        userTurnId: "user-promoted",
        assistantTurnId: "assistant-promoted",
      };
      if (method === "poll") {
        pollUrls.push(pollUrl(params));
        assert.equal(params?.promptSha256, sha256Text(prompt));
        assert.equal(params?.userTurnId, "user-promoted");
        assert.equal(params?.assistantTurnId, "assistant-promoted");
        if (pollUrls.length === 1) return { state: "generating", currentUrl: finalUrl };
        return {
          ...complete("promoted answer", temporaryUrl),
          artifactControls: [{ index: 0, label: "Download promoted.txt" }],
        };
      }
      if (method === "download") {
        downloadUrls.push(pollUrl(params));
        return {
          filename: "promoted.txt",
          outboxPath: artifactPath,
          sha256: await sha256File(artifactPath),
          sizeBytes: 8,
        };
      }
      throw new Error(`unexpected ${method}`);
    },
  }]);
  artifactPath = path.join(directory, "promoted.txt");
  await writeFile(artifactPath, "promoted");
  const result = await supervisor.run(prompt, [], 2);
  assert.equal(result.status, "complete");
  assert.equal(result.answer, "promoted answer");
  assert.deepEqual(pollUrls, [temporaryUrl, finalUrl]);
  assert.deepEqual(downloadUrls, [finalUrl]);
  assert.equal(supervisor.db.getRequest(result.sessionId!)?.conversation_url, finalUrl);
  assert.equal(result.artifacts.length, 1);
});
test("poll promotes an assistant placeholder id and sends the observed id on the next poll", async (t) => {
  const conversationUrl = "https://chatgpt.com/c/stable-assistant-id";
  const placeholder = "request-placeholder-request-WEB:mock-0";
  const observed = "e6888530-7b2b-4c3d-9e5f-000000000001";
  const pollAssistantIds: Array<string | undefined> = [];
  const { supervisor } = await fixture(t, [{
    id: "slot-01",
    account: "a",
    handler: async (method, params) => {
      if (method === "readiness") return { state: "ready", modelLabel: "Pro" };
      if (method === "send") return {
        conversationUrl,
        userTurnId: "user-durable",
        assistantTurnId: placeholder,
      };
      if (method === "poll") {
        assert.equal(params?.userTurnId, "user-durable");
        pollAssistantIds.push(typeof params?.assistantTurnId === "string"
          ? params.assistantTurnId
          : undefined);
        if (pollAssistantIds.length === 1) {
          return { state: "generating", currentUrl: conversationUrl, assistantTurnId: observed };
        }
        return { ...complete("assistant id promoted", conversationUrl), assistantTurnId: observed };
      }
      throw new Error(`unexpected ${method}`);
    },
  }]);
  const result = await supervisor.run("promote assistant id", [], 2);
  assert.equal(result.status, "complete");
  assert.equal(result.answer, "assistant id promoted");
  assert.deepEqual(pollAssistantIds, [placeholder, observed]);
  const attempt = supervisor.db.latestAttempt(result.sessionId!);
  assert.equal(attempt?.user_turn_id, "user-durable");
  assert.equal(attempt?.assistant_turn_id, observed);
  assert.equal(supervisor.db.getRequest(result.sessionId!)?.conversation_url, conversationUrl);
});
test("concurrent generating resumes finalize artifacts once and never revert complete", async (t) => {
  let sendCalls = 0;
  let downloadCalls = 0;
  const { supervisor, directory } = await fixture(t, [{
    id: "slot-01",
    account: "a",
    handler: async (method, params) => {
      if (method === "readiness") return { state: "ready", modelLabel: "Pro" };
      if (method === "send") {
        sendCalls += 1;
        return {
          conversationUrl: "https://chatgpt.com/c/concurrent-finalize",
          userTurnId: "user-finalize",
          assistantTurnId: "assistant-finalize",
        };
      }
      if (method === "poll") return {
        ...complete("final answer", pollUrl(params)),
        artifactControls: [{ index: 0, label: "Download final.txt" }],
      };
      if (method === "download") {
        downloadCalls += 1;
        const outboxPath = path.join(directory, `final-${downloadCalls}.txt`);
        await writeFile(outboxPath, "artifact");
        return {
          filename: "final.txt",
          outboxPath,
          sha256: await sha256File(outboxPath),
          sizeBytes: 8,
        };
      }
      throw new Error(`unexpected ${method}`);
    },
  }]);
  const first = await supervisor.run("finalize once", [], 0);
  assert.equal(first.status, "running");
  const second = await Supervisor.open({
    stateDir: path.join(directory, "state"),
    configPath: path.join(directory, "slots.json"),
  });
  t.after(() => second.close());
  const raced = await Promise.all([
    supervisor.resume(first.sessionId!, 2),
    second.resume(first.sessionId!, 2),
  ]);
  const settled = await Promise.all(raced.map((result) => (
    result.status === "running"
      ? supervisor.resume(first.sessionId!, 2)
      : Promise.resolve(result)
  )));
  assert.deepEqual(settled.map((result) => result.status), ["complete", "complete"]);
  assert.equal(supervisor.db.getRequest(first.sessionId!)?.status, "complete");
  assert.equal(supervisor.db.listArtifacts(first.sessionId!).length, 1);
  assert.equal(sendCalls, 1);
  assert.equal(downloadCalls, 1);
});
test("request-level attachments survive pool wait and duplicate basenames are staged deterministically", async (t) => {
  let receivedFiles: unknown[] = [];
  const { supervisor, directory } = await fixture(t, [{
    id: "slot-01",
    account: "a",
    handler: standardHandler((method, params) => {
      if (method === "send") {
        receivedFiles = params?.files as unknown[];
        return {
          conversationUrl: "https://chatgpt.com/c/files",
          userTurnId: "user-files",
          assistantTurnId: "assistant-files",
        };
      }
      throw new Error(`unexpected ${method}`);
    }),
  }], { maxConcurrent: 1 });
  const blockerId = "req_5000000000000003";
  supervisor.db.createRequest(blockerId, sha256Text("capacity blocker"));
  supervisor.db.updateRequest(blockerId, { slot_id: "slot-01" });
  const one = path.join(directory, "one");
  const two = path.join(directory, "two");
  await Promise.all([mkdir(one), mkdir(two)]);
  await Promise.all([
    writeFile(path.join(one, "a.tar.gz"), "one"),
    writeFile(path.join(two, "a.tar.gz"), "two"),
  ]);
  const waiting = await supervisor.run("with files", [
    path.join(one, "a.tar.gz"),
    path.join(two, "a.tar.gz"),
  ], 2);
  assert.equal(waiting.status, "recovering");
  const attachmentDir = path.join(directory, "state", "requests", waiting.sessionId!, "attachments");
  assert.deepEqual((await readdir(attachmentDir)).sort(), ["a-2.tar.gz", "a.tar.gz"]);
  supervisor.db.setRequestStatus(blockerId, "complete");
  const resumed = await supervisor.resume(waiting.sessionId!, 2);
  assert.equal(resumed.status, "complete");
  assert.deepEqual(
    receivedFiles.map((item) => (item as { name: string }).name).sort(),
    ["a-2.tar.gz", "a.tar.gz"],
  );
});
test("managed container artifact paths map, clean up, and preserve pure-file completion", async (t) => {
  let failedDownloads = 0;
  let goodSource = "", badSource = "";
  const { supervisor, daemons } = await fixture(t, [{
    id: "slot-01",
    account: "a",
    handler: async (method, params) => {
      if (method === "readiness") return { state: "ready", modelLabel: "Pro" };
      if (method === "send") return {
        conversationUrl: "https://chatgpt.com/c/artifact-partial",
        userTurnId: "user-artifact",
        assistantTurnId: "assistant-artifact",
      };
      if (method === "poll") return {
        ...complete("", pollUrl(params)),
        artifactControls: [
          { index: 0, label: "Download numbers.txt" },
          { index: 1, label: "Download bad.txt" },
        ],
      };
      if (method === "download" && params?.controlIndex === 0) {
        await mkdir(path.dirname(goodSource), { recursive: true });
        await writeFile(goodSource, "good");
        return {
          filename: "numbers.txt",
          outboxPath: `${CONTAINER_OUTBOX}/${path.basename(goodSource)}`,
          sha256: await sha256File(goodSource),
          sizeBytes: 4,
        };
      }
      if (method === "download") {
        failedDownloads += 1;
        await writeFile(badSource, "bad!");
        return {
          filename: "bad.txt",
          outboxPath: `${CONTAINER_OUTBOX}/${path.basename(badSource)}`,
          sha256: "0".repeat(64),
          sizeBytes: 4,
        };
      }
      throw new Error(`unexpected ${method}`);
    },
  }]);
  const slot = supervisor.config.slots[0]!;
  slot.unmanaged = false;
  const paths = supervisor.docker.paths(slot.id);
  goodSource = path.join(paths.outbox, ".gwp-0-0-numbers.txt");
  badSource = path.join(paths.outbox, ".gwp-1-0-bad.txt");
  supervisor.docker.ensure = async () => ({ port: daemons[0]!.port, tokenPath: paths.tokenPath });
  supervisor.docker.stop = async () => undefined;
  const result = await supervisor.run("artifact partial", [], 2);
  assert.equal(result.status, "complete");
  assert.equal(result.errorKind, null);
  assert.equal(result.answer, "");
  assert.equal(result.artifacts.length, 1);
  assert.equal(result.artifacts[0]!.filename, "numbers.txt");
  assert.equal(result.artifacts[0]!.sizeBytes, 4);
  assert.equal(await sha256File(result.artifacts[0]!.path), result.artifacts[0]!.sha256);
  assert.match(result.message ?? "", /1 artifact control/);
  assert.equal(failedDownloads, 2);
  assert.deepEqual(await readdir(paths.outbox), []);
});
test("keepalive separates probe results from durable slot states", async (t) => {
  const observed = new Map<string, "ready" | "needs_login" | "provider_limit" | "unknown">([
    ["slot-ready", "ready"],
    ["slot-login", "needs_login"],
    ["slot-limit", "provider_limit"],
    ["slot-unknown", "unknown"],
    ["slot-active", "ready"],
  ]);
  const readinessCalls = new Map<string, number>();
  const definitions = [...observed].map(([id, state]) => ({
    id,
    account: id,
    handler: (method: string) => {
      if (method !== "readiness") throw new Error(`unexpected ${method}`);
      readinessCalls.set(id, (readinessCalls.get(id) ?? 0) + 1);
      return { state, modelLabel: "Pro" };
    },
  }));
  const { supervisor, daemons } = await fixture(t, definitions);
  markSlotNeedsLogin(supervisor.db, "slot-ready");
  markSlotNeedsLogin(supervisor.db, "slot-active");
  markSlotProviderLimit(supervisor.db, "slot-unknown", 1_000);
  const activeId = "req_5000000000000004";
  supervisor.db.createRequest(activeId, sha256Text("already alive"));
  supervisor.db.updateRequest(activeId, { slot_id: "slot-active" });
  const before = Date.now();
  const report = await supervisor.keepalive();
  assert.deepEqual(report, {
    ok: true,
    slots: [
      { id: "slot-ready", state: "idle", probe: "ready" },
      { id: "slot-login", state: "needs_login", probe: "needs_login" },
      { id: "slot-limit", state: "provider_limit", probe: "provider_limit" },
      { id: "slot-unknown", state: "provider_limit", probe: "unknown" },
      { id: "slot-active", state: "needs_login", probe: "unknown" },
    ],
  });
  assert.deepEqual(Object.fromEntries(readinessCalls), {
    "slot-ready": 1,
    "slot-login": 1,
    "slot-limit": 1,
    "slot-unknown": 1,
  });
  assert.ok((supervisor.db.getSlot("slot-limit")?.cooldown_until ?? 0) >= before + 179_000);
  assert.equal(daemons[4]?.healthCalls, 0);
  assert.ok(daemons.slice(0, 4).every((daemon) => daemon.healthCalls >= 1));
});
test("keepalive stops only a managed runtime that it started", async (t) => {
  const { supervisor, daemons } = await fixture(t, [{
    id: "slot-a",
    account: "a",
    handler: (method) => {
      if (method === "readiness") return { state: "ready", modelLabel: "Pro" };
      throw new Error(`unexpected ${method}`);
    },
  }]);
  supervisor.config.slots[0]!.unmanaged = false;
  const endpoint = {
    port: daemons[0]!.port,
    tokenPath: supervisor.docker.paths("slot-a").tokenPath,
  };
  let running = false;
  let stopCalls = 0;
  supervisor.docker.inspect = async () => ({ exists: running, running, startedAt: null });
  supervisor.docker.ensure = async () => endpoint;
  supervisor.docker.stop = async () => { stopCalls += 1; };
  assert.deepEqual((await supervisor.keepalive()).slots[0], {
    id: "slot-a", state: "idle", probe: "ready",
  });
  assert.equal(stopCalls, 1);
  running = true;
  assert.deepEqual((await supervisor.keepalive()).slots[0], {
    id: "slot-a", state: "idle", probe: "ready",
  });
  assert.equal(stopCalls, 1);
  running = false;
  markSlotNeedsLogin(supervisor.db, "slot-a");
  supervisor.docker.ensure = async () => { throw new Error("daemon offline"); };
  assert.deepEqual((await supervisor.keepalive()).slots[0], {
    id: "slot-a", state: "needs_login", probe: "unreachable",
  });
  assert.equal(stopCalls, 2);
  supervisor.docker.inspect = async () => { throw new Error("inspect unavailable"); };
  assert.deepEqual((await supervisor.keepalive()).slots[0], {
    id: "slot-a", state: "needs_login", probe: "unreachable",
  });
  assert.equal(stopCalls, 2);
});
test("login polls needs_login to ready and reports the slot noVNC URL", async (t) => {
  let readinessCalls = 0;
  const urls: string[] = [];
  const progress: Array<{ state: string; elapsedMs: number }> = [];
  const { supervisor, daemons } = await fixture(t, [{
    id: "slot-a",
    account: "a",
    handler: (method) => {
      if (method !== "readiness") throw new Error(`unexpected ${method}`);
      readinessCalls += 1;
      return {
        state: readinessCalls === 1 ? "needs_login" : "ready",
        modelLabel: "Pro",
      };
    },
  }]);
  markSlotNeedsLogin(supervisor.db, "slot-a");
  const expectedUrl = `http://127.0.0.1:${daemons[0]!.port + 600}/vnc.html`;
  supervisor.config.slots[0]!.unmanaged = false;
  const endpoint = {
    port: daemons[0]!.port,
    tokenPath: supervisor.docker.paths("slot-a").tokenPath,
  };
  let stopCalls = 0;
  supervisor.docker.ensure = async (_slot, _timeout, options) => {
    assert.equal(options?.loginMode, true);
    return endpoint;
  };
  supervisor.docker.stop = async (slotId) => {
    assert.equal(slotId, "slot-a");
    stopCalls += 1;
  };
  const result = await supervisor.login("slot-a", {
    timeoutMs: 1_000,
    pollIntervalMs: 1,
    onUrl: (url) => urls.push(url),
    onProgress: (elapsedMs, state) => progress.push({ elapsedMs, state }),
  });
  assert.deepEqual(result, { slotId: "slot-a", state: "ready", url: expectedUrl });
  assert.deepEqual(urls, [expectedUrl]);
  assert.equal(progress.length, 1);
  assert.equal(progress[0]?.state, "needs_login");
  assert.ok((progress[0]?.elapsedMs ?? -1) >= 0);
  assert.equal(readinessCalls, 2);
  assert.equal(supervisor.db.getSlot("slot-a")?.state, "idle");
  assert.equal(stopCalls, 1);
});
test("login rejects timeout, interruption, active requests, and unknown slots", async (t) => {
  const abortReadiness = deferred<void>();
  let abortPhase = false;
  let rpcFailure = false;
  let readinessCalls = 0;
  const { supervisor, daemons } = await fixture(t, [{
    id: "slot-a",
    account: "a",
    handler: (method) => {
      if (method !== "readiness") throw new Error(`unexpected ${method}`);
      readinessCalls += 1;
      if (rpcFailure) throw new Error("readiness transport failed");
      if (abortPhase) abortReadiness.resolve();
      return { state: "needs_login", modelLabel: "Pro" };
    },
  }]);
  supervisor.config.slots[0]!.unmanaged = false;
  const endpoint = {
    port: daemons[0]!.port,
    tokenPath: supervisor.docker.paths("slot-a").tokenPath,
  };
  let stopCalls = 0;
  supervisor.docker.ensure = async () => endpoint;
  supervisor.docker.stop = async () => { stopCalls += 1; };
  await assert.rejects(
    supervisor.login("slot-a", { timeoutMs: 0, pollIntervalMs: 1 }),
    LoginTimeoutError,
  );
  assert.equal(supervisor.db.getSlot("slot-a")?.state, "needs_login");
  abortPhase = true;
  const controller = new AbortController();
  const interrupted = supervisor.login("slot-a", {
    timeoutMs: 1_000,
    pollIntervalMs: 1_000,
    signal: controller.signal,
  });
  await abortReadiness.promise;
  controller.abort();
  await assert.rejects(interrupted, LoginInterruptedError);
  assert.equal(stopCalls, 2);
  abortPhase = false;
  rpcFailure = true;
  await assert.rejects(
    supervisor.login("slot-a", { timeoutMs: 1_000, pollIntervalMs: 1 }),
    /readiness transport failed/,
  );
  assert.equal(stopCalls, 3);
  assert.equal(supervisor.db.getSlot("slot-a")?.state, "needs_login");
  const activeId = "req_5000000000000005";
  supervisor.db.createRequest(activeId, sha256Text("active login blocker"));
  supervisor.db.updateRequest(activeId, { slot_id: "slot-a" });
  const beforeRejectedCalls = readinessCalls;
  await assert.rejects(supervisor.login("slot-a"), InputError);
  await assert.rejects(supervisor.login("slot-missing"), InputError);
  assert.equal(readinessCalls, beforeRejectedCalls);
  assert.equal(stopCalls, 3);
});
test("cleanup --apply rechecks needs_login readiness and restores idle", async (t) => {
  const { supervisor } = await fixture(t, [{
    id: "slot-01",
    account: "a",
    handler: (method) => {
      if (method === "readiness") return { state: "ready", modelLabel: "Pro" };
      throw new Error(`unexpected ${method}`);
    },
  }]);
  markSlotNeedsLogin(supervisor.db, "slot-01");
  const result = await supervisor.cleanup(true);
  assert.equal(result.ok, true);
  assert.equal(result.dryRun, false);
  assert.ok(result.actions.some((action) => action.kind === "recover_login_slot"));
  assert.equal(supervisor.db.getSlot("slot-01")?.state, "idle");
});
