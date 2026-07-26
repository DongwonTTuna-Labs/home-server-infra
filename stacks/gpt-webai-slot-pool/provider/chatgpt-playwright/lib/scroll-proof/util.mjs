
export const DEFAULT_BOTTOM_GAP_EPSILON_PX = 14;
export const DEFAULT_SCROLL_METRIC_EPSILON_PX = 8;

export function clampNumber(value, fallback = 0) {
  const number = Number(value);
  return Number.isFinite(number) ? number : fallback;
}
