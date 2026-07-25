const test = require("node:test");
const assert = require("node:assert/strict");
const { computeStatus } = require("../app/statusMonitor");

test("all false when nothing is present", () => {
  const status = computeStatus({ psOutput: "" });
  assert.deepEqual(status, { mixingStation: false, console1Osd: false });
});

test("mixingStation and console1Osd each match ps output independently", () => {
  const status = computeStatus({
    psOutput: [
      "/Applications/Mixing Station.app/Contents/MacOS/Mixing Station",
      "/Applications/Softube On-Screen Display.app/Contents/MacOS/Softube On-Screen Display",
    ].join("\n"),
  });
  assert.equal(status.mixingStation, true);
  assert.equal(status.console1Osd, true);
});

test("ps matches are independent of each other (only one process running)", () => {
  const status = computeStatus({
    psOutput: "/Applications/Mixing Station.app/Contents/MacOS/Mixing Station",
  });
  assert.equal(status.mixingStation, true);
  assert.equal(status.console1Osd, false);
});

test("missing psOutput defaults to no matches rather than throwing", () => {
  const status = computeStatus({});
  assert.deepEqual(status, { mixingStation: false, console1Osd: false });
});

test("no args at all defaults to no matches rather than throwing", () => {
  const status = computeStatus();
  assert.deepEqual(status, { mixingStation: false, console1Osd: false });
});
