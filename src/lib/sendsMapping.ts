function clampInt(n: number, min: number, max: number): number {
  if (!Number.isFinite(n)) return min;
  return Math.max(min, Math.min(max, Math.trunc(n)));
}

/** Always returns exactly 6 entries, each clamped to `[1, busTotalCount]`. Any missing or
 * non-finite input slot falls back to `min(slotIndex + 1, busTotalCount)`. Ported from
 * `app/renderer/renderer.js:436-444`'s `normalizeC1SendMapping` -- deliberately does not
 * dedupe (two Console 1 sends may legally point at the same Mixing Station bus). */
export function normalizeC1SendMapping(raw: number[] | undefined, busTotalCount: number): number[] {
  const out: number[] = [];
  const maxBus = clampInt(busTotalCount, 1, 512);
  for (let i = 0; i < 6; i++) {
    const v = Number(raw ? raw[i] : undefined);
    out[i] = Number.isFinite(v) ? clampInt(v, 1, maxBus) : Math.min(i + 1, maxBus);
  }
  return out;
}
