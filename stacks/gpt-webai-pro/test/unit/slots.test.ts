import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { GwpDatabase } from "../../src/supervisor/db.js";
import { validateConfig } from "../../src/supervisor/run.js";
import {
  PROVIDER_COOLDOWN_MS,
  allocateSlot,
  markSlotNeedsLogin,
  markSlotProviderLimit,
  releaseSlotIfUnused,
} from "../../src/supervisor/slots.js";
import type { SlotConfig } from "../../src/shared/types.js";

const slots: SlotConfig[] = [
  { id: "slot-01", account: "a", port: 19301 },
  { id: "slot-02", account: "a", port: 19302 },
  { id: "slot-03", account: "b", port: 19303 },
  { id: "slot-04", account: "b", port: 19304 },
  { id: "slot-05", account: "c", port: 19305 },
  { id: "slot-06", account: "c", port: 19306 },
];

test("slot config requires a distinct valid TCP port per slot", () => {
  assert.throws(
    () => validateConfig({ image: "test", slots: [{ id: "slot-01", account: "a", port: 0 }] }),
    /invalid or duplicate port/,
  );
  assert.throws(
    () => validateConfig({
      image: "test",
      slots: [
        { id: "slot-01", account: "a", port: 19301 },
        { id: "slot-02", account: "b", port: 19301 },
      ],
    }),
    /invalid or duplicate port/,
  );
  assert.equal(validateConfig({ image: "test", slots }).slots.length, slots.length);
});

test("slot allocation rotates accounts by LRU then slots by LRU", async () => {
  const db = await GwpDatabase.open(":memory:");
  db.syncSlots(slots);
  const sequence: string[] = [];
  for (let index = 0; index < 6; index += 1) {
    const selected = allocateSlot(db, slots, 100 + index);
    assert.ok(selected);
    sequence.push(selected.id);
    assert.equal(releaseSlotIfUnused(db, selected.id), true);
  }
  assert.deepEqual(sequence, ["slot-01", "slot-03", "slot-05", "slot-02", "slot-04", "slot-06"]);
  db.close();
});

test("provider cooldown and needs-login states are excluded until eligible", async () => {
  const db = await GwpDatabase.open(":memory:");
  const config = [{ id: "slot-01", account: "a", port: 19301 }];
  db.syncSlots(config);
  const first = allocateSlot(db, config, 1_000)!;
  markSlotProviderLimit(db, first.id, 1_000);
  assert.equal(allocateSlot(db, config, 1_000 + PROVIDER_COOLDOWN_MS - 1), null);
  assert.equal(allocateSlot(db, config, 1_000 + PROVIDER_COOLDOWN_MS)?.id, "slot-01");
  releaseSlotIfUnused(db, "slot-01");
  markSlotNeedsLogin(db, "slot-01");
  assert.equal(allocateSlot(db, config, 9_999_999), null);
  db.close();
});

test("two independent database connections never claim the same slot", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-slot-race-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const filename = path.join(directory, "db.sqlite");
  const one = await GwpDatabase.open(filename);
  one.syncSlots(slots.slice(0, 2));
  const two = await GwpDatabase.open(filename);
  t.after(() => one.close());
  t.after(() => two.close());

  const claimed = await Promise.all([
    Promise.resolve().then(() => allocateSlot(one, slots.slice(0, 2), 100)),
    Promise.resolve().then(() => allocateSlot(two, slots.slice(0, 2), 100)),
  ]);
  assert.equal(new Set(claimed.map((slot) => slot?.id)).size, 2);
});
