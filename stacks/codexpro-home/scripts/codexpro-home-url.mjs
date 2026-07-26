#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const root = process.env.HOME;
if (!root || !path.isAbsolute(root)) {
  throw new Error("HOME must be an absolute path.");
}

const stateRoot = path.join(root, ".codexpro");
const outputPath = path.join(stateRoot, "current-server-url.txt");

function readMatchingJson(directory) {
  if (!fs.existsSync(directory)) return null;
  for (const name of fs.readdirSync(directory).filter((entry) => entry.endsWith(".json"))) {
    const filePath = path.join(directory, name);
    try {
      const value = JSON.parse(fs.readFileSync(filePath, "utf8"));
      if (value?.root === root) return value;
    } catch {
      // Ignore incomplete or unrelated state files while the service starts.
    }
  }
  return null;
}

function currentUrl() {
  const profile = readMatchingJson(path.join(stateRoot, "profiles"));
  const runtime = readMatchingJson(path.join(stateRoot, "runtime"));
  if (!profile?.token) throw new Error("CodexPro profile token is unavailable.");
  const endpoint = profile.tunnel === "cloudflare-named" && profile.hostname
    ? `https://${profile.hostname}/mcp`
    : runtime?.endpoint;
  if (!endpoint) throw new Error("CodexPro endpoint is unavailable.");
  const url = new URL(endpoint);
  url.searchParams.set("codexpro_token", profile.token);
  return url;
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function resolveUrl(waitMilliseconds) {
  const deadline = Date.now() + waitMilliseconds;
  let lastError;
  do {
    try {
      return currentUrl();
    } catch (error) {
      lastError = error;
      if (Date.now() >= deadline) break;
      await sleep(250);
    }
  } while (true);
  throw lastError;
}

const args = process.argv.slice(2);
const writeRequested = args.includes("--write");
const redactedRequested = args.includes("--redacted");
if (writeRequested === redactedRequested) {
  throw new Error("Pass exactly one of --write or --redacted.");
}
const waitIndex = args.indexOf("--wait");
const waitMilliseconds = waitIndex >= 0 ? Number(args[waitIndex + 1] ?? 0) : 0;
if (!Number.isFinite(waitMilliseconds) || waitMilliseconds < 0 || waitMilliseconds > 60_000) {
  throw new Error("--wait must be between 0 and 60000 milliseconds.");
}

const url = await resolveUrl(waitMilliseconds);
if (writeRequested) {
  fs.mkdirSync(stateRoot, { recursive: true, mode: 0o700 });
  fs.writeFileSync(outputPath, `${url.toString()}\n`, { mode: 0o600 });
  fs.chmodSync(outputPath, 0o600);
  process.stdout.write(`${outputPath}\n`);
} else {
  url.searchParams.set("codexpro_token", "<redacted>");
  process.stdout.write(`${url.toString()}\n`);
}
