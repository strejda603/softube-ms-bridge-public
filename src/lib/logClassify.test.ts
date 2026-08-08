import { describe, expect, it } from "vitest";
import { classifyLogLine, isProbablyJsonLine } from "./logClassify";

describe("isProbablyJsonLine", () => {
  it("returns false for an empty string", () => {
    expect(isProbablyJsonLine("")).toBe(false);
  });

  it("returns false for non-JSON text that starts with {", () => {
    expect(isProbablyJsonLine("{not actually json")).toBe(false);
  });

  it("returns true for a valid JSON array", () => {
    expect(isProbablyJsonLine("[1,2,3]")).toBe(true);
  });

  it("returns true for a valid JSON object", () => {
    expect(isProbablyJsonLine('{"a":1}')).toBe(true);
  });
});

describe("classifyLogLine", () => {
  it("classifies error/failed/exception/cannot find module lines as log-error", () => {
    expect(classifyLogLine("Something error occurred")).toBe("log-error");
    expect(classifyLogLine("Cannot find module 'foo'")).toBe("log-error");
  });

  it("classifies warn(ing) lines as log-warn", () => {
    expect(classifyLogLine("This is a warning message")).toBe("log-warn");
  });

  it("classifies reconnecting/reconnect lines as log-warn", () => {
    expect(classifyLogLine("Reconnecting to server")).toBe("log-warn");
  });

  // "Failed to load config" contains the word "Failed", which the earlier
  // error/failed/exception regex already matches (case-insensitively) -- so this
  // branch is shadowed and can never actually be reached, a faithful port of the
  // original's dead code.
  it("actually classifies 'Failed to load config' as log-error (shadowed by the failed/error rule)", () => {
    expect(classifyLogLine("Failed to load config")).toBe("log-error");
  });

  it("classifies WebSocket closed lines as log-warn", () => {
    expect(classifyLogLine("WebSocket closed unexpectedly")).toBe("log-warn");
  });

  it("classifies [GUI]-prefixed lines as log-gui", () => {
    expect(classifyLogLine("[GUI] user clicked start")).toBe("log-gui");
  });

  it("classifies lines containing [Mode] as log-mode", () => {
    expect(classifyLogLine("[Mode] STANDARD active")).toBe("log-mode");
  });

  it("classifies fixed lifecycle-status strings as log-warn", () => {
    expect(classifyLogLine("Connected to Mixing Station WebSocket")).toBe("log-warn");
    expect(classifyLogLine("Console 1 handshake sent")).toBe("log-warn");
    expect(classifyLogLine("Listening to MIDI port X")).toBe("log-warn");
    expect(classifyLogLine("Opened MIDI output port Y")).toBe("log-warn");
    expect(classifyLogLine("Received RESET command")).toBe("log-warn");
    expect(classifyLogLine("Softube-MS-Bridge running")).toBe("log-warn");
    expect(classifyLogLine("Shutting down")).toBe("log-warn");
  });

  it("classifies SysEx/MIDI-message keyword lines as log-json", () => {
    expect(classifyLogLine("SysEx JSON received")).toBe("log-json");
    expect(classifyLogLine("SysEx data dump")).toBe("log-json");
    expect(classifyLogLine("Received MIDI message")).toBe("log-json");
  });

  it("classifies 'Finalizing initialization' as log-warn", () => {
    expect(classifyLogLine("Finalizing initialization")).toBe("log-warn");
  });

  it("classifies raw JSON lines that don't match any keyword rule as log-json (fallback)", () => {
    expect(classifyLogLine('{"a":1}')).toBe("log-json");
  });

  it("returns empty string for unrecognized lines", () => {
    expect(classifyLogLine("plain text line")).toBe("");
  });
});
