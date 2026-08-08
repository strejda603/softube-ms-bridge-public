import { describe, expect, it } from "vitest";
import type { TrackOrderEntry } from "./ipc";
import {
  canLinkAdjacent,
  clampInt,
  computeRestNumbers,
  configBusOrderToRows,
  configInputOrderToRows,
  dedupeAndClampRows,
  ensureRowsLength,
  flattenRowsToMax,
  generateSequentialRows,
  insertRowAt,
  labelFor,
  labelForRest,
  linkRowsAt,
  moveArrayItem,
  removeRowAt,
  rowsToConfigBusOrder,
  rowsToConfigInputOrder,
  rowsUsedNumbers,
  swapRows,
  unlinkRowAt,
} from "./trackLayoutRows";

describe("clampInt", () => {
  it("clamps within range", () => {
    expect(clampInt(5, 1, 10)).toBe(5);
    expect(clampInt(-3, 1, 10)).toBe(1);
    expect(clampInt(99, 1, 10)).toBe(10);
  });
  it("truncates non-integers", () => {
    expect(clampInt(5.9, 0, 10)).toBe(5);
  });
  it("falls back to min for non-finite input", () => {
    expect(clampInt(NaN, 2, 10)).toBe(2);
    expect(clampInt(Infinity, 2, 10)).toBe(2);
  });
});

describe("configInputOrderToRows / rowsToConfigInputOrder (0-based wire, 1-based rows)", () => {
  it("converts a mono entry", () => {
    expect(configInputOrderToRows([5])).toEqual([[6]]);
    expect(rowsToConfigInputOrder([[6]])).toEqual([5]);
  });
  it("converts a stereo pair entry", () => {
    expect(configInputOrderToRows([[0, 1]])).toEqual([[1, 2]]);
    expect(rowsToConfigInputOrder([[1, 2]])).toEqual([[0, 1]]);
  });
  it("round-trips a mixed list", () => {
    const wire: TrackOrderEntry[] = [0, [1, 2], 3];
    expect(rowsToConfigInputOrder(configInputOrderToRows(wire))).toEqual(wire);
  });
});

describe("configBusOrderToRows / rowsToConfigBusOrder (1-based, no offset)", () => {
  it("converts a mono entry with no offset", () => {
    expect(configBusOrderToRows([5])).toEqual([[5]]);
    expect(rowsToConfigBusOrder([[5]])).toEqual([5]);
  });
  it("converts a stereo pair entry with no offset", () => {
    expect(configBusOrderToRows([[2, 3]])).toEqual([[2, 3]]);
    expect(rowsToConfigBusOrder([[2, 3]])).toEqual([[2, 3]]);
  });
});

describe("generateSequentialRows", () => {
  it("generates 1..n mono rows", () => {
    expect(generateSequentialRows(3)).toEqual([[1], [2], [3]]);
  });
  it("generates an empty list for 0", () => {
    expect(generateSequentialRows(0)).toEqual([]);
  });
});

describe("flattenRowsToMax", () => {
  it("finds the highest number across mono and stereo rows", () => {
    expect(flattenRowsToMax([[1], [5, 6], [3]])).toBe(6);
  });
  it("returns 0 for an empty list", () => {
    expect(flattenRowsToMax([])).toBe(0);
  });
});

describe("rowsUsedNumbers", () => {
  it("collects every number across all rows", () => {
    expect(rowsUsedNumbers([[1], [2, 3]])).toEqual(new Set([1, 2, 3]));
  });
});

describe("dedupeAndClampRows", () => {
  it("drops a stereo row whose two values are equal after clamping", () => {
    expect(dedupeAndClampRows([[2, 2]], 10)).toEqual([]);
  });
  it("clamps values to maxValue and sorts a stereo pair ascending", () => {
    expect(dedupeAndClampRows([[9, 3]], 5)).toEqual([[3, 5]]);
  });
  it("drops a later row that reuses an already-used number", () => {
    expect(dedupeAndClampRows([[1], [1]], 10)).toEqual([[1]]);
    expect(dedupeAndClampRows([[1, 2], [2, 3]], 10)).toEqual([[1, 2]]);
  });
});

describe("ensureRowsLength", () => {
  it("returns an empty list when desired length is 0", () => {
    expect(ensureRowsLength([[1], [2]], 0, 10)).toEqual([]);
  });
  it("truncates when shrinking", () => {
    expect(ensureRowsLength([[1], [2], [3]], 2, 10)).toEqual([[1], [2]]);
  });
  it("pads with the lowest unused numbers when growing", () => {
    expect(ensureRowsLength([[3]], 3, 10)).toEqual([[3], [1], [2]]);
  });
});

describe("computeRestNumbers", () => {
  it("lists every number 1..totalCount not present in rows", () => {
    expect(computeRestNumbers([[1], [3]], 4)).toEqual([2, 4]);
  });
  it("returns an empty list when everything is used", () => {
    expect(computeRestNumbers([[1], [2]], 2)).toEqual([]);
  });
});

describe("canLinkAdjacent", () => {
  it("is true for two consecutive mono rows", () => {
    expect(canLinkAdjacent([[1], [2]], 0)).toBe(true);
  });
  it("is false when the numbers aren't consecutive", () => {
    expect(canLinkAdjacent([[1], [3]], 0)).toBe(false);
  });
  it("is false when either row is already stereo", () => {
    expect(canLinkAdjacent([[1, 2], [3]], 0)).toBe(false);
  });
  it("is false at the last index (no next row)", () => {
    expect(canLinkAdjacent([[1]], 0)).toBe(false);
  });
});

describe("linkRowsAt / unlinkRowAt", () => {
  it("merges two adjacent mono rows into one stereo row", () => {
    expect(linkRowsAt([[1], [2], [5]], 0)).toEqual([[1, 2], [5]]);
  });
  it("is a no-op when the rows aren't linkable", () => {
    const rows = [[1], [3]];
    expect(linkRowsAt(rows, 0)).toEqual(rows);
  });
  it("splits a stereo row back into two mono rows at the same position", () => {
    expect(unlinkRowAt([[1, 2], [5]], 0)).toEqual([[1], [2], [5]]);
  });
  it("is a no-op on a mono row", () => {
    const rows = [[1], [2]];
    expect(unlinkRowAt(rows, 0)).toEqual(rows);
  });
});

describe("swapRows", () => {
  it("swaps two rows by index", () => {
    expect(swapRows([[1], [2], [3]], 0, 1)).toEqual([[2], [1], [3]]);
  });
  it("is a no-op for an out-of-range index", () => {
    const rows = [[1], [2]];
    expect(swapRows(rows, 0, 5)).toEqual(rows);
  });
});

describe("insertRowAt / removeRowAt", () => {
  it("inserts a new mono row at a position", () => {
    expect(insertRowAt([[1], [2]], 1, 9)).toEqual([[1], [9], [2]]);
  });
  it("clamps the insert index into range", () => {
    expect(insertRowAt([[1]], 99, 9)).toEqual([[1], [9]]);
  });
  it("removes a row by index", () => {
    expect(removeRowAt([[1], [2], [3]], 1)).toEqual([[1], [3]]);
  });
  it("is a no-op for an out-of-range index", () => {
    const rows = [[1], [2]];
    expect(removeRowAt(rows, 5)).toEqual(rows);
  });
});

describe("moveArrayItem", () => {
  it("moves an item from one index to another", () => {
    expect(moveArrayItem([1, 2, 3, 4], 0, 2)).toEqual([2, 3, 1, 4]);
  });
  it("is a no-op when fromIndex === toIndex", () => {
    const arr = [1, 2, 3];
    expect(moveArrayItem(arr, 1, 1)).toEqual(arr);
  });
  it("moves an item to the end when toIndex === arr.length", () => {
    expect(moveArrayItem([1, 2, 3], 0, 3)).toEqual([2, 3, 1]);
  });
});

describe("labelFor / labelForRest", () => {
  it("labels a mono input row", () => {
    expect(labelFor("input", [5])).toBe("Ch 5");
  });
  it("labels a stereo bus row", () => {
    expect(labelFor("bus", [2, 3])).toBe("Bus 2+3");
  });
  it("labels a rest number by kind", () => {
    expect(labelForRest("input", 7)).toBe("Ch 7");
    expect(labelForRest("bus", 7)).toBe("Bus 7");
  });
});
