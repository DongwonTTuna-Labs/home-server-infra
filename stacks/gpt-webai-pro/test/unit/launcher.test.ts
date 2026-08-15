import assert from "node:assert/strict";
import { chmod, mkdtemp, mkdir, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

const stackRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

test("installed symlink resolves the launcher relative to the real stack", async () => {
  const fixture = await mkdtemp(join(tmpdir(), "gwp-launcher-"));
  try {
    const installBin = join(fixture, "install", "bin");
    const fakeBin = join(fixture, "fake-bin");
    await mkdir(installBin, { recursive: true });
    await mkdir(fakeBin, { recursive: true });

    const installedLauncher = join(installBin, "gpt-webai-pro");
    await symlink(join(stackRoot, "bin", "gpt-webai-pro"), installedLauncher);

    const fakeNode = join(fakeBin, "node");
    await writeFile(fakeNode, "#!/usr/bin/env bash\nprintf '%s\\n' \"$@\"\n", "utf8");
    await chmod(fakeNode, 0o755);

    const result = spawnSync(installedLauncher, ["status", "--json"], {
      encoding: "utf8",
      env: { ...process.env, PATH: `${fakeBin}:/usr/bin:/bin` },
    });

    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stderr, "");
    assert.deepEqual(result.stdout.trimEnd().split("\n"), [
      join(stackRoot, "dist", "cli", "main.js"),
      "status",
      "--json",
    ]);
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});
