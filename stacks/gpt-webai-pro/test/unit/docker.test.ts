import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, stat, symlink } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { CONTAINER_OUTBOX, DockerManager, inspectOwnedContainer, mapContainerOutboxPath, rotateDaemonToken } from "../../src/supervisor/docker.js";
test("container ownership accepts only the same canonical profile, inbox, and outbox", async (context) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-container-owner-"));
  context.after(() => rm(directory, { recursive: true, force: true }));
  const manager = new DockerManager(path.join(directory, "state"), "test-image");
  const paths = manager.paths("slot-a");
  await Promise.all([paths.profile, paths.inbox, paths.outbox].map((source) => mkdir(source, { recursive: true })));
  const alias = path.join(directory, "profile-alias");
  await symlink(paths.profile, alias, "dir");
  const inspection = {
    Id: "owned-container",
    State: { Running: true, StartedAt: "2026-09-05T00:00:00Z" },
    Mounts: [
      { Type: "bind", Source: alias, Destination: "/profile" },
      { Type: "bind", Source: paths.inbox, Destination: "/inbox" },
      { Type: "bind", Source: paths.outbox, Destination: "/outbox" },
    ],
  };
  assert.deepEqual(await inspectOwnedContainer(inspection, paths), {
    id: "owned-container", exists: true, running: true, startedAt: 1788566400000,
  });
  const foreign = path.join(directory, "foreign");
  await mkdir(foreign);
  for (const destination of ["/profile", "/inbox", "/outbox"]) {
    const mismatched = inspection.Mounts.map((mount) => mount.Destination === destination ? { ...mount, Source: foreign } : mount);
    await assert.rejects(inspectOwnedContainer({ ...inspection, Mounts: mismatched }, paths), /container ownership mismatch/);
  }
  await assert.rejects(inspectOwnedContainer({ ...inspection, Mounts: [] }, paths), /container ownership mismatch/);
  await assert.rejects(inspectOwnedContainer({ ...inspection, Mounts: [...inspection.Mounts, inspection.Mounts[0]!] }, paths), /container ownership mismatch/);
  await assert.rejects(inspectOwnedContainer({ ...inspection, Id: undefined }, paths), /ownership cannot be established/);
});
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
test("managed artifact paths map only from the container outbox mount", () => {
  const hostOutbox = path.resolve("/state/slots/slot-01/outbox");
  assert.equal(
    mapContainerOutboxPath(`${CONTAINER_OUTBOX}/.gwp-0-0-numbers.txt`, hostOutbox),
    path.join(hostOutbox, ".gwp-0-0-numbers.txt"),
  );
  for (const outside of ["/etc/passwd", "/outboxevil/x", "/outbox/../etc/passwd"]) {
    assert.throws(() => mapContainerOutboxPath(outside, hostOutbox), /outside slot outbox/);
  }
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
