import Database from "better-sqlite3";
import path from "node:path";
import { mkdirp } from "../shared/fsx.js";
import type {
  ArtifactRow,
  RequestRow,
  RequestStatus,
  SendAttemptRow,
  SendAttemptState,
  SlotConfig,
  SlotRow,
  UsageEventRow,
  ImageChunkRow,
  ImagePrompt,
} from "../shared/types.js";
const SLOTS_DDL = `
CREATE TABLE slots (
  id             TEXT PRIMARY KEY,
  account        TEXT NOT NULL,
  state          TEXT NOT NULL DEFAULT 'idle' CHECK (state IN
                 ('idle','needs_login','provider_limit')),
  cooldown_until INTEGER, last_used_at INTEGER
);`;
// 주간 사용량 원장: 확정(confirmed/reconciled)된 전송 1건 = 1행. request_id가 기본키라
// 재확정·resume에서 중복 계상되지 않는다. 7일 이동창 집계는 (slot_id, sent_at) 색인으로 한다.
const USAGE_DDL = `
CREATE TABLE usage_events (
  request_id  TEXT PRIMARY KEY REFERENCES requests(id),
  slot_id     TEXT NOT NULL REFERENCES slots(id),
  model_label TEXT,
  sent_at     INTEGER NOT NULL
);
CREATE INDEX usage_events_slot_sent ON usage_events (slot_id, sent_at);`;
const IMAGE_BATCH_DDL = `
CREATE TABLE image_batches (id TEXT PRIMARY KEY, created_at INTEGER NOT NULL);
CREATE TABLE image_chunks (
  batch_id TEXT NOT NULL REFERENCES image_batches(id),
  ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
  request_id TEXT NOT NULL UNIQUE REFERENCES requests(id),
  items_json TEXT NOT NULL CHECK (json_valid(items_json) AND json_array_length(items_json) BETWEEN 1 AND 5),
  PRIMARY KEY (batch_id, ordinal)
);`;
const DDL = `
CREATE TABLE requests (
  id            TEXT PRIMARY KEY,
  prompt_sha256 TEXT NOT NULL,
  status        TEXT NOT NULL CHECK (status IN
                ('staged','sending','generating','complete',
                 'uncertain','needs_user_action','failed')),
  slot_id       TEXT,
  conversation_url TEXT,
  answer_sha256 TEXT,
  error_kind    TEXT, error_detail TEXT,
  created_at    INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE send_attempts (
  request_id  TEXT NOT NULL REFERENCES requests(id),
  attempt_no  INTEGER NOT NULL,
  state       TEXT NOT NULL CHECK (state IN
              ('armed','confirmed','reconciled','no_send_proven','uncertain')),
  user_turn_id TEXT, assistant_turn_id TEXT,
  created_at  INTEGER NOT NULL, updated_at INTEGER NOT NULL,
  PRIMARY KEY (request_id, attempt_no)
);
CREATE TABLE artifacts (
  request_id TEXT NOT NULL REFERENCES requests(id),
  filename   TEXT NOT NULL, path TEXT NOT NULL,
  sha256     TEXT NOT NULL, size_bytes INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (request_id, filename)
);
${SLOTS_DDL}
${USAGE_DDL}
${IMAGE_BATCH_DDL}
`;
type RequestPatch = Partial<Pick<
  RequestRow,
  | "status"
  | "slot_id"
  | "conversation_url"
  | "answer_sha256"
  | "error_kind"
  | "error_detail"
>>;
export class GwpDatabase {
  readonly connection: Database.Database;
  private constructor(connection: Database.Database) {
    this.connection = connection;
  }
  static async open(filename: string): Promise<GwpDatabase> {
    if (filename !== ":memory:") await mkdirp(path.dirname(filename));
    const connection = new Database(filename);
    connection.pragma("journal_mode = WAL");
    connection.pragma("busy_timeout = 5000");
    connection.pragma("foreign_keys = ON");
    const current = Number(connection.pragma("user_version", { simple: true }));
    if (current > 4) {
      connection.close();
      throw new Error(`unsupported database user_version ${current}`);
    }
    if (current < 4) {
      connection.exec("BEGIN IMMEDIATE");
      try {
        if (current === 0) connection.exec(DDL);
        else {
          if (current === 1) connection.exec(`
            ALTER TABLE slots RENAME TO slots_v1;
            ${SLOTS_DDL}
            INSERT INTO slots (id, account, state, cooldown_until, last_used_at)
            SELECT id, account, CASE state WHEN 'busy' THEN 'idle' ELSE state END,
                   cooldown_until, last_used_at
            FROM slots_v1;
            DROP TABLE slots_v1;
          `);
          // v2 → v3: 주간 사용량 원장 추가 (기존 행은 계상하지 않는다 — 과거 전송은 증거가 없다).
          if (current < 3) connection.exec(USAGE_DDL);
          connection.exec(IMAGE_BATCH_DDL);
        }
        connection.pragma("user_version = 4");
        connection.exec("COMMIT");
      } catch (error) {
        connection.exec("ROLLBACK");
        connection.close();
        throw error;
      }
    }
    return new GwpDatabase(connection);
  }
  close(): void {
    this.connection.close();
  }
  createImageBatch(id: string, chunks: Array<{ requestId: string; promptSha256: string; items: ImagePrompt[] }>): void {
    this.immediate(() => {
      this.connection.prepare("INSERT INTO image_batches VALUES (?, ?)").run(id, Date.now());
      const insert = this.connection.prepare("INSERT INTO image_chunks VALUES (?, ?, ?, ?)");
      for (const [index, chunk] of chunks.entries()) {
        this.createRequest(chunk.requestId, chunk.promptSha256);
        insert.run(id, index, chunk.requestId, JSON.stringify(chunk.items));
      }
    });
  }
  imageChunks(id: string): ImageChunkRow[] {
    return this.connection.prepare("SELECT * FROM image_chunks WHERE batch_id = ? ORDER BY ordinal").all(id) as ImageChunkRow[];
  }
  imageChunkForRequest(id: string): ImageChunkRow | undefined {
    return this.connection.prepare("SELECT * FROM image_chunks WHERE request_id = ?").get(id) as ImageChunkRow | undefined;
  }
  immediate<T>(operation: () => T): T {
    this.connection.exec("BEGIN IMMEDIATE");
    try {
      const result = operation();
      this.connection.exec("COMMIT");
      return result;
    } catch (error) {
      this.connection.exec("ROLLBACK");
      throw error;
    }
  }
  createRequest(id: string, promptSha256: string, now = Date.now()): RequestRow {
    this.connection.prepare(`
      INSERT INTO requests
        (id, prompt_sha256, status, created_at, updated_at)
      VALUES (?, ?, 'staged', ?, ?)
    `).run(id, promptSha256, now, now);
    return this.getRequest(id)!;
  }
  getRequest(id: string): RequestRow | undefined {
    return this.connection.prepare("SELECT * FROM requests WHERE id = ?")
      .get(id) as RequestRow | undefined;
  }
  updateRequest(id: string, patch: RequestPatch, now = Date.now()): RequestRow {
    const allowed = [
      "status",
      "slot_id",
      "conversation_url",
      "answer_sha256",
      "error_kind",
      "error_detail",
    ] as const;
    const entries = allowed
      .filter((key) => Object.prototype.hasOwnProperty.call(patch, key))
      .map((key) => [key, patch[key]] as const);
    if (entries.length === 0) return this.requireRequest(id);
    const assignments = entries.map(([key]) => `${key} = ?`).join(", ");
    this.connection.prepare(`UPDATE requests SET ${assignments}, updated_at = ? WHERE id = ?`)
      .run(...entries.map(([, value]) => value), now, id);
    return this.requireRequest(id);
  }
  setRequestStatus(
    id: string,
    status: RequestStatus,
    errorKind: string | null = null,
    errorDetail: string | null = null,
    now = Date.now(),
  ): RequestRow {
    return this.updateRequest(id, {
      status,
      error_kind: errorKind,
      error_detail: errorDetail,
    }, now);
  }
  createAttempt(requestId: string, attemptNo: number, now = Date.now()): SendAttemptRow {
    if (attemptNo !== 1 && attemptNo !== 2) throw new Error("send attempt must be 1 or 2");
    this.connection.prepare(`
      INSERT INTO send_attempts
        (request_id, attempt_no, state, created_at, updated_at)
      VALUES (?, ?, 'armed', ?, ?)
    `).run(requestId, attemptNo, now, now);
    return this.getAttempt(requestId, attemptNo)!;
  }
  getAttempt(requestId: string, attemptNo: number): SendAttemptRow | undefined {
    return this.connection.prepare(`
      SELECT * FROM send_attempts WHERE request_id = ? AND attempt_no = ?
    `).get(requestId, attemptNo) as SendAttemptRow | undefined;
  }
  latestAttempt(requestId: string): SendAttemptRow | undefined {
    return this.connection.prepare(`
      SELECT * FROM send_attempts WHERE request_id = ? ORDER BY attempt_no DESC LIMIT 1
    `).get(requestId) as SendAttemptRow | undefined;
  }
  listAttempts(requestId: string): SendAttemptRow[] {
    return this.connection.prepare(`
      SELECT * FROM send_attempts WHERE request_id = ? ORDER BY attempt_no
    `).all(requestId) as SendAttemptRow[];
  }
  transitionAttempt(
    requestId: string,
    attemptNo: number,
    from: readonly SendAttemptState[],
    state: SendAttemptState,
    ids: { userTurnId?: string | null; assistantTurnId?: string | null } = {},
    now = Date.now(),
  ): { changed: boolean; row: SendAttemptRow } {
    if (from.length === 0) throw new Error("guarded attempt transition needs a source state");
    const assignments = ["state = ?", "updated_at = ?"];
    const values: unknown[] = [state, now];
    if (ids.userTurnId !== undefined) {
      assignments.push("user_turn_id = ?");
      values.push(ids.userTurnId);
    }
    if (ids.assistantTurnId !== undefined) {
      assignments.push("assistant_turn_id = ?");
      values.push(ids.assistantTurnId);
    }
    const guards = from.map(() => "?").join(", ");
    const result = this.connection.prepare(`
      UPDATE send_attempts
      SET ${assignments.join(", ")}
      WHERE request_id = ? AND attempt_no = ? AND state IN (${guards})
    `).run(...values, requestId, attemptNo, ...from);
    const row = this.getAttempt(requestId, attemptNo);
    if (!row) throw new Error(`missing send attempt ${requestId}/${attemptNo}`);
    return { changed: result.changes === 1, row };
  }
  rebindAssistantTurnId(
    requestId: string,
    attemptNo: number,
    userTurnId: string,
    expectedAssistantTurnId: string | null,
    observedAssistantTurnId: string,
    now = Date.now(),
  ): { changed: boolean; row: SendAttemptRow } {
    return this.immediate(() => {
      const current = this.getAttempt(requestId, attemptNo);
      if (!current) throw new Error(`missing send attempt ${requestId}/${attemptNo}`);
      const request = this.getRequest(requestId);
      if (request?.status !== "generating"
        || (current.state !== "confirmed" && current.state !== "reconciled")
        || current.user_turn_id !== userTurnId
        || current.assistant_turn_id !== expectedAssistantTurnId
        || current.assistant_turn_id === observedAssistantTurnId) {
        return { changed: false, row: current };
      }
      const result = this.connection.prepare(`
        UPDATE send_attempts
        SET assistant_turn_id = ?, updated_at = ?
        WHERE request_id = ? AND attempt_no = ?
          AND state IN ('confirmed','reconciled')
          AND user_turn_id = ? AND assistant_turn_id IS ?
          AND EXISTS (
            SELECT 1 FROM requests
            WHERE id = ? AND status = 'generating'
          )
      `).run(
        observedAssistantTurnId,
        now,
        requestId,
        attemptNo,
        userTurnId,
        expectedAssistantTurnId,
        requestId,
      );
      const row = this.getAttempt(requestId, attemptNo);
      if (!row) throw new Error(`missing send attempt ${requestId}/${attemptNo}`);
      return { changed: result.changes === 1, row };
    });
  }
  addArtifact(row: ArtifactRow): void {
    this.connection.prepare(`
      INSERT INTO artifacts
        (request_id, filename, path, sha256, size_bytes, created_at)
      VALUES (?, ?, ?, ?, ?, ?)
      ON CONFLICT(request_id, filename) DO UPDATE SET
        path = excluded.path,
        sha256 = excluded.sha256,
        size_bytes = excluded.size_bytes,
        created_at = excluded.created_at
    `).run(
      row.request_id,
      row.filename,
      row.path,
      row.sha256,
      row.size_bytes,
      row.created_at,
    );
  }
  listArtifacts(requestId: string): ArtifactRow[] {
    return this.connection.prepare(`
      SELECT * FROM artifacts WHERE request_id = ? ORDER BY filename
    `).all(requestId) as ArtifactRow[];
  }
  syncSlots(slots: SlotConfig[]): void {
    const statement = this.connection.prepare(`
      INSERT INTO slots (id, account) VALUES (?, ?)
      ON CONFLICT(id) DO UPDATE SET account = excluded.account
    `);
    this.immediate(() => {
      for (const slot of slots) statement.run(slot.id, slot.account);
      const configured = new Set(slots.map((slot) => slot.id));
      for (const row of this.listSlots()) {
        if (!configured.has(row.id) && this.countActiveForSlot(row.id) === 0) {
          this.connection.prepare("DELETE FROM slots WHERE id = ?").run(row.id);
        }
      }
    });
  }
  getSlot(id: string): SlotRow | undefined {
    return this.connection.prepare("SELECT * FROM slots WHERE id = ?")
      .get(id) as SlotRow | undefined;
  }
  listSlots(): SlotRow[] {
    return this.connection.prepare("SELECT * FROM slots ORDER BY id").all() as SlotRow[];
  }
  /** 확정된 전송을 주간 사용량 원장에 1건 기록한다. 같은 요청은 한 번만 계상된다. */
  recordUsage(requestId: string, slotId: string, modelLabel: string | null, now = Date.now()): boolean {
    const result = this.connection.prepare(`
      INSERT OR IGNORE INTO usage_events (request_id, slot_id, model_label, sent_at)
      VALUES (?, ?, ?, ?)
    `).run(requestId, slotId, modelLabel, now);
    return result.changes === 1;
  }
  /** since(포함) 이후 슬롯의 확정 전송 수. */
  countUsageSince(slotId: string, since: number): number {
    const row = this.connection.prepare(`
      SELECT COUNT(*) AS n FROM usage_events WHERE slot_id = ? AND sent_at >= ?
    `).get(slotId, since) as { n: number };
    return row.n;
  }
  /** since(포함) 이후 가장 오래된 전송 시각 — 이동창에서 다음으로 빠져나갈 시각의 근거. */
  oldestUsageSince(slotId: string, since: number): number | null {
    const row = this.connection.prepare(`
      SELECT MIN(sent_at) AS t FROM usage_events WHERE slot_id = ? AND sent_at >= ?
    `).get(slotId, since) as { t: number | null };
    return row.t;
  }
  listUsage(slotId: string, since = 0): UsageEventRow[] {
    return this.connection.prepare(`
      SELECT * FROM usage_events WHERE slot_id = ? AND sent_at >= ? ORDER BY sent_at
    `).all(slotId, since) as UsageEventRow[];
  }
  listNonterminalRequests(): RequestRow[] {
    return this.connection.prepare(`
      SELECT * FROM requests
      WHERE status IN ('staged','sending','generating','uncertain')
      ORDER BY created_at
    `).all() as RequestRow[];
  }
  listActiveForSlot(slotId: string): RequestRow[] {
    return this.connection.prepare(`
      SELECT * FROM requests
      WHERE slot_id = ?
        AND status IN ('staged','sending','generating','uncertain')
      ORDER BY created_at, id
    `).all(slotId) as RequestRow[];
  }
  listReapCandidates(limit = 1): RequestRow[] {
    if (!Number.isInteger(limit) || limit < 0) throw new Error("reap limit must be non-negative");
    return this.connection.prepare(`
      SELECT * FROM requests
      WHERE status IN ('sending','generating','uncertain')
      ORDER BY updated_at, created_at, id
      LIMIT ?
    `).all(limit) as RequestRow[];
  }
  touchNonterminalRequest(id: string, now = Date.now()): boolean {
    const result = this.connection.prepare(`
      UPDATE requests SET updated_at = ?
      WHERE id = ? AND status IN ('sending','generating','uncertain')
    `).run(now, id);
    return result.changes === 1;
  }
  countActiveForSlot(slotId: string, excludingRequestId?: string): number {
    const suffix = excludingRequestId ? " AND id <> ?" : "";
    const parameters = excludingRequestId ? [slotId, excludingRequestId] : [slotId];
    const row = this.connection.prepare(`
      SELECT COUNT(*) AS count FROM requests
      WHERE slot_id = ?
        AND status IN ('staged','sending','generating','uncertain')${suffix}
    `).get(...parameters) as { count: number };
    return Number(row.count);
  }
  private requireRequest(id: string): RequestRow {
    const row = this.getRequest(id);
    if (!row) throw new Error(`unknown request ${id}`);
    return row;
  }
}
export { DDL };
