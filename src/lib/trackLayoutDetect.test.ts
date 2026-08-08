import { describe, expect, it } from "vitest";
import { inferCountsFromInformation } from "./trackLayoutDetect";

describe("inferCountsFromInformation", () => {
  it("prefers numeric channelTypes[].type (0=input, 4=bus) when present", () => {
    const info = {
      channelTypes: [
        { offset: 0, count: 32, name: "Input", type: 0 },
        { offset: 32, count: 16, name: "Bus", type: 4 },
      ],
    };
    expect(inferCountsFromInformation(info)).toEqual({ inputCount: 32, busCount: 16 });
  });

  it("falls back to name/shortName substring matching when type is absent", () => {
    const info = {
      channelTypes: [
        { offset: 0, count: 24, name: "Line Input", shortName: "in" },
        { offset: 24, count: 8, name: "Mix Bus", shortName: "bus" },
      ],
    };
    expect(inferCountsFromInformation(info)).toEqual({ inputCount: 24, busCount: 8 });
  });

  it("falls back to offset-0 for inputs when no type/name match", () => {
    const info = {
      channelTypes: [
        { offset: 0, count: 16, name: "Foo" },
        { offset: 16, count: 4, name: "Bar" },
      ],
    };
    expect(inferCountsFromInformation(info).inputCount).toBe(16);
  });

  it("defaults to 32/16 when channelTypes is missing or empty", () => {
    expect(inferCountsFromInformation({})).toEqual({ inputCount: 32, busCount: 16 });
    expect(inferCountsFromInformation(null)).toEqual({ inputCount: 32, busCount: 16 });
  });

  it("clamps detected counts to the 1..512 range", () => {
    const info = { channelTypes: [{ offset: 0, count: 9999, name: "Input", type: 0 }] };
    expect(inferCountsFromInformation(info).inputCount).toBe(512);
  });

  it("ignores entries with non-finite offset or count", () => {
    const info = {
      channelTypes: [
        { offset: 0, count: 32, name: "Input", type: 0 },
        { offset: NaN, count: 99, name: "Junk", type: 4 },
      ],
    };
    expect(inferCountsFromInformation(info)).toEqual({ inputCount: 32, busCount: 16 });
  });
});
