const test = require("node:test");
const assert = require("node:assert/strict");
const { computeStatus } = require("../app/statusMonitor");

const EMPTY = { midiInputNames: [], midiOutputNames: [], psOutput: "" };

test("all false when nothing is present", () => {
  const status = computeStatus(EMPTY);
  assert.deepEqual(status, {
    ipad: false,
    spdSxPro: false,
    midiMaestro: false,
    bomeMtp: false,
    mixingStation: false,
    console1Osd: false,
    abletonLive: false,
  });
});

test("ipad requires BOTH a matching input and a matching output", () => {
  const inputOnly = computeStatus({
    ...EMPTY,
    midiInputNames: ["iPad"],
  });
  assert.equal(inputOnly.ipad, false);

  const outputOnly = computeStatus({
    ...EMPTY,
    midiOutputNames: ["iPad"],
  });
  assert.equal(outputOnly.ipad, false);

  const both = computeStatus({
    ...EMPTY,
    midiInputNames: ["iPad"],
    midiOutputNames: ["iPad"],
  });
  assert.equal(both.ipad, true);
});

test("spdSxPro requires a matching OUTPUT only", () => {
  const outputMatch = computeStatus({ ...EMPTY, midiOutputNames: ["SPD-SX PRO"] });
  assert.equal(outputMatch.spdSxPro, true);

  const inputOnlyDoesNotCount = computeStatus({ ...EMPTY, midiInputNames: ["SPD-SX PRO"] });
  assert.equal(inputOnlyDoesNotCount.spdSxPro, false);
});

test("midiMaestro requires a matching INPUT only, and matches the Bluetooth variant", () => {
  const wired = computeStatus({ ...EMPTY, midiInputNames: ["MIDI Maestro"] });
  assert.equal(wired.midiMaestro, true);

  const bluetooth = computeStatus({ ...EMPTY, midiInputNames: ["MIDI Maestro Bluetooth"] });
  assert.equal(bluetooth.midiMaestro, true);

  const outputOnlyDoesNotCount = computeStatus({ ...EMPTY, midiOutputNames: ["MIDI Maestro"] });
  assert.equal(outputOnlyDoesNotCount.midiMaestro, false);
});

test("substring matching: a longer port name still matches", () => {
  const status = computeStatus({
    ...EMPTY,
    midiInputNames: ["iPad (Network Session 1)"],
    midiOutputNames: ["iPad (Network Session 1)"],
  });
  assert.equal(status.ipad, true);
});

test("bomeMtp, mixingStation, console1Osd, abletonLive each match ps output independently", () => {
  const status = computeStatus({
    ...EMPTY,
    psOutput: [
      "/Applications/Bome MIDI Translator Pro.app/Contents/MacOS/BomeMTP",
      "/Applications/Mixing Station.app/Contents/MacOS/Mixing Station",
      "/Applications/Softube On-Screen Display.app/Contents/MacOS/Softube On-Screen Display",
      "/Applications/Ableton Live 12 Suite.app/Contents/MacOS/Live",
    ].join("\n"),
  });
  assert.equal(status.bomeMtp, true);
  assert.equal(status.mixingStation, true);
  assert.equal(status.console1Osd, true);
  assert.equal(status.abletonLive, true);
});

test("ps matches are independent of each other (only one process running)", () => {
  const status = computeStatus({
    ...EMPTY,
    psOutput: "/Applications/Ableton Live 12 Suite.app/Contents/MacOS/Live",
  });
  assert.equal(status.abletonLive, true);
  assert.equal(status.bomeMtp, false);
  assert.equal(status.mixingStation, false);
  assert.equal(status.console1Osd, false);
});

test("missing input arrays default to no matches rather than throwing", () => {
  const status = computeStatus({});
  assert.deepEqual(status, {
    ipad: false,
    spdSxPro: false,
    midiMaestro: false,
    bomeMtp: false,
    mixingStation: false,
    console1Osd: false,
    abletonLive: false,
  });
});
