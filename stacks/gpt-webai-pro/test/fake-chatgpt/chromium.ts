import { spawnSync } from "node:child_process";
import { constants } from "node:fs";
import { access, readdir } from "node:fs/promises";
import path from "node:path";
export async function findChromium(): Promise<string> {
  const configured = process.env.CHROME_BINARY_PATH;
  if (configured && await executable(configured)) return configured;
  const cached = await cachedChromium();
  if (cached) return cached;
  const install = spawnSync("npx", ["playwright", "install", "chromium"], {
    cwd: path.resolve(import.meta.dirname, "../.."),
    encoding: "utf8",
  });
  const installed = await cachedChromium();
  if (install.status === 0 && installed) return installed;
  throw new Error([
    "Chromium was not found via CHROME_BINARY_PATH or ~/.cache/ms-playwright/chromium-*.",
    `npx playwright install chromium exited ${String(install.status)}.`,
    install.stderr.trim(),
  ].filter(Boolean).join(" "));
}
async function cachedChromium(): Promise<string | null> {
  const cache = path.join(process.env.HOME ?? "", ".cache", "ms-playwright");
  let directories: string[];
  try {
    directories = (await readdir(cache)).filter((name) => name.startsWith("chromium-")).sort().reverse();
  } catch {
    return null;
  }
  for (const directory of directories) {
    for (const relative of ["chrome-linux64/chrome", "chrome-linux/chrome"]) {
      const candidate = path.join(cache, directory, relative);
      if (await executable(candidate)) return candidate;
    }
  }
  return null;
}
async function executable(filename: string): Promise<boolean> {
  try {
    await access(filename, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}
