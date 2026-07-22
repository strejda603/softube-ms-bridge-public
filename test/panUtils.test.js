const test = require("node:test");
const assert = require("node:assert/strict");
const { STEREO_HYBRID_NARROW_ZONE, clamp01, hybridStereoPanToDualMonoPans } = require("../panUtils");

test("clamp01: clamps below 0 and above 1", () => {
  assert.equal(clamp01(-0.5), 0);
  assert.equal(clamp01(1.5), 1);
});

test("clamp01: any non-finite input (including +/-Infinity) clamps to 0", () => {
  assert.equal(clamp01(NaN), 0);
  assert.equal(clamp01(Infinity), 0);
  assert.equal(clamp01(-Infinity), 0);
});

test("clamp01: in-range input passes through", () => {
  assert.equal(clamp01(0.42), 0.42);
});

test("hybridStereoPanToDualMonoPans: center knob is full stereo width, hard L/R", () => {
  const result = hybridStereoPanToDualMonoPans(0.5);
  assert.deepEqual(result, { left: 0, right: 1, width: 1, mid: 0.5 });
});

test("hybridStereoPanToDualMonoPans: at the narrow-zone edge, width reaches 0 (mono)", () => {
  const result = hybridStereoPanToDualMonoPans(0.5 + STEREO_HYBRID_NARROW_ZONE / 2);
  assert.ok(result.width < 1e-9);
  assert.equal(result.left, result.right);
});

test("hybridStereoPanToDualMonoPans: past the narrow zone, pans the mono image right", () => {
  const result = hybridStereoPanToDualMonoPans(1);
  assert.equal(result.width, 0);
  assert.equal(result.left, result.right);
  assert.equal(result.mid, 1);
});

test("hybridStereoPanToDualMonoPans: past the narrow zone, pans the mono image left", () => {
  const result = hybridStereoPanToDualMonoPans(0);
  assert.equal(result.width, 0);
  assert.equal(result.left, result.right);
  assert.equal(result.mid, 0);
});

test("hybridStereoPanToDualMonoPans: out-of-range input is clamped to 0..1 first", () => {
  const atZero = hybridStereoPanToDualMonoPans(-5);
  const atOne = hybridStereoPanToDualMonoPans(5);
  assert.deepEqual(atZero, hybridStereoPanToDualMonoPans(0));
  assert.deepEqual(atOne, hybridStereoPanToDualMonoPans(1));
});
