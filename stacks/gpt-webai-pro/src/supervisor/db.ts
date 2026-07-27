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
} from "../shared/types.js";

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
CREATE TABLE slots (
  id             TEXT PRIMARY KEY,
  account        TEXT NOT NULL,
  state          TEXT NOT NULL DEFAULT 'idle' CHECK (state IN
                 ('idle','busy','needs_login','provider_limit')),
  cooldown_until INTEGER, last_used_at INTEGER
);
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
    if (current > 1) {
      connection.close();
      throw new Error(`unsupported database user_version ${current}`);
    }
    if (current === 0) {
      connection.exec("BEGIN IMMEDIATE");
      try {
        connection.exec(DDL);
        connection.pragma("user_version = 1");
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
    });
  }

  getSlot(id: string): SlotRow | undefined {
    return this.connection.prepare("SELECT * FROM slots WHERE id = ?")
      .get(id) as SlotRow | undefined;
  }

  listSlots(): SlotRow[] {
    return this.connection.prepare("SELECT * FROM slots ORDER BY id").all() as SlotRow[];
  }

  listNonterminalRequests(): RequestRow[] {
    return this.connection.prepare(`
      SELECT * FROM requests
      WHERE status IN ('staged','sending','generating','uncertain')
      ORDER BY created_at
    `).all() as RequestRow[];
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
