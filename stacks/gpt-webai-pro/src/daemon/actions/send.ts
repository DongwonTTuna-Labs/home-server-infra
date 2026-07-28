import type { Page } from "playwright-core";
import { GwpError } from "../../shared/errors.js";
import type { LabelConfig, SendParams, SendResult } from "../../shared/types.js";
import {
  COMPOSER_SELECTORS,
  FILE_INPUT_SELECTOR,
  SEND_BUTTON_SELECTORS,
  UPLOAD_BUTTON_SELECTORS,
  normalizeChipStem,
  renderedTurnMatchesPrompt,
  observeAttachmentChips,
  readTurns,
  visibleFirst,
} from "../selectors.js";
import type { BrowserSession } from "../browser.js";
import { ensurePro } from "./model.js";
export async function sendMessage(
  session: BrowserSession,
  params: SendParams,
  labels: LabelConfig,
): Promise<SendResult> {
  let page: Page | null = null;
  let clickStarted = false;
  let pendingUserTurnId: string | undefined;
  let preClickBaseline: string[] | undefined;
  try {
    page = await session.newConversation();
    await ensurePro(page, labels);
    await fillComposer(page, params.prompt);
    if (params.files.length > 0) {
      await attachFiles(page, params.files.map((file) => file.containerPath));
      await waitForExpectedChips(page, params.files.map((file) => file.name), 30_000);
    }
    const baseline = await readTurns(page);
    const baselineIds = new Set(baseline.map((turn) => turn.dataMessageId));
    const send = await waitForSendButton(page, 30_000);
    if (!send) throw new GwpError("compose_failed", "send button is not ready", { phase: "pre_click" });
    preClickBaseline = [...baselineIds];
    clickStarted = true;
    try {
      await send.click({ timeout: 10_000 });
    } catch (error) {
      throw new GwpError("click_uncertain", `send click did not return cleanly: ${String(error)}`, {
        phase: "post_click",
        cause: error,
      });
    }
    const deadline = Date.now() + 60_000;
    while (Date.now() < deadline) {
      const turns = await readTurns(page);
      const newUsers = turns.filter((turn) => (
        turn.role === "user" && !baselineIds.has(turn.dataMessageId)
      ));
      pendingUserTurnId ??= newUsers[0]?.dataMessageId;
      const user = newUsers.find((turn) => renderedTurnMatchesPrompt(turn.text, params.prompt));
      const assistant = user && turns.find((turn) => (
        turn.role === "assistant"
        && !baselineIds.has(turn.dataMessageId)
        && turn.domIndex > user.domIndex
      ));
      const conversationUrl = page.url();
      if (user && assistant && session.isConversationUrl(conversationUrl)) {
        return {
          conversationUrl,
          userTurnId: user.dataMessageId,
          assistantTurnId: assistant.dataMessageId,
        };
      }
      await page.waitForTimeout(250);
    }
    throw new GwpError(
      "click_uncertain",
      "new user turn, following assistant turn, and non-root conversation URL were not all observed",
      { phase: "post_click" },
    );
  } catch (error) {
    const gwp = error instanceof GwpError ? error : null;
    throw new GwpError(
      gwp?.kind ?? (clickStarted ? "click_uncertain" : "compose_failed"),
      gwp?.detail ?? String(error),
      {
        // Entering send.click() transfers authority irrevocably to post-click.
        phase: clickStarted ? "post_click" : gwp?.phase ?? "pre_click",
        cause: error,
        ...(gwp?.networkEvidence ? { networkEvidence: true } : {}),
        ...(clickStarted && page
          ? {
              ...(pendingUserTurnId ? { pendingUserTurnId } : {}),
              pendingConversationUrl: page.url(),
              preClickBaseline: preClickBaseline ?? [],
            }
          : {}),
      },
    );
  }
}
async function fillComposer(page: Page, prompt: string): Promise<void> {
  const composer = await visibleFirst(page, COMPOSER_SELECTORS);
  if (!composer) throw new GwpError("compose_failed", "composer is not visible", { phase: "pre_click" });
  await composer.click({ timeout: 10_000 });
  try {
    await composer.fill(prompt, { timeout: 10_000 });
  } catch {
    await page.keyboard.press(process.platform === "darwin" ? "Meta+A" : "Control+A");
    await page.keyboard.insertText(prompt);
  }
}
async function attachFiles(page: Page, files: string[]): Promise<void> {
  const input = page.locator(FILE_INPUT_SELECTOR).first();
  if (await input.count() > 0) {
    await input.setInputFiles(files, { timeout: 30_000 });
    return;
  }
  const upload = await visibleFirst(page, UPLOAD_BUTTON_SELECTORS);
  if (!upload) throw new GwpError("compose_failed", "upload control is not available", { phase: "pre_click" });
  const chooser = page.waitForEvent("filechooser", { timeout: 10_000 });
  await upload.click({ timeout: 10_000 });
  await (await chooser).setFiles(files);
}
async function waitForExpectedChips(
  page: Page,
  expectedNames: string[],
  timeoutMs: number,
): Promise<void> {
  const expected = counts(expectedNames.map(normalizeChipStem));
  const deadline = Date.now() + timeoutMs;
  let latest = await observeAttachmentChips(page);
  while (Date.now() < deadline) {
    latest = await observeAttachmentChips(page);
    const observed = counts(latest.map((chip) => normalizeChipStem(chip.filename)));
    if (latest.length === expectedNames.length
      && latest.every((chip) => chip.complete)
      && sameCounts(expected, observed)) return;
    await page.waitForTimeout(250);
  }
  throw new GwpError(
    "chip_mismatch",
    `attachment chips did not match: expected=${JSON.stringify([...expected])} observed=${JSON.stringify(latest)}`,
    { phase: "pre_click" },
  );
}
async function waitForSendButton(page: Page, timeoutMs: number) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const send = await visibleFirst(page, SEND_BUTTON_SELECTORS);
    if (send && await send.isEnabled().catch(() => false)) return send;
    await page.waitForTimeout(250);
  }
  return null;
}
function counts(values: string[]): Map<string, number> {
  const result = new Map<string, number>();
  for (const value of values) result.set(value, (result.get(value) ?? 0) + 1);
  return result;
}
function sameCounts(left: Map<string, number>, right: Map<string, number>): boolean {
  return left.size === right.size && [...left].every(([key, value]) => right.get(key) === value);
}
