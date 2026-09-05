import assert from "node:assert/strict";
import test from "node:test";
import { waitForOwnerLock } from "../../src/supervisor/run.js";

function clock(start = 0) {
  let current = start;
  const sleeps: number[] = [];
  return {
    now: () => current,
    sleep: async (milliseconds: number) => {
      sleeps.push(milliseconds);
      current += milliseconds;
    },
    sleeps,
  };
}

test("owner attach acquires an available lock without waiting", async () => {
  const timer = clock();
  const lock = { id: "owner" };
  const result = await waitForOwnerLock({
    ...timer,
    tryLock: async () => lock,
    isTerminal: () => false,
    deadline: 5_000,
    pollMs: 2_000,
  });
  assert.deepEqual(result, { kind: "acquired", lock });
  assert.deepEqual(timer.sleeps, []);
});

test("owner attach takes over when the previous owner releases its lock", async () => {
  const timer = clock();
  const lock = { id: "successor" };
  let attempts = 0;
  const result = await waitForOwnerLock({
    ...timer,
    tryLock: async () => ++attempts === 3 ? lock : null,
    isTerminal: () => false,
    deadline: 10_000,
    pollMs: 2_000,
  });
  assert.deepEqual(result, { kind: "acquired", lock });
  assert.deepEqual(timer.sleeps, [2_000, 2_000]);
});

test("owner attach observes completion while the original owner still holds its lock", async () => {
  const timer = clock();
  const result = await waitForOwnerLock({
    ...timer,
    tryLock: async () => null,
    isTerminal: () => timer.now() === 2_000,
    deadline: 10_000,
    pollMs: 2_000,
  });
  assert.deepEqual(result, { kind: "terminal" });
  assert.deepEqual(timer.sleeps, [2_000]);
});

test("owner attach caps its final sleep and never acquires a lock after the deadline", async () => {
  const timer = clock();
  let attempts = 0;
  const result = await waitForOwnerLock({
    ...timer,
    tryLock: async () => {
      attempts += 1;
      return timer.now() >= 5_000 ? { id: "too-late" } : null;
    },
    isTerminal: () => false,
    deadline: 5_000,
    pollMs: 2_000,
  });
  assert.deepEqual(result, { kind: "timeout" });
  assert.equal(attempts, 3);
  assert.deepEqual(timer.sleeps, [2_000, 2_000, 1_000]);
  assert.equal(timer.now(), 5_000);
});

test("owner attach makes one immediate acquisition attempt with a zero budget", async () => {
  const timer = clock(100);
  const lock = { id: "immediate" };
  const result = await waitForOwnerLock({
    ...timer,
    tryLock: async () => lock,
    isTerminal: () => false,
    deadline: 100,
    pollMs: 2_000,
  });
  assert.deepEqual(result, { kind: "acquired", lock });
  assert.deepEqual(timer.sleeps, []);
});

test("owner attach returns timeout without sleeping when a zero-budget lock is occupied", async () => {
  const timer = clock();
  const result = await waitForOwnerLock({
    ...timer,
    tryLock: async () => null,
    isTerminal: () => false,
    deadline: 0,
    pollMs: 2_000,
  });
  assert.deepEqual(result, { kind: "timeout" });
  assert.deepEqual(timer.sleeps, []);
});

test("owner attach propagates lock errors instead of hiding them as running", async () => {
  await assert.rejects(waitForOwnerLock({
    tryLock: async () => { throw new Error("lock unavailable"); },
    isTerminal: () => false,
    deadline: 1_000,
    pollMs: 10,
  }), /lock unavailable/);
});
