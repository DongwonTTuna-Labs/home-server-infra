import type { GwpDatabase } from "./db.js";
import type { RequestRow, SlotConfig, SlotRow } from "../shared/types.js";
const PROVIDER_COOLDOWN_MS = 3 * 60 * 1_000;
function age(value: number | null): number {
  return value === null ? Number.NEGATIVE_INFINITY : value;
}
export function claimSlotForRequest(
  db: GwpDatabase,
  config: SlotConfig[],
  maxConcurrent: number,
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
    const selected = selectSlot(db, db.listSlots(), config, maxConcurrent, now, excluded);
    if (!selected) return { request, slot: null };
    db.connection.prepare("UPDATE slots SET last_used_at = ? WHERE id = ?").run(now, selected.id);
    const claimed = db.connection.prepare(`
      UPDATE requests SET slot_id = ?, updated_at = ?
      WHERE id = ? AND status = 'staged' AND slot_id IS NULL
    `).run(selected.id, now, requestId);
    if (claimed.changes !== 1) throw new Error(`request ${requestId} could not claim a slot`);
    return { request: db.getRequest(requestId)!, slot: selected };
  });
}
function selectSlot(
  db: GwpDatabase,
  rows: SlotRow[],
  config: SlotConfig[],
  maxConcurrent: number,
  now: number,
  excluded: Set<string>,
): SlotConfig | null {
  const configured = new Map(config.map((slot) => [slot.id, slot]));
  const eligible = rows.map((slot) => ({ slot, active: db.countActiveForSlot(slot.id) }))
    .filter(({ slot, active }) => configured.has(slot.id) && !excluded.has(slot.id)
      && active < maxConcurrent && (slot.state === "idle"
        || (slot.state === "provider_limit" && slot.cooldown_until !== null
          && slot.cooldown_until <= now)));
  if (eligible.length === 0) return null;
  const selected = eligible.sort((left, right) => left.active - right.active
    || age(left.slot.last_used_at) - age(right.slot.last_used_at)
    || left.slot.id.localeCompare(right.slot.id))[0]!.slot;
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
export { PROVIDER_COOLDOWN_MS };
