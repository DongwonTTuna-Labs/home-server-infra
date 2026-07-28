import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { access, mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { chromium, type Browser, type Page } from "playwright-core";
import WebSocket from "ws";
import { GwpError } from "../src/shared/errors.js";
import { sha256Text } from "../src/shared/fsx.js";
import type { ReconcileParams } from "../src/shared/types.js";
import { startDaemon, type DaemonHandle } from "../src/daemon/main.js";
import { BrowserSession } from "../src/daemon/browser.js";
import { artifactControls } from "../src/daemon/selectors.js";
import { RpcClient } from "../src/supervisor/rpc-client.js";
import { findChromium } from "./fake-chatgpt/chromium.js";
import { startFakeChatGpt } from "./fake-chatgpt/server.js";
const LABELS = {
  target: ["Pro"],
  intelligence: ["Instant", "Medium", "High", "Extra High", "Pro"],
};
interface ScenarioRuntime {
  browser: Browser;
  daemon: DaemonHandle;
  rpc: RpcClient;
  directory: string;
  token: string;
  tokenPath: string;
  close(): Promise<void>;
}
function pollRequest(
  sent: { conversationUrl: string; userTurnId: string; assistantTurnId: string },
  prompt: string,
  waitMs: number,
) {
  return {
    conversationUrl: sent.conversationUrl,
    promptSha256: sha256Text(prompt),
    userTurnId: sent.userTurnId,
    assistantTurnId: sent.assistantTurnId,
    waitMs,
  };
}
function reconcileRequest(
  prompt: string,
  extra: Omit<ReconcileParams, "prompt" | "promptSha256"> = {},
): ReconcileParams {
  return { prompt, promptSha256: sha256Text(prompt), ...extra };
}
async function seedCompletedConversation(page: Page, values: {
  url: string;
  prompt: string;
  answer: string;
  userTurnId: string;
  assistantTurnId: string;
}): Promise<void> {
  await page.evaluate((input) => {
    const conversation = document.querySelector<HTMLElement>("#conversation");
    if (!conversation) throw new Error("fake conversation is missing");
    conversation.replaceChildren();
    const user = document.createElement("div");
    user.setAttribute("data-message-author-role", "user");
    user.setAttribute("data-message-id", input.userTurnId);
    user.textContent = input.prompt;
    const assistant = document.createElement("div");
    assistant.setAttribute("data-message-author-role", "assistant");
    assistant.setAttribute("data-message-id", input.assistantTurnId);
    assistant.textContent = input.answer;
    const copy = document.createElement("button");
    copy.type = "button";
    copy.dataset.testid = "copy-turn-action";
    copy.setAttribute("aria-label", "Copy");
    assistant.append(copy);
    conversation.append(user, assistant);
    history.replaceState({}, "", input.url);
  }, values);
}
async function pageWithUserTurn(browser: Browser, userTurnId: string): Promise<Page> {
  for (const context of browser.contexts()) {
    for (const page of context.pages()) {
      if (page.isClosed()) continue;
      const found = await page.locator('[data-message-author-role="user"]').evaluateAll(
        (nodes, expected) => nodes.some((node) => (
          node.getAttribute("data-message-id") === expected
        )),
        userTurnId,
      ).catch(() => false);
      if (found) return page;
    }
  }
  throw new Error(`browser page for user turn ${userTurnId} was not found`);
}
async function unauthorizedStatus(port: number, authorization?: string): Promise<number> {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(`ws://127.0.0.1:${port}/`, authorization
      ? { headers: { Authorization: authorization } }
      : undefined);
    let settled = false;
    socket.once("unexpected-response", (_request, response) => {
      settled = true;
      response.resume();
      socket.terminate();
      resolve(response.statusCode ?? 0);
    });
    socket.once("open", () => {
      settled = true;
      socket.terminate();
      reject(new Error("unauthenticated WebSocket unexpectedly opened"));
    });
    socket.once("error", (error) => {
      if (!settled) reject(error);
    });
  });
}
test("daemon RPC covers required fake scenarios and the live Intelligence picker contract", async (t) => {
  const executablePath = await findChromium();
  const fake = await startFakeChatGpt();
  t.after(() => fake.close());
  async function setup(scenario: string): Promise<ScenarioRuntime> {
    const directory = await mkdtemp(path.join(os.tmpdir(), `gwp-daemon-${scenario}-`));
    const browser = await chromium.launch({
      executablePath,
      headless: true,
      args: ["--no-sandbox", "--disable-setuid-sandbox"],
    });
    const context = await browser.newContext({ acceptDownloads: true });
    const page = await context.newPage();
    const baseUrl = fake.baseUrl(scenario);
    await page.goto(baseUrl);
    const session = await BrowserSession.fromBrowser(browser, baseUrl);
    const token = randomBytes(16).toString("hex");
    const tokenPath = path.join(directory, "daemon.token");
    await writeFile(tokenPath, `${token}\n`, { mode: 0o600 });
    const daemon = await startDaemon({
      session,
      port: 0,
      token,
      outboxDir: path.join(directory, "outbox"),
      labels: LABELS,
    });
    const rpc = await RpcClient.connect(daemon.port, tokenPath);
    return {
      browser,
      daemon,
      rpc,
      directory,
      token,
      tokenPath,
      async close() {
        await rpc.close().catch(() => undefined);
        await daemon.close().catch(() => undefined);
        await browser.close().catch(() => undefined);
        await rm(directory, { recursive: true, force: true });
      },
    };
  }
  await t.test("happy: readiness, send, reconcile, poll, open, health, capture", async () => {
    const runtime = await setup("happy");
    try {
      const initialReadiness = await runtime.rpc.call("readiness", undefined);
      assert.equal(initialReadiness.state, "ready");
      assert.equal(initialReadiness.modelLabel, "Instant\n5.5");
      assert.deepEqual(
        await runtime.rpc.call("reconcile", reconcileRequest("lost with browser")),
        { found: false, proven: false },
      );
      const prompt = "hello from daemon e2e";
      const sent = await runtime.rpc.call("send", { prompt, files: [], newConversation: true });
      assert.match(sent.conversationUrl, /\/c\/fake-/);
      assert.match(sent.userTurnId, /^user-/);
      assert.match(sent.assistantTurnId, /^assistant-/);
      assert.equal((await runtime.rpc.call("readiness", undefined)).modelLabel, "Pro");
      const page = await pageWithUserTurn(runtime.browser, sent.userTurnId);
      assert.equal(await page.locator("#intelligence-pill").getAttribute("data-open-count"), "1");
      assert.equal(
        await page.locator('[role="menuitemradio"][data-intelligence="Pro"]').getAttribute("aria-checked"),
        "true",
      );
      const reconciled = await runtime.rpc.call("reconcile", reconcileRequest(prompt, {
        conversationUrl: sent.conversationUrl,
      }));
      assert.equal(reconciled.found, true);
      const missing = await runtime.rpc.call("reconcile", reconcileRequest("not present", {
        conversationUrl: sent.conversationUrl,
      }));
      assert.deepEqual(missing, { found: false, proven: true });
      const polled = await runtime.rpc.call("poll", pollRequest(sent, prompt, 7_000));
      assert.equal(polled.state, "complete");
      assert.equal(polled.answerMarkdown, "fake answer");
      assert.equal(polled.answerSha256, sha256Text("fake answer"));
      assert.equal((await runtime.rpc.call("open", { conversationUrl: sent.conversationUrl })).ok, true);
      assert.equal((await runtime.rpc.call("health", undefined)).chromeConnected, true);
      const captures = await Promise.all([
        runtime.rpc.call("captureFailure", { tag: "test-capture" }),
        runtime.rpc.call("captureFailure", { tag: "test-capture" }),
      ]);
      assert.notEqual(captures[0].screenshotPath, captures[1].screenshotPath);
      assert.notEqual(captures[0].htmlPath, captures[1].htmlPath);
      for (const capture of captures) {
        await access(capture.screenshotPath);
        await access(capture.htmlPath);
      }
      assert.equal(
        (await runtime.rpc.call("closeConversation", {
          conversationUrl: polled.currentUrl,
        })).ok,
        true,
      );
      assert.equal(page.isClosed(), true);
    } finally { await runtime.close(); }
  });
  await t.test("temporary WEB URL is replaced without navigating away from confirmed turns", async () => {
    const runtime = await setup("url-rebind");
    try {
      const prompt = "temporary URL promotion";
      const sent = await runtime.rpc.call("send", { prompt, files: [], newConversation: true });
      assert.match(sent.conversationUrl, /\/c\/WEB:fake-/);
      const page = await pageWithUserTurn(runtime.browser, sent.userTurnId);
      await page.waitForURL(/\/c\/final-/);
      const finalUrl = page.url();
      const terminal = await runtime.rpc.call("poll", pollRequest(sent, prompt, 7_000));
      assert.equal(terminal.state, "complete");
      assert.equal(terminal.currentUrl, finalUrl);
      assert.equal(page.url(), finalUrl);
      const repeated = await runtime.rpc.call("poll", pollRequest(sent, prompt, 0));
      assert.equal(repeated.currentUrl, finalUrl);
      assert.equal(page.url(), finalUrl);
    } finally { await runtime.close(); }
  });
  await t.test("assistant placeholder id rebinds to the observed final id behind the durable user turn", async () => {
    const runtime = await setup("assistant-id-rebind");
    try {
      const prompt = "assistant id promotion";
      const sent = await runtime.rpc.call("send", { prompt, files: [], newConversation: true });
      assert.match(sent.userTurnId, /^user-/);
      assert.match(sent.assistantTurnId, /^request-placeholder-request-WEB:/);
      const page = await pageWithUserTurn(runtime.browser, sent.userTurnId);
      await page.waitForFunction((placeholder) => {
        const assistant = document.querySelector('[data-message-author-role="assistant"]');
        const observed = assistant?.getAttribute("data-message-id");
        return Boolean(observed && observed !== placeholder);
      }, sent.assistantTurnId);
      const observedAssistantTurnId = await page
        .locator('[data-message-author-role="assistant"]')
        .getAttribute("data-message-id");
      assert.match(observedAssistantTurnId ?? "", /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/);
      const terminal = await runtime.rpc.call("poll", pollRequest(sent, prompt, 7_000));
      assert.equal(terminal.state, "complete");
      assert.equal(terminal.answerMarkdown, "fake answer");
      assert.equal(terminal.assistantTurnId, observedAssistantTurnId);
    } finally {
      await runtime.close();
    }
  });
  await t.test("root redirect during rebind falls back to prompt reconciliation in open tabs", async () => {
    const runtime = await setup("root-redirect");
    try {
      const prompt = "recover after stale URL redirects to root";
      const context = runtime.browser.contexts()[0];
      assert.ok(context);
      const target = await context.newPage();
      await target.goto(fake.baseUrl("root-redirect"));
      await target.evaluate((text) => {
        const conversation = document.querySelector<HTMLElement>("#conversation");
        if (!conversation) throw new Error("fake conversation is missing");
        const user = document.createElement("div");
        user.setAttribute("data-message-author-role", "user");
        user.setAttribute("data-message-id", "user-root-rebind");
        user.textContent = text;
        const assistant = document.createElement("div");
        assistant.setAttribute("data-message-author-role", "assistant");
        assistant.setAttribute("data-message-id", "assistant-root-rebind");
        assistant.textContent = "recoverable answer";
        conversation.append(user, assistant);
        conversation.style.display = "none";
        addEventListener("storage", (event) => {
          if (event.key === "gwp-root-redirected") conversation.style.display = "block";
        });
        history.replaceState({}, "", "/c/final-root?scenario=root-redirect");
      }, prompt);
      const finalUrl = target.url();
      const staleUrl = new URL(
        "/c/WEB:stale-root?scenario=root-redirect",
        fake.baseUrl("root-redirect"),
      ).href;
      const reconciled = await runtime.rpc.call("reconcile", reconcileRequest(prompt, {
        conversationUrl: staleUrl,
        pendingUserTurnId: "user-root-rebind",
      }));
      assert.equal(reconciled.found, true);
      assert.equal(reconciled.conversationUrl, finalUrl);
      assert.equal(reconciled.userTurnId, "user-root-rebind");
      assert.equal(reconciled.assistantTurnId, "assistant-root-rebind");
    } finally {
      await runtime.close();
    }
  });
  await t.test("stored URL wins over another open tab with the same prompt", async () => {
    const runtime = await setup("happy");
    try {
      const prompt = "identical prompt in two conversations";
      const context = runtime.browser.contexts()[0];
      const competitor = context?.pages()[0];
      assert.ok(context);
      assert.ok(competitor);
      await seedCompletedConversation(competitor, {
        url: "/c/competitor?scenario=happy",
        prompt,
        answer: "wrong answer",
        userTurnId: "user-competitor",
        assistantTurnId: "assistant-competitor",
      });
      const competitorUrl = competitor.url();
      const target = await context.newPage();
      await target.goto(fake.baseUrl("happy"));
      await seedCompletedConversation(target, {
        url: "/c/target?scenario=happy",
        prompt,
        answer: "target answer",
        userTurnId: "user-target",
        assistantTurnId: "assistant-target",
      });
      const targetUrl = target.url();
      const polled = await runtime.rpc.call("poll", {
        conversationUrl: targetUrl,
        promptSha256: sha256Text(prompt),
        waitMs: 4_000,
      });
      assert.equal(polled.state, "complete");
      assert.equal(polled.currentUrl, targetUrl);
      assert.equal(polled.answerMarkdown, "target answer");
      const reconciled = await runtime.rpc.call("reconcile", reconcileRequest(prompt, {
        conversationUrl: targetUrl,
      }));
      assert.equal(reconciled.found, true);
      assert.equal(reconciled.conversationUrl, targetUrl);
      assert.equal(reconciled.userTurnId, "user-target");
      assert.equal(reconciled.assistantTurnId, "assistant-target");
      const idAnchored = await runtime.rpc.call("reconcile", reconcileRequest(prompt, {
        conversationUrl: competitorUrl,
        pendingUserTurnId: "user-target",
      }));
      assert.equal(idAnchored.conversationUrl, targetUrl);
      assert.equal(idAnchored.userTurnId, "user-target");
    } finally {
      await runtime.close();
    }
  });
  await t.test("durable user anchor wins across same-prompt tabs and a stale assistant id", async () => {
    const runtime = await setup("happy");
    try {
      const prompt = "durable identity in duplicate conversations";
      const context = runtime.browser.contexts()[0];
      const competitor = context?.pages()[0];
      assert.ok(context);
      assert.ok(competitor);
      await seedCompletedConversation(competitor, {
        url: "/c/durable-competitor?scenario=happy",
        prompt,
        answer: "wrong durable answer",
        userTurnId: "user-durable-competitor",
        assistantTurnId: "assistant-durable-competitor",
      });
      const competitorUrl = competitor.url();
      const target = await context.newPage();
      await target.goto(fake.baseUrl("happy"));
      await seedCompletedConversation(target, {
        url: "/c/durable-target?scenario=happy",
        prompt,
        answer: "durable target answer",
        userTurnId: "user-durable-target",
        assistantTurnId: "assistant-durable-target",
      });
      const polled = await runtime.rpc.call("poll", {
        conversationUrl: competitorUrl,
        promptSha256: sha256Text(prompt),
        userTurnId: "user-durable-target",
        assistantTurnId: "request-placeholder-request-WEB:stale-target-0",
        waitMs: 4_000,
      });
      assert.equal(polled.state, "complete");
      assert.equal(polled.currentUrl, target.url());
      assert.equal(polled.answerMarkdown, "durable target answer");
      assert.equal(polled.assistantTurnId, "assistant-durable-target");
    } finally {
      await runtime.close();
    }
  });
  await t.test("two requests share one browser without sharing tabs and close independently", async () => {
    const runtime = await setup("multi-tab");
    try {
      const before = runtime.browser.contexts()[0]?.pages().length;
      assert.equal(before, 1);
      const [first, second] = await Promise.all([
        runtime.rpc.call("send", {
          prompt: "first multiplexed request",
          files: [],
          newConversation: true,
        }),
        runtime.rpc.call("send", {
          prompt: "second multiplexed request",
          files: [],
          newConversation: true,
        }),
      ]);
      assert.notEqual(first.conversationUrl, second.conversationUrl);
      assert.notEqual(first.userTurnId, second.userTurnId);
      const firstPage = await pageWithUserTurn(runtime.browser, first.userTurnId);
      const secondPage = await pageWithUserTurn(runtime.browser, second.userTurnId);
      assert.notEqual(firstPage, secondPage);
      assert.equal(runtime.browser.contexts()[0]?.pages().length, 3);
      const [firstPoll, secondPoll] = await Promise.all([
        runtime.rpc.call("poll", pollRequest(first, "first multiplexed request", 7_000)),
        runtime.rpc.call("poll", pollRequest(second, "second multiplexed request", 7_000)),
      ]);
      assert.equal(firstPoll.state, "complete");
      assert.equal(firstPoll.answerMarkdown, "answer for first multiplexed request");
      assert.equal(secondPoll.state, "complete");
      assert.equal(secondPoll.answerMarkdown, "answer for second multiplexed request");
      await runtime.rpc.call("closeConversation", { conversationUrl: firstPoll.currentUrl });
      assert.equal(firstPage.isClosed(), true);
      assert.equal(secondPage.isClosed(), false);
      await runtime.rpc.call("closeConversation", { conversationUrl: secondPoll.currentUrl });
      assert.equal(secondPage.isClosed(), true);
    } finally {
      await runtime.close();
    }
  });
  await t.test("anchorless reconcile recovers one bound match but not absence or ambiguity", async () => {
    const runtime = await setup("multi-tab");
    try {
      const prompt = "identical prompt during reconcile";
      const first = await runtime.rpc.call("send", { prompt, files: [], newConversation: true });
      const recovered = await runtime.rpc.call("reconcile", reconcileRequest(prompt));
      assert.equal(recovered.found, true);
      assert.equal(recovered.userTurnId, first.userTurnId);
      const absent = await runtime.rpc.call("reconcile", reconcileRequest("missing anchorless prompt"));
      assert.deepEqual(absent, { found: false, proven: false });
      const second = await runtime.rpc.call("send", {
        prompt,
        files: [],
        newConversation: true,
      });
      const ambiguous = await runtime.rpc.call("reconcile", reconcileRequest(prompt));
      assert.deepEqual(ambiguous, { found: false, proven: false });
      const firstScoped = await runtime.rpc.call("reconcile", reconcileRequest(prompt, {
        conversationUrl: first.conversationUrl,
      }));
      const secondScoped = await runtime.rpc.call("reconcile", reconcileRequest(prompt, {
        conversationUrl: second.conversationUrl,
      }));
      assert.equal(firstScoped.userTurnId, first.userTurnId);
      assert.equal(secondScoped.userTurnId, second.userTurnId);
    } finally {
      await runtime.close();
    }
  });
  await t.test("reconcile follows a pending root tab but never proves an inaccessible tab absent", async () => {
    const runtime = await setup("multi-tab");
    try {
      const page = await runtime.browser.contexts()[0]!.newPage();
      await page.goto(fake.baseUrl("multi-tab"));
      const pendingConversationUrl = page.url();
      const prompt = "uniquely landed after confirmation miss";
      const origin = new URL(fake.baseUrl("multi-tab")).origin;
      const finalUrl = `${origin}/c/pending-promoted?scenario=multi-tab`;
      await seedCompletedConversation(page, {
        url: finalUrl,
        prompt,
        answer: "landed",
        userTurnId: "user-pending-promoted",
        assistantTurnId: "assistant-pending-promoted",
      });
      const found = await runtime.rpc.call("reconcile", reconcileRequest(prompt, {
        pendingConversationUrl,
        pendingUserTurnId: "user-pending-promoted",
        preClickBaseline: [],
      }));
      assert.equal(found.found, true);
      assert.equal(found.conversationUrl, finalUrl);
      assert.equal(found.userTurnId, "user-pending-promoted");
      const inaccessible = await runtime.rpc.call("reconcile", reconcileRequest("not present in any tab", {
        pendingConversationUrl: `${origin}/c/WEB:stale-root?scenario=root-redirect`,
        pendingUserTurnId: "missing-user-anchor",
        preClickBaseline: [],
      }));
      assert.deepEqual(inaccessible, { found: false, proven: false });
    } finally {
      await runtime.close();
    }
  });
  await t.test("WebSocket handshake requires the exact bearer token", async () => {
    const runtime = await setup("happy");
    try {
      assert.equal((await stat(runtime.tokenPath)).mode & 0o777, 0o600);
      assert.equal(await unauthorizedStatus(runtime.daemon.port), 401);
      const wrongToken = `${runtime.token[0] === "0" ? "1" : "0"}${runtime.token.slice(1)}`;
      assert.equal(
        await unauthorizedStatus(runtime.daemon.port, `Bearer ${wrongToken}`),
        401,
      );
      assert.equal((await runtime.rpc.call("health", undefined)).ok, true);
    } finally {
      await runtime.close();
    }
  });
  await t.test("login-wall", async () => {
    const runtime = await setup("login-wall");
    try {
      assert.equal((await runtime.rpc.call("readiness", undefined)).state, "needs_login");
    } finally {
      await runtime.close();
    }
  });
  await t.test("rate-limit", async () => {
    const runtime = await setup("rate-limit");
    try {
      assert.equal((await runtime.rpc.call("readiness", undefined)).state, "provider_limit");
    } finally {
      await runtime.close();
    }
  });
  await t.test("model-missing is a pre-click error", async () => {
    const runtime = await setup("model-missing");
    try {
      await assert.rejects(
        runtime.rpc.call("send", { prompt: "must not send", files: [], newConversation: true }),
        (error: unknown) => error instanceof GwpError
          && error.kind === "model_unavailable"
          && error.phase === "pre_click",
      );
    } finally {
      await runtime.close();
    }
  });
  await t.test("missing aria-checked confirmation is a pre-click error", async () => {
    const runtime = await setup("model-check-fails");
    try {
      await assert.rejects(
        runtime.rpc.call("send", { prompt: "must not send", files: [], newConversation: true }),
        (error: unknown) => error instanceof GwpError
          && error.kind === "model_unavailable"
          && error.phase === "pre_click",
      );
    } finally {
      await runtime.close();
    }
  });
  await t.test("post-click error preserves the first observed user turn id", async () => {
    const runtime = await setup("confirmation-miss");
    try {
      const context = runtime.browser.contexts()[0]!;
      const newPage = context.waitForEvent("page");
      const sending = runtime.rpc.call("send", {
        prompt: "landed before confirmation failed",
        files: [],
        newConversation: true,
      });
      const page = await newPage;
      const user = page.locator('[data-message-author-role="user"]').first();
      await user.waitFor({ state: "visible", timeout: 10_000 });
      const pendingUserTurnId = await user.getAttribute("data-message-id");
      assert.ok(pendingUserTurnId);
      await page.close();
      await assert.rejects(sending, (error: unknown) => error instanceof GwpError
        && error.phase === "post_click"
        && error.pendingUserTurnId === pendingUserTurnId
        && error.preClickBaseline?.length === 0);
    } finally {
      await runtime.close();
    }
  });
  await t.test("code fences and Unicode normalize identically in send and reconcile", async () => {
    const runtime = await setup("markdown-normalization");
    try {
      const prompt = "파이썬 실행\r\n```python\r\n  print(\"안녕 🧪\")  \r\n```\r\n완료";
      const sent = await runtime.rpc.call("send", { prompt, files: [], newConversation: true });
      const page = await pageWithUserTurn(runtime.browser, sent.userTurnId);
      const rendered = await page.locator(`[data-message-id="${sent.userTurnId}"]`).innerText();
      assert.doesNotMatch(rendered, /```/u);
      assert.match(rendered, /^python$/mu);
      assert.match(rendered, /안녕 🧪/u);
      const reconciled = await runtime.rpc.call("reconcile", reconcileRequest(prompt, {
        conversationUrl: sent.conversationUrl,
      }));
      assert.equal(reconciled.userTurnId, sent.userTurnId);
    } finally {
      await runtime.close();
    }
  });
  await t.test("post-stream-gap remains generating until text materializes and stabilizes", async () => {
    const runtime = await setup("post-stream-gap");
    try {
      const prompt = "gap";
      const sent = await runtime.rpc.call("send", { prompt, files: [], newConversation: true });
      const early = await runtime.rpc.call("poll", pollRequest(sent, prompt, 1_000));
      assert.equal(early.state, "generating");
      const terminal = await runtime.rpc.call("poll", pollRequest(sent, prompt, 6_000));
      assert.equal(terminal.state, "complete");
      assert.equal(terminal.answerMarkdown, "fake answer");
    } finally {
      await runtime.close();
    }
  });
  await t.test("attachments accept semantic chips and duplicate UI rename", async () => {
    const runtime = await setup("attachments");
    const left = path.join(runtime.directory, "left");
    const right = path.join(runtime.directory, "right");
    try {
      await Promise.all([mkdir(left), mkdir(right)]);
      await Promise.all([
        writeFile(path.join(left, "same.txt"), "left"),
        writeFile(path.join(right, "same.txt"), "right"),
      ]);
      const sent = await runtime.rpc.call("send", {
        prompt: "attachments",
        files: [
          { name: "same.txt", containerPath: path.join(left, "same.txt") },
          { name: "same.txt", containerPath: path.join(right, "same.txt") },
        ],
        newConversation: true,
      });
      assert.match(sent.conversationUrl, /\/c\//);
      const page = await pageWithUserTurn(runtime.browser, sent.userTurnId);
      const rendered = await page.locator(`[data-message-id="${sent.userTurnId}"]`).innerText();
      assert.match(rendered, /same\.txt\s+File/);
      assert.match(rendered, /same \(1\)\.txt\s+File/);
      assert.ok(rendered.endsWith("attachments"));
      const reconciled = await runtime.rpc.call("reconcile", reconcileRequest("attachments", {
        conversationUrl: sent.conversationUrl,
      }));
      assert.equal(reconciled.userTurnId, sent.userTurnId);
    } finally {
      await runtime.close();
    }
  });
  for (const scenario of ["artifacts", "artifacts-inline"] as const) {
    await t.test(`${scenario} downloads twice and preserves compound extension`, async () => {
      const runtime = await setup(scenario);
      try {
        const prompt = scenario;
        const sent = await runtime.rpc.call("send", { prompt, files: [], newConversation: true });
        const terminal = await runtime.rpc.call("poll", pollRequest(sent, prompt, 7_000));
        assert.equal(terminal.state, "complete");
        assert.deepEqual(terminal.artifactControls?.map((control) => control.label), [
          "report.txt",
          "archive.tar.gz",
        ]);
        const first = await runtime.rpc.call("download", { conversationUrl: sent.conversationUrl, controlIndex: 0 });
        const second = await runtime.rpc.call("download", { conversationUrl: sent.conversationUrl, controlIndex: 1 });
        assert.deepEqual([first.filename, second.filename], ["report.txt", "archive.tar.gz"]);
        assert.equal((await readFile(first.outboxPath, "utf8")), "report from fake ChatGPT\n");
        assert.deepEqual(await readFile(second.outboxPath), Buffer.from("fake-tar-gz"));
      } finally { await runtime.close(); }
    });
  }
  for (const [scenario, answer, waitMs] of [["artifacts-delayed", "numbers.txt.", 12_000],
    ["artifacts-empty", "", 6_000]] as const) {
    await t.test(`${scenario} discovers a delayed filename-only entity`, async () => {
      const runtime = await setup(scenario);
      try {
        const prompt = scenario;
        const sent = await runtime.rpc.call("send", { prompt, files: [], newConversation: true });
        const terminal = await runtime.rpc.call("poll", pollRequest(sent, prompt, waitMs));
        assert.equal(terminal.state, "complete");
        assert.equal(terminal.answerMarkdown, answer);
        assert.equal(terminal.answerSha256, sha256Text(terminal.answerMarkdown));
        assert.deepEqual(terminal.artifactControls, [{ index: 0, label: "numbers.txt" }]);
        const page = await pageWithUserTurn(runtime.browser, sent.userTurnId);
        assert.deepEqual(await artifactControls(page, "stale-assistant-id"), terminal.artifactControls);
        const saved = await runtime.rpc.call("download", { conversationUrl: sent.conversationUrl, controlIndex: 0 });
        assert.equal(saved.filename, "numbers.txt");
        assert.equal(await readFile(saved.outboxPath, "utf8"), "1\n2\n3\n");
      } finally { await runtime.close(); }
    });
  }
  await t.test("answer without a file hint completes before a delayed entity appears", async () => {
    const runtime = await setup("artifacts-no-hint");
    try {
      const prompt = "ordinary answer only";
      const sent = await runtime.rpc.call("send", { prompt, files: [], newConversation: true });
      const terminal = await runtime.rpc.call("poll", pollRequest(sent, prompt, 12_000));
      assert.equal(terminal.state, "complete");
      assert.equal(terminal.answerMarkdown, "ordinary answer");
      assert.deepEqual(terminal.artifactControls, []);
      const page = await pageWithUserTurn(runtime.browser, sent.userTurnId);
      assert.equal(await page.locator('button[aria-label="report.txt"]').count(), 0);
    } finally { await runtime.close(); }
  });
  await t.test("slow exceeds waitMs without terminalizing", async () => {
    const runtime = await setup("slow");
    const second = await RpcClient.connect(runtime.daemon.port, runtime.tokenPath);
    try {
      const prompt = "slow";
      const sent = await runtime.rpc.call("send", { prompt, files: [], newConversation: true });
      const page = await pageWithUserTurn(runtime.browser, sent.userTurnId);
      assert.equal(await page.locator("#intelligence-pill").innerText(), "Pro");
      assert.equal(await page.locator("#intelligence-pill").getAttribute("data-open-count"), "0");
      const order: string[] = [];
      const pollPromise = runtime.rpc.call(
        "poll",
        pollRequest(sent, prompt, 300),
      ).then((result) => {
        order.push("poll");
        return result;
      });
      await new Promise((resolve) => setTimeout(resolve, 25));
      const healthPromise = second.call("health", undefined).then((result) => {
        order.push("health");
        return result;
      });
      const [polled, health] = await Promise.all([pollPromise, healthPromise]);
      assert.deepEqual(polled, {
        state: "generating",
        currentUrl: sent.conversationUrl,
        assistantTurnId: sent.assistantTurnId,
      });
      assert.equal(health.chromeConnected, true);
      assert.deepEqual(order, ["health", "poll"]);
    } finally {
      await second.close().catch(() => undefined);
      await runtime.close();
    }
  });
});
