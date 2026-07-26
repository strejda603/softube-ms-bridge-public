const test = require("node:test");
const assert = require("node:assert/strict");
const {
  MIN_DB_AS_NEGATIVE_INFINITY,
  isNonEmptyString,
  resolveNextBooleanFromMomentary,
  isNegativeInfinityDb,
  normalizeConsole1LevelForMs,
  trimStereoSuffixFromName,
  coerceConsole1NumericString,
  readC1DspValue,
  coerceWsPayloadToText,
} = require("../valueCoercion");

test("isNonEmptyString: rejects non-strings, empty strings, and whitespace-only strings", () => {
  assert.equal(isNonEmptyString("hello"), true);
  assert.equal(isNonEmptyString(""), false);
  assert.equal(isNonEmptyString("   "), false);
  assert.equal(isNonEmptyString(42), false);
  assert.equal(isNonEmptyString(undefined), false);
});

test("resolveNextBooleanFromMomentary: incoming equal to current toggles (momentary press)", () => {
  assert.equal(resolveNextBooleanFromMomentary(true, true), false);
  assert.equal(resolveNextBooleanFromMomentary(false, false), true);
});

test("resolveNextBooleanFromMomentary: incoming different from current is used as-is", () => {
  assert.equal(resolveNextBooleanFromMomentary(true, false), false);
  assert.equal(resolveNextBooleanFromMomentary(false, true), true);
});

test("isNegativeInfinityDb: at or below the floor (plus epsilon) is true", () => {
  assert.equal(isNegativeInfinityDb(MIN_DB_AS_NEGATIVE_INFINITY), true);
  assert.equal(isNegativeInfinityDb(-89.999), true);
});

test("isNegativeInfinityDb: above the floor is false", () => {
  assert.equal(isNegativeInfinityDb(-40), false);
});

test("isNegativeInfinityDb: non-numbers are false", () => {
  assert.equal(isNegativeInfinityDb("-90"), false);
  assert.equal(isNegativeInfinityDb(undefined), false);
});

test("normalizeConsole1LevelForMs: the '-Infinity' string maps to the dB floor", () => {
  assert.equal(normalizeConsole1LevelForMs("-Infinity"), MIN_DB_AS_NEGATIVE_INFINITY);
});

test("normalizeConsole1LevelForMs: finite numbers pass through", () => {
  assert.equal(normalizeConsole1LevelForMs(-6), -6);
});

test("normalizeConsole1LevelForMs: numeric strings are parsed", () => {
  assert.equal(normalizeConsole1LevelForMs("-6.5"), -6.5);
});

test("normalizeConsole1LevelForMs: non-finite number maps to the dB floor", () => {
  assert.equal(normalizeConsole1LevelForMs(-Infinity), MIN_DB_AS_NEGATIVE_INFINITY);
});

test("normalizeConsole1LevelForMs: unparseable value returns null", () => {
  assert.equal(normalizeConsole1LevelForMs("not a number"), null);
  assert.equal(normalizeConsole1LevelForMs(null), null);
});

test("trimStereoSuffixFromName: strips trailing L/R/P side markers", () => {
  assert.equal(trimStereoSuffixFromName("Keys L"), "Keys");
  assert.equal(trimStereoSuffixFromName("Vox R"), "Vox");
  assert.equal(trimStereoSuffixFromName("GTR P"), "GTR");
});

test("trimStereoSuffixFromName: strips a side marker before a trailing bus label", () => {
  assert.equal(trimStereoSuffixFromName("Dr L MixBus"), "Dr MixBus");
  assert.equal(trimStereoSuffixFromName("Kuba Vox P MB"), "Kuba Vox MB");
});

test("trimStereoSuffixFromName: names without a side marker pass through unchanged", () => {
  assert.equal(trimStereoSuffixFromName("Lead Vocal"), "Lead Vocal");
});

test("trimStereoSuffixFromName: non-string input passes through unchanged", () => {
  assert.equal(trimStereoSuffixFromName(undefined), undefined);
  assert.equal(trimStereoSuffixFromName(42), 42);
});

test("coerceConsole1NumericString: numeric strings become numbers", () => {
  assert.equal(coerceConsole1NumericString("-6.5"), -6.5);
});

test("coerceConsole1NumericString: the '-Infinity' string is preserved as-is", () => {
  assert.equal(coerceConsole1NumericString("-Infinity"), "-Infinity");
});

test("coerceConsole1NumericString: non-numeric strings and non-strings pass through", () => {
  assert.equal(coerceConsole1NumericString("abc"), "abc");
  assert.equal(coerceConsole1NumericString(5), 5);
  assert.equal(coerceConsole1NumericString(true), true);
});

test("readC1DspValue: unwraps a {value: ...}-wrapped field", () => {
  assert.strictEqual(readC1DspValue({ value: 0.5 }), 0.5);
});

test("readC1DspValue: unwraps a {value: ...}-wrapped boolean field", () => {
  assert.strictEqual(readC1DspValue({ value: true }), true);
});

test("readC1DspValue: passes through a bare (non-wrapped) value unchanged", () => {
  assert.strictEqual(readC1DspValue(0.5), 0.5);
  assert.strictEqual(readC1DspValue(true), true);
});

test("readC1DspValue: returns undefined for an undefined field", () => {
  assert.strictEqual(readC1DspValue(undefined), undefined);
});

test("readC1DspValue: returns null unchanged (not undefined) for a null field", () => {
  assert.strictEqual(readC1DspValue(null), null);
});

test("coerceWsPayloadToText: string input passes through", () => {
  assert.equal(coerceWsPayloadToText("hello"), "hello");
});

test("coerceWsPayloadToText: Buffer is decoded as utf8", () => {
  assert.equal(coerceWsPayloadToText(Buffer.from("hi")), "hi");
});

test("coerceWsPayloadToText: Uint8Array is decoded as utf8", () => {
  assert.equal(coerceWsPayloadToText(new Uint8Array(Buffer.from("hi"))), "hi");
});

test("coerceWsPayloadToText: ArrayBuffer is decoded as utf8", () => {
  const ab = new Uint8Array(Buffer.from("hi")).buffer;
  assert.equal(coerceWsPayloadToText(ab), "hi");
});
