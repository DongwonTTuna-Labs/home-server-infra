import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, stat } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { DockerManager, rotateDaemonToken } from "../../src/supervisor/docker.js";

test("slot container create args publish only the authenticated daemon TCP port", () => {
  const manager = new DockerManager("/state", "test-image", "http://fake-chatgpt.invalid");
  const token = "a".repeat(32);
  const args = manager.createArguments(
    { id: "slot-01", account: "a", port: 19301 },
    token,
  );

  assert.deepEqual(
    args.filter((argument, index) => args[index - 1] === "--publish"),
    ["127.0.0.1:19301:19301"],
  );
  assert.ok(args.includes("GWP_DAEMON_PORT=19301"));
  assert.ok(args.includes(`GWP_DAEMON_TOKEN=${token}`));
  assert.equal(args.includes("GWP_LOGIN_MODE=1"), false);
  assert.equal(args.some((argument) => argument.startsWith("GWP_NOVNC_PORT=")), false);
  assert.ok(args.includes("type=bind,src=/state/slots/slot-01/profile,dst=/profile"));
  assert.ok(args.includes("type=bind,src=/state/slots/slot-01/inbox,dst=/inbox,readonly"));
  assert.ok(args.includes("type=bind,src=/state/slots/slot-01/outbox,dst=/outbox"));
  assert.equal(args.some((argument) => argument.includes("/sock")), false);
});

test("login-mode container adds loopback noVNC publish and login-only environment", () => {
  const manager = new DockerManager("/state", "test-image", "http://fake-chatgpt.invalid");
  const token = "b".repeat(32);
  const args = manager.createArguments(
    { id: "slot-a", account: "a", port: 19301 },
    token,
    undefined,
    { loginMode: true },
  );

  assert.deepEqual(
    args.filter((argument, index) => args[index - 1] === "--publish"),
    ["127.0.0.1:19301:19301", "127.0.0.1:19901:19901"],
  );
  assert.ok(args.includes("GWP_LOGIN_MODE=1"));
  assert.ok(args.includes("GWP_NOVNC_PORT=19901"));
});

test("container image and entrypoint contain login runtime and anti-throttling contract", async () => {
  const dockerfile = await readFile(new URL("../../container/Dockerfile", import.meta.url), "utf8");
  const entrypoint = await readFile(new URL("../../container/entrypoint.sh", import.meta.url), "utf8");

  for (const packageName of ["python3", "make", "g++", "x11vnc", "novnc", "websockify"]) {
    assert.ok(dockerfile.includes(packageName), `Dockerfile is missing ${packageName}`);
  }
  assert.match(entrypoint, /1440x900x24/);
  assert.match(entrypoint, /x11vnc[\s\S]*-localhost/);
  assert.match(entrypoint, /websockify[\s\S]*--web=\/usr\/share\/novnc/);
  for (const flag of [
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
  ]) {
    assert.ok(entrypoint.includes(flag));
  }
});

test("daemon token rotation writes a fresh 32 lower-hex value with mode 0600", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-daemon-token-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const tokenPath = path.join(directory, "slot-01", "daemon.token");

  const first = await rotateDaemonToken(tokenPath);
  assert.match(first, /^[0-9a-f]{32}$/);
  assert.equal((await readFile(tokenPath, "utf8")).trim(), first);
  assert.equal((await stat(tokenPath)).mode & 0o777, 0o600);

  const second = await rotateDaemonToken(tokenPath);
  assert.match(second, /^[0-9a-f]{32}$/);
  assert.notEqual(second, first);
  assert.equal((await readFile(tokenPath, "utf8")).trim(), second);
  assert.equal((await stat(tokenPath)).mode & 0o777, 0o600);
});
