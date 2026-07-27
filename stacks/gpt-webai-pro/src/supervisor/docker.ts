import { randomBytes } from "node:crypto";
import { execFile } from "node:child_process";
import { chmod } from "node:fs/promises";
import path from "node:path";
import { promisify } from "node:util";

import { atomicWrite, mkdirp } from "../shared/fsx.js";
import type { SlotConfig } from "../shared/types.js";
import { RpcClient } from "./rpc-client.js";

const execFileAsync = promisify(execFile);

export const CREATE_LIMIT_ARGS = [
  "--memory", "3g",
  "--cpus", "2",
  "--pids-limit", "1024",
  "--shm-size", "1g",
  "--security-opt", "no-new-privileges",
  "--cap-drop", "ALL",
] as const;

export interface ContainerState {
  exists: boolean;
  running: boolean;
  startedAt: number | null;
}

export interface SlotPaths {
  profile: string;
  tokenPath: string;
  inbox: string;
  outbox: string;
}

export interface DaemonEndpoint {
  port: number;
  tokenPath: string;
}

export class DockerManager {
  constructor(
    readonly stateDir: string,
    readonly image: string,
    readonly baseUrl = process.env.GWP_BASE_URL ?? "https://chatgpt.com",
  ) {}

  paths(slotId: string): SlotPaths {
    const root = path.join(this.stateDir, "slots", slotId);
    return {
      profile: path.join(root, "profile"),
      tokenPath: path.join(root, "daemon.token"),
      inbox: path.join(root, "inbox"),
      outbox: path.join(root, "outbox"),
    };
  }

  containerName(slotId: string): string {
    return `gwp-${slotId}`;
  }

  async ensure(slot: SlotConfig, timeoutMs = 60_000): Promise<DaemonEndpoint> {
    const paths = this.paths(slot.id);
    const endpoint = { port: slot.port, tokenPath: paths.tokenPath };
    if (slot.unmanaged === true) {
      await chmod(paths.tokenPath, 0o600);
      await waitForDaemon(endpoint, timeoutMs);
      return endpoint;
    }

    await Promise.all([
      mkdirp(paths.profile),
      mkdirp(paths.inbox),
      mkdirp(paths.outbox),
    ]);
    const state = await this.inspect(slot.id);
    if (!state.running) {
      if (state.exists) await this.remove(slot.id);
      const token = await rotateDaemonToken(paths.tokenPath);
      await this.create(slot, token, paths);
      await this.start(slot.id);
    }
    await chmod(paths.tokenPath, 0o600);
    await waitForDaemon(endpoint, timeoutMs);
    return endpoint;
  }

  async create(slot: SlotConfig, token: string, paths = this.paths(slot.id)): Promise<void> {
    await docker(this.createArguments(slot, token, paths));
  }

  createArguments(slot: SlotConfig, token: string, paths = this.paths(slot.id)): string[] {
    const uid = process.getuid?.() ?? 1000;
    const gid = process.getgid?.() ?? 1000;
    return [
      "create",
      "--name", this.containerName(slot.id),
      ...CREATE_LIMIT_ARGS,
      "--user", `${uid}:${gid}`,
      "--restart", "no",
      "--publish", `127.0.0.1:${slot.port}:${slot.port}`,
      "--mount", `type=bind,src=${paths.profile},dst=/profile`,
      "--mount", `type=bind,src=${paths.inbox},dst=/inbox,readonly`,
      "--mount", `type=bind,src=${paths.outbox},dst=/outbox`,
      "--env", `GWP_BASE_URL=${this.baseUrl}`,
      "--env", "GWP_CDP_URL=http://127.0.0.1:9222",
      "--env", `GWP_DAEMON_PORT=${slot.port}`,
      "--env", `GWP_DAEMON_TOKEN=${token}`,
      "--env", "GWP_OUTBOX_DIR=/outbox",
      this.image,
    ];
  }

  async start(slotId: string): Promise<void> {
    await docker(["start", this.containerName(slotId)]);
  }

  async stop(slotId: string): Promise<void> {
    const state = await this.inspect(slotId);
    if (state.exists && state.running) await docker(["stop", this.containerName(slotId)]);
  }

  private async remove(slotId: string): Promise<void> {
    await docker(["rm", this.containerName(slotId)]);
  }

  async inspect(slotId: string): Promise<ContainerState> {
    try {
      const output = await docker(["inspect", this.containerName(slotId)]);
      const item = (JSON.parse(output) as Array<{
        State?: { Running?: boolean; StartedAt?: string };
      }>)[0];
      const started = item?.State?.StartedAt ? Date.parse(item.State.StartedAt) : Number.NaN;
      return {
        exists: true,
        running: item?.State?.Running === true,
        startedAt: Number.isFinite(started) ? started : null,
      };
    } catch (error) {
      if (error instanceof Error && /No such object|No such container/i.test(error.message)) {
        return { exists: false, running: false, startedAt: null };
      }
      throw error;
    }
  }
}

export async function rotateDaemonToken(tokenPath: string): Promise<string> {
  const token = randomBytes(16).toString("hex");
  await atomicWrite(tokenPath, `${token}\n`, 0o600);
  await chmod(tokenPath, 0o600);
  return token;
}

async function docker(args: string[]): Promise<string> {
  try {
    const result = await execFileAsync("docker", args, {
      encoding: "utf8",
      maxBuffer: 4 * 1024 * 1024,
    });
    return result.stdout;
  } catch (error) {
    if (error instanceof Error && "stderr" in error) {
      let detail = String((error as Error & { stderr?: string }).stderr ?? "").trim();
      for (const argument of args) {
        if (!argument.startsWith("GWP_DAEMON_TOKEN=")) continue;
        detail = detail.replaceAll(argument.slice("GWP_DAEMON_TOKEN=".length), "[redacted]");
      }
      throw new Error(detail || `docker ${args[0] ?? "command"} failed`);
    }
    throw error;
  }
}

async function waitForDaemon(endpoint: DaemonEndpoint, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    let client: RpcClient | null = null;
    try {
      const remaining = Math.max(1, deadline - Date.now());
      client = await RpcClient.connect(
        endpoint.port,
        endpoint.tokenPath,
        Math.min(1_000, remaining),
      );
      const health = await client.call("health", undefined, Math.min(1_000, remaining));
      await client.close();
      if (health.ok === true && health.chromeConnected === true) return;
      await new Promise((resolve) => setTimeout(resolve, 100));
    } catch {
      if (client) await client.close().catch(() => undefined);
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
  throw new Error(`daemon did not become healthy on 127.0.0.1:${endpoint.port}`);
}
