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
  assert.ok(args.includes("type=bind,src=/state/slots/slot-01/profile,dst=/profile"));
  assert.ok(args.includes("type=bind,src=/state/slots/slot-01/inbox,dst=/inbox,readonly"));
  assert.ok(args.includes("type=bind,src=/state/slots/slot-01/outbox,dst=/outbox"));
  assert.equal(args.some((argument) => argument.includes("/sock")), false);
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
