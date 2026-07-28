import assert from "node:assert/strict";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import path from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";
const root = path.resolve(import.meta.dirname, "../..");
const mainUrl = pathToFileURL(path.join(root, "src/cli/main.ts")).href;
const runUrl = pathToFileURL(path.join(root, "src/supervisor/run.ts")).href;
const source = `
  import {
    InputError,
    LoginInterruptedError,
    LoginTimeoutError,
    Supervisor,
  } from ${JSON.stringify(runUrl)};
  import { main } from ${JSON.stringify(mainUrl)};
  const mode = process.env.LOGIN_TEST_MODE;
  Supervisor.open = async () => ({
    close() {
      if (mode === "close_error" || mode === "keepalive_close_error") {
        throw new Error("close failed");
      }
    },
    async login(slot, options) {
      if (mode === "usage") throw new InputError("slot slot-a has active requests");
      const url = "http://127.0.0.1:19901/vnc.html";
      options.onUrl?.(url);
      if (mode === "success" || mode === "close_error") {
        options.onProgress?.(5_000, "needs_login");
        return { slotId: slot, state: "ready", url };
      }
      if (mode === "timeout") throw new LoginTimeoutError("login timed out for slot-a");
      if (mode === "daemon") throw new Error("daemon offline");
      if (mode === "abort") {
        process.stderr.write("TEST_READY\\n");
        const keepAlive = setInterval(() => {}, 1_000);
        return new Promise((_, reject) => options.signal.addEventListener("abort", () => {
          clearInterval(keepAlive);
          reject(new LoginInterruptedError("login interrupted"));
        }, { once: true }));
      }
      throw new Error("unknown test mode");
    },
    async keepalive() {
      return { ok: true, slots: [{ id: "slot-a", state: "idle", probe: "ready" }] };
    },
  });
  process.exitCode = await main(mode === "keepalive_close_error"
    ? ["keepalive"]
    : ["login", "--slot", "slot-a"]);
`;
interface Completion {
  code: number | null;
  signal: NodeJS.Signals | null;
  stdout: string;
  stderr: string;
}
function start(mode: string): {
  child: ChildProcessWithoutNullStreams;
  completion: Promise<Completion>;
  stderr: () => string;
} {
  const child = spawn(
    process.execPath,
    ["--import", "tsx", "--input-type=module", "--eval", source],
    {
      cwd: root,
      env: { ...process.env, LOGIN_TEST_MODE: mode },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk: string) => { stdout += chunk; });
  child.stderr.on("data", (chunk: string) => { stderr += chunk; });
  const completion = new Promise<Completion>((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code, signal) => resolve({ code, signal, stdout, stderr }));
  });
  return { child, completion, stderr: () => stderr };
}
function jsonLine(output: string): unknown {
  assert.equal(output.split("\n").filter(Boolean).length, 1);
  return JSON.parse(output);
}
test("login CLI emits one stdout JSON object for terminal outcomes", async (t) => {
  await t.test("success keeps progress on stderr", async () => {
    const process = start("success");
    const result = await process.completion;
    assert.equal(result.code, 0, result.stderr);
    assert.deepEqual(jsonLine(result.stdout), {
      ok: true,
      slot: "slot-a",
      state: "ready",
      novncUrl: "http://127.0.0.1:19901/vnc.html",
    });
    assert.match(result.stderr, /noVNC: http:\/\/127\.0\.0\.1:19901\/vnc\.html/);
    assert.match(result.stderr, /로그인 대기 중/);
    assert.match(result.stderr, /경과 5초/);
  });
  await t.test("close failure cannot append a second stdout object", async () => {
    const result = await start("close_error").completion;
    assert.equal(result.code, 0, result.stderr);
    assert.deepEqual(jsonLine(result.stdout), {
      ok: true,
      slot: "slot-a",
      state: "ready",
      novncUrl: "http://127.0.0.1:19901/vnc.html",
    });
    assert.match(result.stderr, /supervisor close failed: close failed/);
  });
  await t.test("timeout is an exit-zero observation", async () => {
    const result = await start("timeout").completion;
    assert.equal(result.code, 0, result.stderr);
    assert.deepEqual(jsonLine(result.stdout), {
      ok: false,
      slot: "slot-a",
      state: "needs_login",
      errorKind: "login_timeout",
      message: "login timed out for slot-a",
    });
  });
  await t.test("daemon failure is exit zero", async () => {
    const result = await start("daemon").completion;
    assert.equal(result.code, 0, result.stderr);
    assert.deepEqual(jsonLine(result.stdout), {
      ok: false,
      slot: "slot-a",
      errorKind: "daemon_unreachable",
      message: "daemon offline",
    });
  });
  await t.test("usage failure is exit two", async () => {
    const result = await start("usage").completion;
    assert.equal(result.code, 2, result.stderr);
    assert.deepEqual(jsonLine(result.stdout), {
      ok: false,
      error: "slot slot-a has active requests",
    });
  });
});
test("keepalive CLI preserves one JSON object when supervisor close fails", async () => {
  const result = await start("keepalive_close_error").completion;
  assert.equal(result.code, 0, result.stderr);
  assert.deepEqual(jsonLine(result.stdout), {
    ok: true,
    slots: [{ id: "slot-a", state: "idle", probe: "ready" }],
  });
  assert.match(result.stderr, /supervisor close failed: close failed/);
});
for (const signal of ["SIGINT", "SIGTERM"] as const) {
  test(`login CLI handles ${signal} after cleanup with exit 130`, async () => {
    const running = start("abort");
    const deadline = Date.now() + 5_000;
    while (!running.stderr().includes("TEST_READY") && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    assert.match(running.stderr(), /TEST_READY/);
    running.child.kill(signal);
    const result = await running.completion;
    assert.equal(result.code, 130, result.stderr);
    assert.equal(result.signal, null);
    assert.deepEqual(jsonLine(result.stdout), {
      ok: false,
      slot: "slot-a",
      errorKind: "login_aborted",
    });
  });
}
