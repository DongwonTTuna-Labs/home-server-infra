import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { access, mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  emptyPromptEnvelope,
  networkFailureEnvelope,
  recoveringEnvelope,
  runningEnvelope,
} from "../../src/cli/envelope.js";
test("envelope contract cases are closed and stable", () => {
  const empty = emptyPromptEnvelope();
  assert.equal(empty.ok, true);
  assert.equal(empty.usageError, true);
  assert.equal(empty.status, "needs_user_action");
  assert.equal(empty.hardFailure, false);
  const running = runningEnvelope("req_aaaaaaaaaaaaaaaa");
  assert.equal(running.status, "running");
  assert.equal(running.sessionId, "req_aaaaaaaaaaaaaaaa");
  assert.equal(running.resumeCommand, "gpt-webai-pro resume --session req_aaaaaaaaaaaaaaaa");
  assert.equal(running.nextCommand, null);
  const busy = recoveringEnvelope("req_bbbbbbbbbbbbbbbb", "pool_busy", "busy");
  assert.equal(busy.status, "recovering");
  assert.equal(busy.nextCommand, busy.resumeCommand);
  const network = networkFailureEnvelope("req_cccccccccccccccc", "offline proof");
  assert.equal(network.ok, false);
  assert.equal(network.hardFailure, true);
  assert.equal(network.networkDisconnected, true);
  assert.equal(network.errorKind, "network_disconnected");
});
test("empty prompt exits zero, emits one JSON line, and creates no state", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-empty-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const stateDir = path.join(directory, "state");
  const result = spawnSync(process.execPath, ["--import", "tsx", "src/cli/main.ts", "run", ""], {
    cwd: path.resolve(import.meta.dirname, "../.."),
    env: { ...process.env, GPT_WEBAI_PRO_STATE_DIR: stateDir },
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout.split("\n").filter(Boolean).length, 1);
  assert.equal(JSON.parse(result.stdout).usageError, true);
  await assert.rejects(access(stateDir));
});
test("other input errors use exit 2 and an envelope", () => {
  const result = spawnSync(process.execPath, ["--import", "tsx", "src/cli/main.ts", "run", "--bogus"], {
    cwd: path.resolve(import.meta.dirname, "../.."),
    encoding: "utf8",
  });
  assert.equal(result.status, 2, result.stderr);
  const envelope = JSON.parse(result.stdout);
  assert.equal(envelope.usageError, true);
  assert.equal(envelope.status, "needs_user_action");
});
test("status uses its own exact JSON shape rather than a request envelope", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-status-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const result = spawnSync(process.execPath, ["--import", "tsx", "src/cli/main.ts", "status", "--json"], {
    cwd: path.resolve(import.meta.dirname, "../.."),
    env: { ...process.env, GPT_WEBAI_PRO_STATE_DIR: path.join(directory, "state") },
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr);
  const status = JSON.parse(result.stdout);
  assert.deepEqual(Object.keys(status), ["ok", "slots", "requests"]);
  assert.equal(status.ok, true);
  assert.equal(status.slots.length, 4);
  assert.deepEqual(
    Object.keys(status.slots[0]),
    ["id", "account", "state", "cooldownUntil", "lastUsedAt", "activeRequests", "weeklyUsed", "weeklyLimit", "weeklyResetAt"],
  );
  assert.equal(status.slots[0].activeRequests, 0);
  assert.equal(status.slots[0].weeklyUsed, 0);
  assert.equal(status.slots[0].weeklyLimit, 200);
  assert.equal(status.slots[0].weeklyResetAt, null);
  assert.deepEqual(status.requests, []);
});
