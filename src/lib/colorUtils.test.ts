import { describe, expect, it } from "vitest";
import { formatColorIntHex, parseHexInt, rgbToSoftubeInt, softubeIntToRgb } from "./colorUtils";

describe("rgbToSoftubeInt", () => {
  it("packs R in the low byte, G in the middle byte, B in the high byte", () => {
    expect(rgbToSoftubeInt("#0000ff")).toBe(0xff0000);
    expect(rgbToSoftubeInt("#ff0000")).toBe(0x0000ff);
  });

  it("returns null for a non-hex-color string", () => {
    expect(rgbToSoftubeInt("not-a-color")).toBeNull();
    expect(rgbToSoftubeInt("#fff")).toBeNull();
  });
});

describe("softubeIntToRgb", () => {
  it("is the inverse of rgbToSoftubeInt", () => {
    expect(softubeIntToRgb(0xff0000)).toBe("#0000ff");
    expect(softubeIntToRgb(0x0000ff)).toBe("#ff0000");
  });

  it("returns black for a non-finite value", () => {
    expect(softubeIntToRgb(NaN)).toBe("#000000");
  });
});

describe("parseHexInt", () => {
  it("parses a 0x-prefixed hex string", () => {
    expect(parseHexInt("0x00A5FF")).toBe(0x00a5ff);
  });

  it("parses a bare hex string without a 0x prefix", () => {
    expect(parseHexInt("800080")).toBe(0x800080);
  });

  it("returns null for empty or non-hex input", () => {
    expect(parseHexInt("")).toBeNull();
    expect(parseHexInt("not hex")).toBeNull();
  });
});

describe("formatColorIntHex", () => {
  it("formats as an uppercase 6-digit 0x-prefixed string", () => {
    expect(formatColorIntHex(0x00a5ff)).toBe("0x00A5FF");
    expect(formatColorIntHex(0x80)).toBe("0x000080");
  });
});
