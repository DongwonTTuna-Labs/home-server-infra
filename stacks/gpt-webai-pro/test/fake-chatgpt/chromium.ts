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
    "Chromium was not found via CHROME_BINARY_PATH or the platform Playwright cache.",
    `npx playwright install chromium exited ${String(install.status)}.`,
    install.stderr.trim(),
  ].filter(Boolean).join(" "));
}
async function cachedChromium(): Promise<string | null> {
  const home = process.env.HOME ?? "";
  const caches = process.platform === "darwin"
    ? [path.join(home, "Library", "Caches", "ms-playwright"), path.join(home, ".cache", "ms-playwright")]
    : [path.join(home, ".cache", "ms-playwright")];
  const relatives = [
    "chrome-linux64/chrome",
    "chrome-linux/chrome",
    "chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
    "chrome-mac/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
    "chrome-mac-arm64/Chromium.app/Contents/MacOS/Chromium",
    "chrome-mac/Chromium.app/Contents/MacOS/Chromium",
  ];
  for (const cache of caches) {
    let directories: string[];
    try {
      directories = (await readdir(cache)).filter((name) => name.startsWith("chromium-")).sort().reverse();
    } catch {
      continue;
    }
    for (const directory of directories) {
      for (const relative of relatives) {
        const candidate = path.join(cache, directory, relative);
        if (await executable(candidate)) return candidate;
      }
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
