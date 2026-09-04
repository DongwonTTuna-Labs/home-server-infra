import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { GwpDatabase } from "../../src/supervisor/db.js";
import { validateConfig } from "../../src/supervisor/run.js";
import {
  claimSlotForRequest,
  markSlotIdle,
  markSlotNeedsLogin,
  markSlotProviderLimit,
  PROVIDER_COOLDOWN_MS,
  WEEK_MS,
  resolveWeeklyLimits,
  weeklyUsageFor,
} from "../../src/supervisor/slots.js";
import type { RequestStatus, SlotConfig } from "../../src/shared/types.js";
const slots: SlotConfig[] = [
  { id: "slot-a", account: "a", port: 19301 },
  { id: "slot-b", account: "b", port: 19302 },
  { id: "slot-c", account: "c", port: 19303 },
];
function requestId(index: number): string {
  return `req_${index.toString(16).padStart(16, "0")}`;
}
function addRequest(
  db: GwpDatabase,
  index: number,
  slotId: string | null = null,
  status: RequestStatus = "staged",
): string {
  const id = requestId(index);
  db.createRequest(id, index.toString(16).padStart(64, "0"), index);
  if (slotId !== null || status !== "staged") {
    db.updateRequest(id, { slot_id: slotId, status }, index);
  }
  return id;
}
test("slot config requires maxConcurrent and a distinct valid TCP port", () => {
  assert.throws(
    () => validateConfig({ image: "test", maxConcurrent: 0, slots }),
    /invalid slots config/,
  );
  assert.throws(
    () => validateConfig({
      image: "test",
      maxConcurrent: 3,
      slots: [{ id: "slot-a", account: "a", port: 0 }],
    }),
    /invalid or duplicate port/,
  );
  assert.throws(
    () => validateConfig({
      image: "test",
      maxConcurrent: 3,
      slots: [
        { id: "slot-a", account: "a", port: 19301 },
        { id: "slot-b", account: "a", port: 19302 },
      ],
    }),
    /invalid slot config entry/,
  );
  assert.throws(
    () => validateConfig({
      image: "test",
      maxConcurrent: 3,
      slots: [
        { id: "slot-a", account: "a", port: 19301 },
        { id: "slot-b", account: "b", port: 19301 },
      ],
    }),
    /invalid or duplicate port/,
  );
  assert.equal(validateConfig({ image: "test", maxConcurrent: 3, slots }).slots.length, 3);
});
test("allocation orders by occupancy, then oldest use, then slot id", async () => {
  const db = await GwpDatabase.open(":memory:");
  db.syncSlots(slots);
  db.connection.prepare("UPDATE slots SET last_used_at = ? WHERE id = ?").run(50, "slot-a");
  db.connection.prepare("UPDATE slots SET last_used_at = ? WHERE id = ?").run(20, "slot-b");
  db.connection.prepare("UPDATE slots SET last_used_at = ? WHERE id = ?").run(10, "slot-c");
  addRequest(db, 1, "slot-a", "sending");
  addRequest(db, 2, "slot-a", "generating");
  addRequest(db, 3, "slot-b", "uncertain");
  addRequest(db, 4, "slot-c", "staged");
  const target = addRequest(db, 5);
  const claim = claimSlotForRequest(db, slots, 3, target, 100);
  assert.equal(claim.slot?.id, "slot-c");
  assert.equal(claim.request.slot_id, "slot-c");
  assert.equal(db.countActiveForSlot("slot-c"), 2);
  const tieDb = await GwpDatabase.open(":memory:");
  tieDb.syncSlots(slots);
  const tieTarget = addRequest(tieDb, 6);
  assert.equal(claimSlotForRequest(tieDb, slots, 3, tieTarget, 100).slot?.id, "slot-a");
  tieDb.close();
  db.close();
});
test("allocation enforces maxConcurrent using only nonterminal requests", async () => {
  const db = await GwpDatabase.open(":memory:");
  const config = slots.slice(0, 1);
  db.syncSlots(config);
  addRequest(db, 10, "slot-a", "sending");
  addRequest(db, 11, "slot-a", "generating");
  addRequest(db, 12, "slot-a", "complete");
  const target = addRequest(db, 13);
  assert.equal(claimSlotForRequest(db, config, 2, target, 100).slot, null);
  assert.equal(db.getRequest(target)?.slot_id, null);
  db.setRequestStatus(requestId(10), "complete");
  const claimed = claimSlotForRequest(db, config, 2, target, 101);
  assert.equal(claimed.slot?.id, "slot-a");
  assert.equal(db.countActiveForSlot("slot-a"), 2);
  assert.equal(claimSlotForRequest(db, config, 2, target, 102).slot?.id, "slot-a");
  assert.equal(db.countActiveForSlot("slot-a"), 2);
  db.close();
});
test("provider cooldown and needs-login states remain ineligible until recovered", async () => {
  const db = await GwpDatabase.open(":memory:");
  const config = slots.slice(0, 1);
  db.syncSlots(config);
  markSlotProviderLimit(db, "slot-a", 1_000);
  const target = addRequest(db, 20);
  assert.equal(
    claimSlotForRequest(db, config, 3, target, 1_000 + PROVIDER_COOLDOWN_MS - 1).slot,
    null,
  );
  assert.equal(
    claimSlotForRequest(db, config, 3, target, 1_000 + PROVIDER_COOLDOWN_MS).slot?.id,
    "slot-a",
  );
  db.setRequestStatus(target, "complete");
  markSlotNeedsLogin(db, "slot-a");
  const blocked = addRequest(db, 21);
  assert.equal(claimSlotForRequest(db, config, 3, blocked, 9_999_999).slot, null);
  markSlotIdle(db, "slot-a");
  assert.equal(claimSlotForRequest(db, config, 3, blocked, 9_999_999).slot?.id, "slot-a");
  db.close();
});
test("weekly limits exclude exhausted slots on a 7-day sliding window and resolve per slot", async () => {
  const db = await GwpDatabase.open(":memory:");
  db.syncSlots(slots);
  const limits = resolveWeeklyLimits({ weeklyLimit: 2, slots: [slots[0]!, { ...slots[1]!, weeklyLimit: 5 }, slots[2]!] });
  assert.deepEqual([...limits.entries()], [["slot-a", 2], ["slot-b", 5], ["slot-c", 2]]);
  assert.deepEqual([...resolveWeeklyLimits({ slots }).values()], [null, null, null]);
  const now = 10 * WEEK_MS;
  // slot-a: 2건 확정(한도 도달), slot-c: 1건은 창 밖(7일 전 이상), 1건은 창 안.
  for (const [index, slotId, sentAt] of [[1, "slot-a", now - 1_000], [2, "slot-a", now - 2_000], [3, "slot-c", now - WEEK_MS - 1], [4, "slot-c", now - 5]] as const) {
    const id = addRequest(db, index, slotId, "complete");
    assert.equal(db.recordUsage(id, slotId, "6 Pro", sentAt), true);
  }
  const usageA = weeklyUsageFor(db, "slot-a", 2, now);
  assert.deepEqual(usageA, { used: 2, limit: 2, resetAt: now - 2_000 + WEEK_MS, exhausted: true });
  assert.deepEqual(weeklyUsageFor(db, "slot-c", 2, now), { used: 1, limit: 2, resetAt: null, exhausted: false });
  assert.deepEqual(weeklyUsageFor(db, "slot-b", null, now), { used: 0, limit: null, resetAt: null, exhausted: false });
  // 소진된 slot-a는 건너뛰고, 가장 오래 놀았던 슬롯 규칙에 따라 slot-b를 고른다.
  const target = addRequest(db, 10);
  assert.equal(claimSlotForRequest(db, slots, 3, target, now, new Set(), limits).slot?.id, "slot-b");
  // 전부 소진이면 null.
  const tight = resolveWeeklyLimits({ weeklyLimit: 1, slots });
  db.recordUsage(addRequest(db, 11, "slot-b", "complete"), "slot-b", null, now - 10);
  db.recordUsage(addRequest(db, 12, "slot-c", "complete"), "slot-c", null, now - 10);
  assert.equal(claimSlotForRequest(db, slots, 3, addRequest(db, 13), now, new Set(), tight).slot, null);
  // 창이 지나면 다시 후보가 된다.
  assert.equal(claimSlotForRequest(db, slots, 3, addRequest(db, 14), now + WEEK_MS + 1, new Set(), tight).slot?.id, "slot-a");
});
test("slot config validates weeklyLimit as a positive integer", () => {
  assert.throws(
    () => validateConfig({ image: "test", maxConcurrent: 3, weeklyLimit: 0, slots }),
    /invalid weeklyLimit/,
  );
  assert.throws(
    () => validateConfig({ image: "test", maxConcurrent: 3, slots: [{ ...slots[0]!, weeklyLimit: 1.5 }] }),
    /invalid weeklyLimit for slot slot-a/,
  );
  assert.equal(validateConfig({ image: "test", maxConcurrent: 3, weeklyLimit: 200, slots }).weeklyLimit, 200);
});
test("two database connections serialize claims without exceeding capacity", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-slot-race-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const filename = path.join(directory, "db.sqlite");
  const one = await GwpDatabase.open(filename);
  one.syncSlots(slots.slice(0, 2));
  const firstRequest = addRequest(one, 30);
  const secondRequest = addRequest(one, 31);
  const thirdRequest = addRequest(one, 32);
  const two = await GwpDatabase.open(filename);
  t.after(() => one.close());
  t.after(() => two.close());
  const claimed = await Promise.all([
    Promise.resolve().then(() => claimSlotForRequest(one, slots.slice(0, 2), 1, firstRequest, 100)),
    Promise.resolve().then(() => claimSlotForRequest(two, slots.slice(0, 2), 1, secondRequest, 100)),
  ]);
  assert.deepEqual(new Set(claimed.map((result) => result.slot?.id)), new Set(["slot-a", "slot-b"]));
  assert.equal(one.countActiveForSlot("slot-a"), 1);
  assert.equal(one.countActiveForSlot("slot-b"), 1);
  assert.equal(claimSlotForRequest(two, slots.slice(0, 2), 1, thirdRequest, 101).slot, null);
});
