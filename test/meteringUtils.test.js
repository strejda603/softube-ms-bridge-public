const test = require("node:test");
const assert = require("node:assert/strict");
const {
  decodeNonPaddedBase64,
  decodeMetering2Binary,
  dbToConsole1MeterNorm,
} = require("../meteringUtils");

test("decodeNonPaddedBase64: decodes a non-padded base64 string", () => {
  const buf = Buffer.from([0x00, 0x64]); // int16BE 100
  const b64 = buf.toString("base64").replace(/=+$/, "");
  assert.deepEqual(decodeNonPaddedBase64(b64), buf);
});

test("decodeNonPaddedBase64: non-string input returns an empty buffer", () => {
  assert.deepEqual(decodeNonPaddedBase64(undefined), Buffer.alloc(0));
});

test("decodeMetering2Binary: single param, single value (1.02 dB encoded as 102)", () => {
  const buf = Buffer.alloc(2);
  buf.writeInt16BE(102, 0);
  const b64 = buf.toString("base64").replace(/=+$/, "");
  const result = decodeMetering2Binary(b64, 1);
  assert.equal(result.valuesPerParam, 1);
  assert.equal(result.maxDbByParam[0], 1.02);
});

test("decodeMetering2Binary: multiple params take the max value per param", () => {
  const buf = Buffer.alloc(8);
  buf.writeInt16BE(-2000, 0); // param0 value A: -20.00
  buf.writeInt16BE(-1000, 2); // param0 value B: -10.00 (max)
  buf.writeInt16BE(500, 4); // param1 value A: 5.00 (max)
  buf.writeInt16BE(200, 6); // param1 value B: 2.00
  const b64 = buf.toString("base64").replace(/=+$/, "");
  const result = decodeMetering2Binary(b64, 2);
  assert.equal(result.valuesPerParam, 2);
  assert.deepEqual(result.maxDbByParam, [-10, 5]);
});

test("decodeMetering2Binary: invalid paramCount returns null", () => {
  assert.equal(decodeMetering2Binary("AAA", 0), null);
  assert.equal(decodeMetering2Binary("AAA", -1), null);
  assert.equal(decodeMetering2Binary("AAA", 1.5), null);
});

test("decodeMetering2Binary: buffer too short to hold any values returns null", () => {
  assert.equal(decodeMetering2Binary("", 1), null);
});

test("decodeMetering2Binary: value count not divisible by paramCount returns null", () => {
  const buf = Buffer.alloc(6); // 3 int16 values, paramCount=2 doesn't divide evenly
  const b64 = buf.toString("base64").replace(/=+$/, "");
  assert.equal(decodeMetering2Binary(b64, 2), null);
});

test("dbToConsole1MeterNorm: non-finite input returns 0", () => {
  assert.equal(dbToConsole1MeterNorm("not a number"), 0);
  assert.equal(dbToConsole1MeterNorm(NaN), 0);
});

test("dbToConsole1MeterNorm: very loud values clamp to 1", () => {
  assert.equal(dbToConsole1MeterNorm(20), 1);
});

test("dbToConsole1MeterNorm: silence (very negative dB) is near 0", () => {
  assert.ok(dbToConsole1MeterNorm(-90) < 0.001);
});

test("dbToConsole1MeterNorm: 0 dB converts to sqrt(2), clamped to 1", () => {
  assert.equal(dbToConsole1MeterNorm(0), 1);
});
