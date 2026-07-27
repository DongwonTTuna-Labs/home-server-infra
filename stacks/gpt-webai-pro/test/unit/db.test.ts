import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { GwpDatabase } from "../../src/supervisor/db.js";

test("SQLite v1 schema, pragmas, constraints, and query helpers", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-db-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const db = await GwpDatabase.open(path.join(directory, "db.sqlite"));
  t.after(() => db.close());

  assert.equal(db.connection.pragma("user_version", { simple: true }), 1);
  assert.equal(db.connection.pragma("journal_mode", { simple: true }), "wal");
  assert.equal(db.connection.pragma("busy_timeout", { simple: true }), 5000);
  assert.equal(db.connection.pragma("foreign_keys", { simple: true }), 1);
  const tables = db.connection.prepare(`
    SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name
  `).all().map((row) => (row as { name: string }).name);
  assert.deepEqual(tables, ["artifacts", "requests", "send_attempts", "slots"]);

  const request = db.createRequest("req_0000000000000001", "a".repeat(64), 10);
  assert.equal(request.status, "staged");
  assert.equal(request.created_at, 10);
  assert.throws(() => db.setRequestStatus(request.id, "bogus" as never), /CHECK constraint/);

  assert.equal(db.createAttempt(request.id, 1, 11).state, "armed");
  assert.equal(
    db.transitionAttempt(request.id, 1, ["armed"], "no_send_proven", {}, 12).changed,
    true,
  );
  assert.equal(
    db.transitionAttempt(request.id, 1, ["armed"], "uncertain", {}, 13).changed,
    false,
  );
  assert.equal(db.createAttempt(request.id, 2, 13).attempt_no, 2);
  assert.throws(() => db.createAttempt(request.id, 3), /must be 1 or 2/);

  db.addArtifact({
    request_id: request.id,
    filename: "answer.tar.gz",
    path: "/tmp/answer.tar.gz",
    sha256: "b".repeat(64),
    size_bytes: 42,
    created_at: 20,
  });
  assert.equal(db.listArtifacts(request.id)[0]?.filename, "answer.tar.gz");
});

test("database migration is idempotent", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-db-reopen-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const filename = path.join(directory, "db.sqlite");
  const first = await GwpDatabase.open(filename);
  first.createRequest("req_0000000000000002", "c".repeat(64));
  first.close();
  const second = await GwpDatabase.open(filename);
  t.after(() => second.close());
  assert.equal(second.getRequest("req_0000000000000002")?.status, "staged");
  assert.equal(second.connection.pragma("user_version", { simple: true }), 1);
});
