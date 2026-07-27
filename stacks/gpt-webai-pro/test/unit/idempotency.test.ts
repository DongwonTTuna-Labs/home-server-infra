import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { GwpDatabase } from "../../src/supervisor/db.js";
import {
  applyReconcileResult,
  armSendAttempt,
  confirmSendAttempt,
  markPreClickNoSend,
  markSendUncertain,
} from "../../src/supervisor/run.js";

async function fresh(id: string): Promise<GwpDatabase> {
  const db = await GwpDatabase.open(":memory:");
  db.createRequest(id, "a".repeat(64));
  return db;
}

test("send transition table covers confirmed, pre-click, and post-click outcomes", async (t) => {
  await t.test("armed -> confirmed -> generating", async () => {
    const db = await fresh("req_1000000000000001");
    assert.equal(armSendAttempt(db, "req_1000000000000001"), 1);
    assert.equal(db.getRequest("req_1000000000000001")?.status, "sending");
    confirmSendAttempt(db, "req_1000000000000001", {
      conversationUrl: "https://chatgpt.com/c/one",
      userTurnId: "user-1",
      assistantTurnId: "assistant-1",
    });
    assert.equal(db.getRequest("req_1000000000000001")?.status, "generating");
    assert.equal(db.latestAttempt("req_1000000000000001")?.state, "confirmed");
    db.close();
  });

  await t.test("armed -> no_send_proven on pre-click failure", async () => {
    const db = await fresh("req_1000000000000002");
    armSendAttempt(db, "req_1000000000000002");
    markPreClickNoSend(db, "req_1000000000000002");
    assert.equal(db.latestAttempt("req_1000000000000002")?.state, "no_send_proven");
    db.close();
  });

  await t.test("armed -> uncertain on post-click or transport loss", async () => {
    const db = await fresh("req_1000000000000003");
    armSendAttempt(db, "req_1000000000000003");
    markSendUncertain(db, "req_1000000000000003", "socket lost");
    assert.equal(db.getRequest("req_1000000000000003")?.status, "uncertain");
    assert.equal(db.latestAttempt("req_1000000000000003")?.state, "uncertain");
    db.close();
  });
});

test("armed-death resume reconcile has exactly three fail-closed branches", async (t) => {
  await t.test("found: reconcile without another click", async () => {
    const db = await fresh("req_2000000000000001");
    armSendAttempt(db, "req_2000000000000001");
    markSendUncertain(db, "req_2000000000000001", "daemon died");
    const decision = applyReconcileResult(db, "req_2000000000000001", {
      found: true,
      proven: true,
      conversationUrl: "https://chatgpt.com/c/found",
      userTurnId: "u-found",
      assistantTurnId: "a-found",
    });
    assert.equal(decision, "found");
    assert.equal(db.getRequest("req_2000000000000001")?.status, "generating");
    assert.equal(db.latestAttempt("req_2000000000000001")?.state, "reconciled");
    assert.equal(db.listAttempts("req_2000000000000001").length, 1);
    db.close();
  });

  await t.test("proven absent: exactly one second attempt is allowed", async () => {
    const id = "req_2000000000000002";
    const db = await fresh(id);
    armSendAttempt(db, id);
    markSendUncertain(db, id, "daemon died");
    assert.equal(applyReconcileResult(db, id, { found: false, proven: true }), "retry_send");
    assert.equal(db.getRequest(id)?.status, "staged");
    assert.equal(armSendAttempt(db, id), 2);
    markSendUncertain(db, id, "daemon died again");
    assert.equal(applyReconcileResult(db, id, { found: false, proven: true }), "exhausted");
    assert.equal(db.getRequest(id)?.status, "needs_user_action");
    assert.equal(db.listAttempts(id).length, 2);
    assert.throws(() => armSendAttempt(db, id), /limit exceeded/);
    db.close();
  });

  await t.test("unproven: uncertain remains and no retry is armed", async () => {
    const id = "req_2000000000000003";
    const db = await fresh(id);
    armSendAttempt(db, id);
    markSendUncertain(db, id, "tabs lost");
    assert.equal(applyReconcileResult(db, id, { found: false, proven: false }), "unproven");
    assert.equal(db.getRequest(id)?.status, "uncertain");
    assert.equal(db.latestAttempt(id)?.state, "uncertain");
    assert.equal(db.listAttempts(id).length, 1);
    db.close();
  });
});

test("attempt terminal states cannot be reused as an armed intent", async () => {
  const id = "req_3000000000000001";
  const db = await fresh(id);
  armSendAttempt(db, id);
  markPreClickNoSend(db, id);
  assert.throws(() => confirmSendAttempt(db, id, {
    conversationUrl: "https://chatgpt.com/c/invalid",
    userTurnId: "u",
    assistantTurnId: "a",
  }), /requires an armed attempt/);
  db.close();
});

test("guarded transitions accept existing terminal winners without overwriting them", async (t) => {
  await t.test("confirmed winner is never reverted or replaced by a stale result", async () => {
    const id = "req_3000000000000002";
    const db = await fresh(id);
    armSendAttempt(db, id);
    confirmSendAttempt(db, id, {
      conversationUrl: "https://chatgpt.com/c/winner-confirmed",
      userTurnId: "winner-user",
      assistantTurnId: "winner-assistant",
    });
    assert.equal(markSendUncertain(db, id, "stale timeout"), "confirmed");
    assert.equal(markPreClickNoSend(db, id), "confirmed");
    assert.equal(confirmSendAttempt(db, id, {
      conversationUrl: "https://chatgpt.com/c/stale",
      userTurnId: "stale-user",
      assistantTurnId: "stale-assistant",
    }), "confirmed");
    assert.equal(db.getRequest(id)?.status, "generating");
    assert.equal(db.getRequest(id)?.conversation_url, "https://chatgpt.com/c/winner-confirmed");
    assert.equal(db.latestAttempt(id)?.user_turn_id, "winner-user");
    assert.equal(db.latestAttempt(id)?.assistant_turn_id, "winner-assistant");
    db.close();
  });

  await t.test("reconciled winner is never reverted or replaced by a stale result", async () => {
    const id = "req_3000000000000003";
    const db = await fresh(id);
    armSendAttempt(db, id);
    markSendUncertain(db, id, "owner died");
    applyReconcileResult(db, id, {
      found: true,
      proven: true,
      conversationUrl: "https://chatgpt.com/c/winner-reconciled",
      userTurnId: "reconciled-user",
      assistantTurnId: "reconciled-assistant",
    });
    assert.equal(markSendUncertain(db, id, "stale timeout"), "reconciled");
    assert.equal(confirmSendAttempt(db, id, {
      conversationUrl: "https://chatgpt.com/c/stale",
      userTurnId: "stale-user",
      assistantTurnId: "stale-assistant",
    }), "reconciled");
    assert.equal(db.getRequest(id)?.conversation_url, "https://chatgpt.com/c/winner-reconciled");
    assert.equal(db.latestAttempt(id)?.user_turn_id, "reconciled-user");
    assert.equal(db.latestAttempt(id)?.assistant_turn_id, "reconciled-assistant");
    db.close();
  });
});

test("assistant turn rebinding is guarded by attempt, durable user, and expected prior id", async () => {
  const id = "req_3000000000000004";
  const db = await fresh(id);
  armSendAttempt(db, id);
  confirmSendAttempt(db, id, {
    conversationUrl: "https://chatgpt.com/c/assistant-rebind",
    userTurnId: "durable-user",
    assistantTurnId: "provisional-without-known-prefix",
  });

  const promoted = db.rebindAssistantTurnId(
    id,
    1,
    "durable-user",
    "provisional-without-known-prefix",
    "final-assistant-id",
  );
  assert.equal(promoted.changed, true);
  assert.equal(promoted.row.assistant_turn_id, "final-assistant-id");
  assert.equal(promoted.row.user_turn_id, "durable-user");

  const stale = db.rebindAssistantTurnId(
    id,
    1,
    "durable-user",
    "provisional-without-known-prefix",
    "stale-assistant-id",
  );
  assert.equal(stale.changed, false);
  assert.equal(stale.row.assistant_turn_id, "final-assistant-id");

  const wrongUser = db.rebindAssistantTurnId(
    id,
    1,
    "another-user",
    "final-assistant-id",
    "wrong-user-assistant-id",
  );
  assert.equal(wrongUser.changed, false);
  assert.equal(db.latestAttempt(id)?.user_turn_id, "durable-user");
  assert.equal(db.latestAttempt(id)?.assistant_turn_id, "final-assistant-id");

  db.setRequestStatus(id, "complete");
  const lateAfterComplete = db.rebindAssistantTurnId(
    id,
    1,
    "durable-user",
    "final-assistant-id",
    "provisional-without-known-prefix",
  );
  assert.equal(lateAfterComplete.changed, false);
  assert.equal(lateAfterComplete.row.assistant_turn_id, "final-assistant-id");
  db.close();
});

test("attempt 2 arm is atomic across independent database connections", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-attempt-race-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const filename = path.join(directory, "db.sqlite");
  const one = await GwpDatabase.open(filename);
  const id = "req_4000000000000001";
  one.createRequest(id, "b".repeat(64));
  armSendAttempt(one, id);
  markSendUncertain(one, id, "owner died");
  assert.equal(applyReconcileResult(one, id, { found: false, proven: true }), "retry_send");
  const two = await GwpDatabase.open(filename);
  t.after(() => one.close());
  t.after(() => two.close());

  const contenders = await Promise.allSettled([
    Promise.resolve().then(() => armSendAttempt(one, id)),
    Promise.resolve().then(() => armSendAttempt(two, id)),
  ]);
  assert.equal(contenders.filter((result) => result.status === "fulfilled").length, 1);
  assert.equal(contenders.filter((result) => result.status === "rejected").length, 1);
  assert.equal(one.listAttempts(id).filter((attempt) => attempt.attempt_no === 2).length, 1);
  assert.equal(one.latestAttempt(id)?.state, "armed");
  assert.equal(one.getRequest(id)?.status, "sending");
  assert.equal(one.listAttempts(id).some((attempt) => attempt.attempt_no > 2), false);
});
