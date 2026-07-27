import type { GwpDatabase } from "./db.js";
import type { RequestRow, SlotConfig, SlotRow } from "../shared/types.js";

const PROVIDER_COOLDOWN_MS = 3 * 60 * 1_000;

function age(value: number | null): number {
  return value === null ? Number.NEGATIVE_INFINITY : value;
}

export function allocateSlot(
  db: GwpDatabase,
  config: SlotConfig[],
  now = Date.now(),
  excluded = new Set<string>(),
): SlotConfig | null {
  return db.immediate(() => {
    const rows = db.listSlots();
    const selected = selectSlot(rows, config, now, excluded);
    if (!selected) return null;
    db.connection.prepare("UPDATE slots SET state = 'busy', last_used_at = ? WHERE id = ?")
      .run(now, selected.id);
    return selected;
  });
}

export function claimSlotForRequest(
  db: GwpDatabase,
  config: SlotConfig[],
  requestId: string,
  now = Date.now(),
  excluded = new Set<string>(),
): { request: RequestRow; slot: SlotConfig | null } {
  return db.immediate(() => {
    const request = db.getRequest(requestId);
    if (!request) throw new Error(`unknown request ${requestId}`);
    if (request.slot_id) {
      return {
        request,
        slot: config.find((slot) => slot.id === request.slot_id) ?? null,
      };
    }
    if (request.status !== "staged") return { request, slot: null };

    const selected = selectSlot(db.listSlots(), config, now, excluded);
    if (!selected) return { request, slot: null };
    db.connection.prepare("UPDATE slots SET state = 'busy', last_used_at = ? WHERE id = ?")
      .run(now, selected.id);
    const claimed = db.connection.prepare(`
      UPDATE requests SET slot_id = ?, updated_at = ?
      WHERE id = ? AND status = 'staged' AND slot_id IS NULL
    `).run(selected.id, now, requestId);
    if (claimed.changes !== 1) throw new Error(`request ${requestId} could not claim a slot`);
    return { request: db.getRequest(requestId)!, slot: selected };
  });
}

function selectSlot(
  rows: SlotRow[],
  config: SlotConfig[],
  now: number,
  excluded: Set<string>,
): SlotConfig | null {
  const configured = new Map(config.map((slot) => [slot.id, slot]));
  const eligible = rows.filter((slot) => (
    configured.has(slot.id)
    && !excluded.has(slot.id)
    && (
      (slot.state === "idle" && (slot.cooldown_until === null || slot.cooldown_until <= now))
      || (slot.state === "provider_limit" && slot.cooldown_until !== null && slot.cooldown_until <= now)
    )
  ));
  if (eligible.length === 0) return null;

  const byAccount = new Map<string, SlotRow[]>();
  for (const slot of rows) {
    if (!configured.has(slot.id)) continue;
    const group = byAccount.get(slot.account) ?? [];
    group.push(slot);
    byAccount.set(slot.account, group);
  }
  const eligibleAccounts = [...new Set(eligible.map((slot) => slot.account))];
  eligibleAccounts.sort((left, right) => {
    const leftLast = Math.max(...(byAccount.get(left) ?? []).map((slot) => age(slot.last_used_at)));
    const rightLast = Math.max(...(byAccount.get(right) ?? []).map((slot) => age(slot.last_used_at)));
    return leftLast - rightLast || left.localeCompare(right);
  });
  const account = eligibleAccounts[0]!;
  const selected = eligible
    .filter((slot) => slot.account === account)
    .sort((left, right) => age(left.last_used_at) - age(right.last_used_at)
      || left.id.localeCompare(right.id))[0]!;
  return configured.get(selected.id)!;
}

export function markSlotNeedsLogin(db: GwpDatabase, slotId: string): void {
  db.connection.prepare(`
    UPDATE slots SET state = 'needs_login', cooldown_until = NULL WHERE id = ?
  `).run(slotId);
}

export function markSlotProviderLimit(
  db: GwpDatabase,
  slotId: string,
  now = Date.now(),
): number {
  const cooldownUntil = now + PROVIDER_COOLDOWN_MS;
  db.connection.prepare(`
    UPDATE slots SET state = 'provider_limit', cooldown_until = ? WHERE id = ?
  `).run(cooldownUntil, slotId);
  return cooldownUntil;
}

export function markSlotIdle(db: GwpDatabase, slotId: string): void {
  db.connection.prepare(`
    UPDATE slots SET state = 'idle', cooldown_until = NULL WHERE id = ?
  `).run(slotId);
}

export function releaseSlotIfUnused(
  db: GwpDatabase,
  slotId: string,
  excludingRequestId?: string,
): boolean {
  if (db.countActiveForSlot(slotId, excludingRequestId) !== 0) return false;
  const result = db.connection.prepare(`
    UPDATE slots SET state = 'idle', cooldown_until = NULL
    WHERE id = ? AND state = 'busy'
  `).run(slotId);
  return result.changes === 1;
}

export function recoverStaleBusySlots(db: GwpDatabase, apply: boolean): string[] {
  const stale = db.listSlots()
    .filter((slot) => slot.state === "busy" && db.countActiveForSlot(slot.id) === 0)
    .map((slot) => slot.id);
  if (apply) {
    const statement = db.connection.prepare("UPDATE slots SET state = 'idle' WHERE id = ?");
    db.immediate(() => {
      for (const slotId of stale) statement.run(slotId);
    });
  }
  return stale;
}

export { PROVIDER_COOLDOWN_MS };
