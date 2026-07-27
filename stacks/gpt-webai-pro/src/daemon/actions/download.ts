import { access } from "node:fs/promises";
import path from "node:path";
import type { Page } from "playwright-core";

import { GwpError } from "../../shared/errors.js";
import { fileSize, mkdirp, sha256File } from "../../shared/fsx.js";
import type { DownloadParams, DownloadResult } from "../../shared/types.js";
import type { BrowserSession } from "../browser.js";
import { artifactControlLocators } from "../selectors.js";

export class ArtifactDownloader {
  private readonly attempts = new Map<string, number>();

  constructor(private readonly outboxDir: string) {}

  async download(
    session: BrowserSession,
    params: DownloadParams,
  ): Promise<DownloadResult> {
    const key = `${params.conversationUrl}\n${params.controlIndex}`;
    const attempt = (this.attempts.get(key) ?? 0) + 1;
    if (attempt > 2) throw new GwpError("artifact_failed", "artifact control exceeded two attempts");
    this.attempts.set(key, attempt);
    const page = await session.findConversationPage(params.conversationUrl)
      ?? await session.open(params.conversationUrl);
    const controls = await artifactControlLocators(page);
    const control = controls[params.controlIndex];
    if (!control) throw new GwpError("artifact_failed", `artifact control ${params.controlIndex} is absent`);

    try {
      const event = page.waitForEvent("download", { timeout: 30_000 });
      await control.click({ timeout: 10_000 });
      const download = await event;
      const filename = safeBasename(download.suggestedFilename());
      await mkdirp(this.outboxDir);
      const outboxPath = await uniqueOutboxPath(this.outboxDir, params.controlIndex, filename);
      await download.saveAs(outboxPath);
      const failure = await download.failure();
      if (failure) throw new Error(failure);
      return {
        filename,
        outboxPath,
        sha256: await sha256File(outboxPath),
        sizeBytes: await fileSize(outboxPath),
      };
    } catch (error) {
      if (error instanceof GwpError) throw error;
      throw new GwpError("artifact_failed", `artifact download failed: ${String(error)}`, { cause: error });
    }
  }
}

function safeBasename(value: string): string {
  const filename = path.basename(value).replace(/[\u0000-\u001f]/gu, "").trim();
  if (!filename || filename === "." || filename === "..") {
    throw new GwpError("artifact_failed", "download supplied an invalid filename");
  }
  return filename;
}

async function uniqueOutboxPath(directory: string, index: number, filename: string): Promise<string> {
  for (let suffix = 0; suffix < 1_000; suffix += 1) {
    const prefix = `.gwp-${process.pid}-${index}-${suffix}-`;
    const candidate = path.join(directory, `${prefix}${filename}`);
    try {
      await access(candidate);
    } catch {
      return candidate;
    }
  }
  throw new GwpError("artifact_failed", "outbox filename space exhausted");
}
