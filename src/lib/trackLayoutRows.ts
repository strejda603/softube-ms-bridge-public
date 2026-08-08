import type { TrackOrderEntry } from "./ipc";

/** One row in the custom-order editor: `[n]` mono (1-based), or `[a, b]` stereo (1-based,
 * ascending). Ported from `app/renderer/renderer.js`'s `state.inputRows`/`busRows` shape. */
export type Row = number[];

export function clampInt(n: number, min: number, max: number): number {
  if (!Number.isFinite(n)) return min;
  return Math.max(min, Math.min(max, Math.trunc(n)));
}

/** Inputs are 0-based on the wire, 1-based in the editor (`renderer.js:530-542`). */
export function configInputOrderToRows(inputTrackOrder: TrackOrderEntry[]): Row[] {
  const rows: Row[] = [];
  for (const entry of inputTrackOrder) {
    if (Array.isArray(entry)) {
      rows.push([entry[0] + 1, entry[1] + 1]);
    } else {
      rows.push([entry + 1]);
    }
  }
  return rows;
}

/** Buses are already 1-based on the wire -- no offset (`renderer.js:544-556`). */
export function configBusOrderToRows(busTrackOrder: TrackOrderEntry[]): Row[] {
  const rows: Row[] = [];
  for (const entry of busTrackOrder) {
    if (Array.isArray(entry)) {
      rows.push([entry[0], entry[1]]);
    } else {
      rows.push([entry]);
    }
  }
  return rows;
}

export function rowsToConfigInputOrder(rows: Row[]): TrackOrderEntry[] {
  const out: TrackOrderEntry[] = [];
  for (const r of rows) {
    if (r.length === 0) continue;
    if (r.length === 1) out.push(r[0] - 1);
    else out.push([r[0] - 1, r[1] - 1]);
  }
  return out;
}

export function rowsToConfigBusOrder(rows: Row[]): TrackOrderEntry[] {
  const out: TrackOrderEntry[] = [];
  for (const r of rows) {
    if (r.length === 0) continue;
    if (r.length === 1) out.push(r[0]);
    else out.push([r[0], r[1]]);
  }
  return out;
}

export function generateSequentialRows(count: number): Row[] {
  const rows: Row[] = [];
  for (let i = 1; i <= count; i++) rows.push([i]);
  return rows;
}

/** Highest number appearing across all rows, or 0 if empty -- used to derive a total count
 * from a loaded config when no explicit total is available (there is no `inputCount`/
 * `busCount` field on the wire -- see the plan's Task 7 notes). */
export function flattenRowsToMax(rows: Row[]): number {
  let maxVal = 0;
  for (const r of rows) {
    for (const v of r) {
      if (Number.isFinite(v)) maxVal = Math.max(maxVal, v);
    }
  }
  return maxVal;
}

export function rowsUsedNumbers(rows: Row[]): Set<number> {
  const used = new Set<number>();
  for (const r of rows) {
    for (const v of r) {
      if (Number.isFinite(v)) used.add(v);
    }
  }
  return used;
}

export function dedupeAndClampRows(rows: Row[], maxValue: number): Row[] {
  const used = new Set<number>();
  const out: Row[] = [];
  for (const r of rows) {
    if (!Array.isArray(r) || r.length === 0) continue;
    const row = r.map((v) => clampInt(v, 1, maxValue));
    if (row.length >= 2) {
      const a = row[0];
      const b = row[1];
      if (a === b) continue;
      const lo = Math.min(a, b);
      const hi = Math.max(a, b);
      if (used.has(lo) || used.has(hi)) continue;
      used.add(lo);
      used.add(hi);
      out.push([lo, hi]);
      continue;
    }
    const v = row[0];
    if (used.has(v)) continue;
    used.add(v);
    out.push([v]);
  }
  return out;
}

export function ensureRowsLength(rows: Row[], desiredLen: number, totalMax: number): Row[] {
  const maxValue = clampInt(totalMax, 1, 512);
  const desired = clampInt(desiredLen, 0, maxValue);
  if (desired === 0) return [];

  let out = dedupeAndClampRows(rows, maxValue);
  if (out.length > desired) out = out.slice(0, desired);

  const used = rowsUsedNumbers(out);
  for (let i = 1; i <= maxValue && out.length < desired; i++) {
    if (used.has(i)) continue;
    used.add(i);
    out.push([i]);
  }
  return out;
}

export function computeRestNumbers(rows: Row[], totalCount: number): number[] {
  const used = rowsUsedNumbers(rows);
  const out: number[] = [];
  for (let i = 1; i <= totalCount; i++) {
    if (!used.has(i)) out.push(i);
  }
  return out;
}

export function canLinkAdjacent(rows: Row[], index: number): boolean {
  const row = rows[index];
  const next = rows[index + 1];
  if (!row || !next) return false;
  if (row.length !== 1 || next.length !== 1) return false;
  return next[0] === row[0] + 1;
}

export function linkRowsAt(rows: Row[], index: number): Row[] {
  if (!canLinkAdjacent(rows, index)) return rows;
  const a = rows[index][0];
  const b = rows[index + 1][0];
  const copy = rows.slice();
  copy.splice(index, 2, [a, b]);
  return copy;
}

export function unlinkRowAt(rows: Row[], index: number): Row[] {
  const row = rows[index];
  if (!row || row.length !== 2) return rows;
  const [a, b] = row;
  const copy = rows.slice();
  copy.splice(index, 1, [a], [b]);
  return copy;
}

export function swapRows(rows: Row[], indexA: number, indexB: number): Row[] {
  if (indexA < 0 || indexA >= rows.length) return rows;
  if (indexB < 0 || indexB >= rows.length) return rows;
  const copy = rows.slice();
  const tmp = copy[indexA];
  copy[indexA] = copy[indexB];
  copy[indexB] = tmp;
  return copy;
}

export function insertRowAt(rows: Row[], index: number, value: number): Row[] {
  const idx = clampInt(index, 0, rows.length);
  const copy = rows.slice();
  copy.splice(idx, 0, [value]);
  return copy;
}

export function removeRowAt(rows: Row[], index: number): Row[] {
  if (index < 0 || index >= rows.length) return rows;
  const copy = rows.slice();
  copy.splice(index, 1);
  return copy;
}

export function moveArrayItem<T>(arr: T[], fromIndex: number, toIndex: number): T[] {
  if (fromIndex === toIndex) return arr;
  if (fromIndex < 0 || fromIndex >= arr.length) return arr;
  if (toIndex < 0 || toIndex > arr.length) return arr;
  const copy = arr.slice();
  const [item] = copy.splice(fromIndex, 1);
  copy.splice(toIndex, 0, item);
  return copy;
}

export function labelFor(kind: "input" | "bus", row: Row): string {
  const a = row[0];
  const b = row.length === 2 ? row[1] : null;
  const prefix = kind === "input" ? "Ch" : "Bus";
  return b ? `${prefix} ${a}+${b}` : `${prefix} ${a}`;
}

export function labelForRest(kind: "input" | "bus", n: number): string {
  const prefix = kind === "input" ? "Ch" : "Bus";
  return `${prefix} ${n}`;
}
