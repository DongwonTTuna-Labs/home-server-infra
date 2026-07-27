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
import { Supervisor } from "../src/supervisor/run.js";
import { markSlotNeedsLogin } from "../src/supervisor/slots.js";

const DROP = Symbol("drop connection");

class MockRpcError extends Error {
  constructor(
    readonly kind: string,
    message: string,
    readonly phase?: "pre_click" | "post_click",
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
  await writeFile(configPath, JSON.stringify({ image: "unused-in-tests", slots }));
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

test("two concurrent runs claim different slots and rotate accounts", async (t) => {
  const prompts = new Map<string, string[]>();
  const definitions = [
    { id: "slot-01", account: "a" },
    { id: "slot-02", account: "b" },
  ].map((slot) => ({
    ...slot,
    handler: standardHandler((method, params) => {
      if (method === "send") {
        const values = prompts.get(slot.id) ?? [];
        values.push(String(params?.prompt));
        prompts.set(slot.id, values);
        return {
          conversationUrl: `https://chatgpt.com/c/${slot.id}`,
          userTurnId: `user-${slot.id}`,
          assistantTurnId: `assistant-${slot.id}`,
        };
      }
      throw new Error(`unexpected ${method}`);
    }),
  }));
  const { supervisor } = await fixture(t, definitions);
  const [left, right] = await Promise.all([
    supervisor.run("left", [], 2),
    supervisor.run("right", [], 2),
  ]);
  assert.equal(left.status, "complete");
  assert.equal(right.status, "complete");
  assert.deepEqual([...prompts.keys()].sort(), ["slot-01", "slot-02"]);
  const assigned = [
    supervisor.db.getRequest(left.sessionId!)?.slot_id,
    supervisor.db.getRequest(right.sessionId!)?.slot_id,
  ];
  assert.equal(new Set(assigned).size, 2);
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
      if (method === "open") throw new Error("assistant id promotion must not navigate");
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
  }]);
  supervisor.db.connection.prepare("UPDATE slots SET state = 'busy' WHERE id = 'slot-01'").run();
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
  supervisor.db.connection.prepare("UPDATE slots SET state = 'idle' WHERE id = 'slot-01'").run();
  const resumed = await supervisor.resume(waiting.sessionId!, 2);
  assert.equal(resumed.status, "complete");
  assert.deepEqual(
    receivedFiles.map((item) => (item as { name: string }).name).sort(),
    ["a-2.tar.gz", "a.tar.gz"],
  );
});

test("artifact failures retry twice, preserve successes, and still complete", async (t) => {
  let failedDownloads = 0;
  const { supervisor, directory } = await fixture(t, [{
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
        ...complete("answer survives", pollUrl(params)),
        artifactControls: [
          { index: 0, label: "Download good.txt" },
          { index: 1, label: "Download bad.txt" },
        ],
      };
      if (method === "download" && params?.controlIndex === 0) {
        const outboxPath = path.join(directory, "good.txt");
        await writeFile(outboxPath, "good");
        return {
          filename: "good.txt",
          outboxPath,
          sha256: await sha256File(outboxPath),
          sizeBytes: 4,
        };
      }
      if (method === "download") {
        failedDownloads += 1;
        throw new MockRpcError("artifact_failed", "download failed");
      }
      throw new Error(`unexpected ${method}`);
    },
  }]);
  const result = await supervisor.run("artifact partial", [], 2);
  assert.equal(result.status, "complete");
  assert.equal(result.errorKind, null);
  assert.equal(result.artifacts.length, 1);
  assert.match(result.message ?? "", /1 artifact control/);
  assert.equal(failedDownloads, 2);
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
