import { createHash, randomBytes } from "node:crypto";
import { spawn } from "node:child_process";
import { createReadStream } from "node:fs";
import { appendFile, copyFile, mkdir, open, rename, rm, stat, writeFile } from "node:fs/promises";
import path from "node:path";
export interface FileLock {
  release(): Promise<void>;
}
export async function mkdirp(directory: string, mode = 0o700): Promise<void> {
  await mkdir(directory, { recursive: true, mode });
}
export async function atomicWrite(
  target: string,
  data: string | Uint8Array,
  mode = 0o600,
): Promise<void> {
  await mkdirp(path.dirname(target));
  const temporary = `${target}.tmp-${process.pid}-${randomBytes(4).toString("hex")}`;
  try {
    await writeFile(temporary, data, { mode });
    await rename(temporary, target);
  } finally {
    await rm(temporary, { force: true }).catch(() => undefined);
  }
}
export function sha256Text(value: string): string {
  return createHash("sha256").update(value, "utf8").digest("hex");
}
export async function sha256File(filename: string): Promise<string> {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(filename)) hash.update(chunk);
  return hash.digest("hex");
}
export async function moveFile(source: string, target: string): Promise<void> {
  await mkdirp(path.dirname(target));
  try {
    await rename(source, target);
  } catch (error) {
    if (!(error instanceof Error) || !("code" in error) || error.code !== "EXDEV") throw error;
    await copyFile(source, target);
    await rm(source);
  }
}
export async function fileSize(filename: string): Promise<number> {
  return (await stat(filename)).size;
}
export async function appendJsonLine(filename: string, value: unknown): Promise<void> {
  await mkdirp(path.dirname(filename));
  await appendFile(filename, `${JSON.stringify(value)}\n`, { mode: 0o600 });
}
export async function tryAcquireFileLock(filename: string): Promise<FileLock | null> {
  await mkdirp(path.dirname(filename));
  const handle = await open(filename, "a", 0o600);
  try {
    await handle.chmod(0o600);
    const result = await new Promise<{ code: number | null; signal: NodeJS.Signals | null; stderr: string }>(
      (resolve, reject) => {
        const child = spawn(
          "flock",
          ["--exclusive", "--nonblock", "--conflict-exit-code", "75", "3"],
          { stdio: ["ignore", "ignore", "pipe", handle.fd] },
        );
        let stderr = "";
        child.stderr?.setEncoding("utf8");
        child.stderr?.on("data", (chunk: string) => {
          if (stderr.length < 8_192) stderr += chunk.slice(0, 8_192 - stderr.length);
        });
        child.once("error", reject);
        child.once("close", (code, signal) => resolve({ code, signal, stderr }));
      },
    );
    if (result.code === 75) {
      await handle.close();
      return null;
    }
    if (result.code !== 0) {
      throw new Error(
        `flock failed${result.signal ? ` with ${result.signal}` : ` with exit ${String(result.code)}`}`
        + `${result.stderr.trim() ? `: ${result.stderr.trim()}` : ""}`,
      );
    }
  } catch (error) {
    await handle.close().catch(() => undefined);
    throw error;
  }
  let released = false;
  return {
    async release() {
      if (released) return;
      released = true;
      await handle.close();
    },
  };
}
