import { describe, expect, it } from "vitest";
import { normalizeC1SendMapping } from "./sendsMapping";

describe("normalizeC1SendMapping", () => {
  it("passes through valid entries clamped to busTotalCount", () => {
    expect(normalizeC1SendMapping([1, 2, 3, 4, 5, 6], 16)).toEqual([1, 2, 3, 4, 5, 6]);
  });

  it("clamps out-of-range entries", () => {
    expect(normalizeC1SendMapping([0, 99, 3, 4, 5, 6], 16)).toEqual([1, 16, 3, 4, 5, 6]);
  });

  it("falls back to slotIndex+1 (capped to busTotalCount) for missing entries", () => {
    expect(normalizeC1SendMapping([1, 2], 16)).toEqual([1, 2, 3, 4, 5, 6]);
  });

  it("falls back for a non-finite entry within a partial array", () => {
    expect(normalizeC1SendMapping([1, NaN, 3, 4, 5, 6], 16)).toEqual([1, 2, 3, 4, 5, 6]);
  });

  it("handles an undefined raw array entirely", () => {
    expect(normalizeC1SendMapping(undefined, 16)).toEqual([1, 2, 3, 4, 5, 6]);
  });

  it("caps fallback slots at busTotalCount when busTotalCount < 6", () => {
    expect(normalizeC1SendMapping(undefined, 3)).toEqual([1, 2, 3, 3, 3, 3]);
  });

  it("always returns exactly 6 entries even given extra input entries", () => {
    expect(normalizeC1SendMapping([1, 2, 3, 4, 5, 6, 7, 8], 16)).toHaveLength(6);
  });

  it("does not dedupe -- two slots may legally share the same bus", () => {
    expect(normalizeC1SendMapping([5, 5, 5, 5, 5, 5], 16)).toEqual([5, 5, 5, 5, 5, 5]);
  });
});
