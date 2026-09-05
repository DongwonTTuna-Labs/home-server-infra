import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import Database from "better-sqlite3";
import { GwpDatabase } from "../../src/supervisor/db.js";
const V1_DDL = `
CREATE TABLE requests (
  id TEXT PRIMARY KEY,
  prompt_sha256 TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN
    ('staged','sending','generating','complete','uncertain','needs_user_action','failed')),
  slot_id TEXT,
  conversation_url TEXT,
  answer_sha256 TEXT,
  error_kind TEXT,
  error_detail TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE TABLE send_attempts (
  request_id TEXT NOT NULL REFERENCES requests(id),
  attempt_no INTEGER NOT NULL,
  state TEXT NOT NULL CHECK (state IN
    ('armed','confirmed','reconciled','no_send_proven','uncertain')),
  user_turn_id TEXT,
  assistant_turn_id TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (request_id, attempt_no)
);
CREATE TABLE artifacts (
  request_id TEXT NOT NULL REFERENCES requests(id),
  filename TEXT NOT NULL,
  path TEXT NOT NULL,
  sha256 TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (request_id, filename)
);
CREATE TABLE slots (
  id TEXT PRIMARY KEY,
  account TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'idle' CHECK (state IN
    ('idle','busy','needs_login','provider_limit')),
  cooldown_until INTEGER,
  last_used_at INTEGER
);
PRAGMA user_version = 1;
`;
test("new databases use the v4 schema, pragmas, constraints, and helpers", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-db-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const db = await GwpDatabase.open(path.join(directory, "db.sqlite"));
  t.after(() => db.close());
  assert.equal(db.connection.pragma("user_version", { simple: true }), 4);
  assert.equal(db.connection.pragma("journal_mode", { simple: true }), "wal");
  assert.equal(db.connection.pragma("busy_timeout", { simple: true }), 5000);
  assert.equal(db.connection.pragma("foreign_keys", { simple: true }), 1);
  const tables = db.connection.prepare(`
    SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name
  `).all().map((row) => (row as { name: string }).name);
  assert.deepEqual(tables, ["artifacts", "image_batches", "image_chunks", "requests", "send_attempts", "slots", "usage_events"]);
  db.syncSlots([{ id: "slot-a", account: "a", port: 19301 }]);
  assert.equal(db.getSlot("slot-a")?.state, "idle");
  assert.throws(
    () => db.connection.prepare("UPDATE slots SET state = 'busy' WHERE id = 'slot-a'").run(),
    /CHECK constraint/,
  );
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
  // 주간 사용량 원장: 요청당 1건, 재기록은 무시, 이동창 집계.
  db.updateRequest(request.id, { slot_id: "slot-a" }, 21);
  assert.equal(db.recordUsage(request.id, "slot-a", "6 Pro", 1_000), true);
  assert.equal(db.recordUsage(request.id, "slot-a", "6 Pro", 2_000), false);
  assert.equal(db.countUsageSince("slot-a", 0), 1);
  assert.equal(db.countUsageSince("slot-a", 1_001), 0);
  assert.equal(db.oldestUsageSince("slot-a", 0), 1_000);
  assert.equal(db.oldestUsageSince("slot-a", 1_001), null);
  assert.deepEqual(db.listUsage("slot-a"), [{ request_id: request.id, slot_id: "slot-a", model_label: "6 Pro", sent_at: 1_000 }]);
  assert.throws(() => db.recordUsage("req_0000000000000009", "slot-a", null, 5), /FOREIGN KEY/);
});
test("v1 busy slots migrate to v4 idle without losing request data", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-db-v1-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const filename = path.join(directory, "db.sqlite");
  const v1 = new Database(filename);
  v1.exec(V1_DDL);
  v1.prepare(`
    INSERT INTO requests
      (id, prompt_sha256, status, slot_id, conversation_url, answer_sha256,
       error_kind, error_detail, created_at, updated_at)
    VALUES (?, ?, 'generating', 'slot-a', ?, NULL, NULL, NULL, 10, 11)
  `).run("req_0000000000000002", "c".repeat(64), "https://chatgpt.com/c/old");
  v1.prepare(`
    INSERT INTO send_attempts
      (request_id, attempt_no, state, user_turn_id, assistant_turn_id, created_at, updated_at)
    VALUES (?, 1, 'confirmed', 'user-1', 'assistant-1', 10, 11)
  `).run("req_0000000000000002");
  v1.prepare(`
    INSERT INTO artifacts (request_id, filename, path, sha256, size_bytes, created_at)
    VALUES (?, 'answer.txt', '/tmp/answer.txt', ?, 7, 12)
  `).run("req_0000000000000002", "d".repeat(64));
  v1.prepare(`
    INSERT INTO slots (id, account, state, cooldown_until, last_used_at)
    VALUES ('slot-a', 'a', 'busy', 123, 456),
           ('slot-b', 'b', 'needs_login', NULL, 789)
  `).run();
  v1.close();
  const migrated = await GwpDatabase.open(filename);
  assert.equal(migrated.connection.pragma("user_version", { simple: true }), 4);
  assert.equal(migrated.countUsageSince("slot-a", 0), 0);
  assert.equal(migrated.getRequest("req_0000000000000002")?.conversation_url,
    "https://chatgpt.com/c/old");
  assert.equal(migrated.getAttempt("req_0000000000000002", 1)?.assistant_turn_id, "assistant-1");
  assert.equal(migrated.listArtifacts("req_0000000000000002")[0]?.size_bytes, 7);
  assert.deepEqual(migrated.getSlot("slot-a"), {
    id: "slot-a",
    account: "a",
    state: "idle",
    cooldown_until: 123,
    last_used_at: 456,
  });
  assert.equal(migrated.getSlot("slot-b")?.state, "needs_login");
  const slotSql = (migrated.connection.prepare(`
    SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'slots'
  `).get() as { sql: string }).sql;
  assert.doesNotMatch(slotSql, /'busy'/);
  assert.throws(
    () => migrated.connection.prepare(`
      INSERT INTO slots (id, account, state) VALUES ('slot-c', 'c', 'busy')
    `).run(),
    /CHECK constraint/,
  );
  migrated.close();
  const reopened = await GwpDatabase.open(filename);
  t.after(() => reopened.close());
  assert.equal(reopened.connection.pragma("user_version", { simple: true }), 4);
  assert.equal(reopened.getSlot("slot-a")?.state, "idle");
  assert.equal(reopened.listAttempts("req_0000000000000002").length, 1);
});
test("slot config sync preserves active orphans and removes inactive ones", async () => {
  const db = await GwpDatabase.open(":memory:");
  const original = [
    { id: "slot-a", account: "old-a", port: 19301 },
    { id: "slot-b", account: "b", port: 19302 },
    { id: "slot-c", account: "c", port: 19303 },
  ];
  db.syncSlots(original);
  db.createRequest("req_0000000000000003", "e".repeat(64), 1);
  db.updateRequest("req_0000000000000003", { slot_id: "slot-b" }, 2);
  db.createRequest("req_0000000000000004", "f".repeat(64), 3);
  db.updateRequest("req_0000000000000004", { slot_id: "slot-c", status: "complete" }, 4);
  db.syncSlots([{ id: "slot-a", account: "new-a", port: 19301 }]);
  assert.deepEqual(db.listSlots().map((slot) => slot.id), ["slot-a", "slot-b"]);
  assert.equal(db.getSlot("slot-a")?.account, "new-a");
  assert.equal(db.getRequest("req_0000000000000004")?.slot_id, "slot-c");
  db.setRequestStatus("req_0000000000000003", "failed");
  db.syncSlots([{ id: "slot-a", account: "new-a", port: 19301 }]);
  assert.deepEqual(db.listSlots().map((slot) => slot.id), ["slot-a"]);
  assert.equal(db.getRequest("req_0000000000000003")?.slot_id, "slot-b");
  db.close();
});

test("v3 migrates to v4 with usage preserved and batch creation rolls back atomically", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "gwp-db-v3-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const filename = path.join(directory, "db.sqlite");
  const initial = await GwpDatabase.open(filename);
  initial.syncSlots([{ id: "slot-a", account: "a", port: 19301 }]);
  initial.createRequest("req_0000000000000020", "b".repeat(64), 10);
  initial.recordUsage("req_0000000000000020", "slot-a", "Pro", 11);
  initial.connection.exec("DROP TABLE image_chunks; DROP TABLE image_batches; PRAGMA user_version = 3;");
  initial.close();
  const db = await GwpDatabase.open(filename);
  t.after(() => db.close());
  assert.equal(db.connection.pragma("user_version", { simple: true }), 4);
  assert.equal(db.countUsageSince("slot-a", 0), 1);
  const chunk = { requestId: "req_0000000000000021", promptSha256: "c".repeat(64), items: [{ id: "one", prompt: "draw one" }] };
  assert.throws(() => db.createImageBatch("img_0000000000000001", [chunk, chunk]), /UNIQUE/);
  assert.equal(db.getRequest(chunk.requestId), undefined);
  assert.deepEqual(db.imageChunks("img_0000000000000001"), []);
  db.createImageBatch("img_0000000000000001", [chunk]);
  assert.equal(db.imageChunks("img_0000000000000001")[0]?.request_id, chunk.requestId);
  assert.equal(db.imageChunkForRequest(chunk.requestId)?.ordinal, 0);
});
test("reap candidates exclude staged requests and rotate by the last reap attempt", async () => {
  const db = await GwpDatabase.open(":memory:");
  db.createRequest("req_0000000000000010", "a".repeat(64), 1);
  db.createRequest("req_0000000000000011", "b".repeat(64), 10);
  db.updateRequest("req_0000000000000011", { status: "generating" }, 20);
  db.createRequest("req_0000000000000012", "c".repeat(64), 11);
  db.updateRequest("req_0000000000000012", { status: "uncertain" }, 30);
  assert.deepEqual(
    db.listReapCandidates(1).map((request) => request.id),
    ["req_0000000000000011"],
  );
  assert.equal(db.touchNonterminalRequest("req_0000000000000011", 40), true);
  assert.deepEqual(
    db.listReapCandidates(1).map((request) => request.id),
    ["req_0000000000000012"],
  );
  db.setRequestStatus("req_0000000000000012", "complete", null, null, 50);
  assert.equal(db.touchNonterminalRequest("req_0000000000000012", 60), false);
  db.close();
});
