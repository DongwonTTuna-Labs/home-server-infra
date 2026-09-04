import type { GwpDatabase } from "./db.js";
import type { RequestRow, SlotConfig, SlotRow, SlotsConfig } from "../shared/types.js";
const PROVIDER_COOLDOWN_MS = 3 * 60 * 1_000;
// 주간 한도는 7일 이동창으로 센다 (제공자의 정확한 리셋 시각은 공개되지 않으므로 보수적 근사).
const WEEK_MS = 7 * 24 * 60 * 60 * 1_000;
export interface WeeklyUsage {
  used: number;
  limit: number | null;
  // 한도에 닿았을 때 가장 오래된 전송이 창 밖으로 나가 1건이 풀리는 시각. 무제한/여유 있으면 null.
  resetAt: number | null;
  exhausted: boolean;
}
/** slots.json의 weeklyLimit(공통) / slot.weeklyLimit(개별)을 슬롯별 한도로 푼다. 없으면 null(무제한). */
export function resolveWeeklyLimits(config: Pick<SlotsConfig, "slots" | "weeklyLimit">): Map<string, number | null> {
  return new Map(config.slots.map((slot) => [slot.id, slot.weeklyLimit ?? config.weeklyLimit ?? null]));
}
export function weeklyUsageFor(
  db: GwpDatabase,
  slotId: string,
  limit: number | null,
  now = Date.now(),
): WeeklyUsage {
  const since = now - WEEK_MS;
  const used = db.countUsageSince(slotId, since);
  const exhausted = limit !== null && used >= limit;
  const oldest = exhausted ? db.oldestUsageSince(slotId, since) : null;
  return { used, limit, resetAt: oldest === null ? null : oldest + WEEK_MS, exhausted };
}
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
  weeklyLimits: Map<string, number | null> = new Map(),
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
    const selected = selectSlot(db, db.listSlots(), config, maxConcurrent, now, excluded, weeklyLimits);
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
  weeklyLimits: Map<string, number | null>,
): SlotConfig | null {
  const configured = new Map(config.map((slot) => [slot.id, slot]));
  const eligible = rows.map((slot) => ({ slot, active: db.countActiveForSlot(slot.id) }))
    .filter(({ slot, active }) => configured.has(slot.id) && !excluded.has(slot.id)
      && active < maxConcurrent && (slot.state === "idle"
        || (slot.state === "provider_limit" && slot.cooldown_until !== null
          && slot.cooldown_until <= now))
      // 주간 한도에 닿은 슬롯은 후보에서 뺀다 (다른 계정으로 넘어가고, 전부 소진이면 호출자가 판단).
      && !weeklyUsageFor(db, slot.id, weeklyLimits.get(slot.id) ?? null, now).exhausted);
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
export { PROVIDER_COOLDOWN_MS, WEEK_MS };
