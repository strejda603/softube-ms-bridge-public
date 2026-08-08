import { describe, expect, it } from "vitest";
import { parseModeFromLogLine } from "./modeParse";

describe("parseModeFromLogLine", () => {
  it("returns null for a line with no [Mode] substring", () => {
    expect(parseModeFromLogLine("some unrelated log line")).toBe(null);
  });

  it("returns 'Standard' for a STANDARD active line", () => {
    expect(parseModeFromLogLine("[Mode] STANDARD active")).toBe("Standard");
  });

  it("returns 'Sends (Bus N)' for a valid SENDS active line with a bus number", () => {
    expect(parseModeFromLogLine("[Mode] SENDS active (source=trackSelect bus=3)")).toBe(
      "Sends (Bus 3)"
    );
  });

  it("returns null for a line containing [Mode] that matches neither recognized pattern", () => {
    expect(parseModeFromLogLine("[Mode] unrecognized state")).toBe(null);
  });
});
