import { describe, expect, it } from "vitest";
import en from "../locales/en.json";
import cs from "../locales/cs.json";
import { LOCALE_STORAGE_KEY, isKnownLocale, resolveLocale } from "./localeResolve";

describe("resolveLocale", () => {
  it("returns the code unchanged when it's in the known list", () => {
    expect(resolveLocale("cs", ["en", "cs"])).toBe("cs");
  });

  it("falls back to en when the code isn't in the known list", () => {
    expect(resolveLocale("fr", ["en", "cs"])).toBe("en");
  });

  it("falls back to en when code is null", () => {
    expect(resolveLocale(null, ["en", "cs"])).toBe("en");
  });

  it("falls back to en when code is undefined", () => {
    expect(resolveLocale(undefined, ["en", "cs"])).toBe("en");
  });

  it("falls back to en when the known list is empty", () => {
    expect(resolveLocale("cs", [])).toBe("en");
  });

  it("normalizes an uppercase/mixed-case known code", () => {
    expect(resolveLocale("CS", ["en", "cs"])).toBe("cs");
  });

  it("falls back to the base language for a region-tagged code", () => {
    expect(resolveLocale("cs-CZ", ["en", "cs"])).toBe("cs");
  });

  it("falls back to en when a region-tagged code's base language isn't known", () => {
    expect(resolveLocale("fr-FR", ["en", "cs"])).toBe("en");
  });
});

describe("isKnownLocale", () => {
  it("returns true for a known code", () => {
    expect(isKnownLocale("cs", ["en", "cs"])).toBe(true);
  });

  it("returns true for a known code in a different case", () => {
    expect(isKnownLocale("CS", ["en", "cs"])).toBe(true);
  });

  it("returns true for a region-tagged known base language", () => {
    expect(isKnownLocale("cs-CZ", ["en", "cs"])).toBe(true);
  });

  it("returns false for an unknown code", () => {
    expect(isKnownLocale("de", ["en", "cs"])).toBe(false);
  });

  it("returns false for a region-tagged unknown base language", () => {
    expect(isKnownLocale("de-DE", ["en", "cs"])).toBe(false);
  });

  it("returns false for null", () => {
    expect(isKnownLocale(null, ["en", "cs"])).toBe(false);
  });

  it("returns false for undefined", () => {
    expect(isKnownLocale(undefined, ["en", "cs"])).toBe(false);
  });
});

describe("LOCALE_STORAGE_KEY", () => {
  it("is a stable, namespaced key", () => {
    expect(LOCALE_STORAGE_KEY).toBe("softubeMsBridge.locale");
  });
});

describe("locale file key parity", () => {
  it("cs.json has exactly the same keys as en.json", () => {
    expect(Object.keys(cs).sort()).toEqual(Object.keys(en).sort());
  });
});
