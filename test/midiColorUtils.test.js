const test = require("node:test");
const assert = require("node:assert/strict");
const {
  MS_PALETTE_BASE_COLORS,
  MS_STYLECLASS_TO_PALETTE_INDEX,
  tintSoftubeColor,
  msColorToSoftubeColorInt,
} = require("../midiColorUtils");

test("tintSoftubeColor: amount 0 returns the original color", () => {
  assert.equal(tintSoftubeColor(0x0000ff, 0), 0x0000ff);
});

test("tintSoftubeColor: amount 1 blends fully to white", () => {
  assert.equal(tintSoftubeColor(0x0000ff, 1), 0xffffff);
});

test("msColorToSoftubeColorInt: palette index 0-7 maps directly to base colors", () => {
  assert.equal(msColorToSoftubeColorInt(1), MS_PALETTE_BASE_COLORS[1]);
  assert.equal(msColorToSoftubeColorInt(2), MS_PALETTE_BASE_COLORS[2]);
});

test("msColorToSoftubeColorInt: palette index 8-15 tints the base color (inv variant)", () => {
  const result = msColorToSoftubeColorInt(9);
  assert.equal(result, tintSoftubeColor(MS_PALETTE_BASE_COLORS[1], 0.6));
});

test("msColorToSoftubeColorInt: already-encoded RGB int passes through unchanged", () => {
  assert.equal(msColorToSoftubeColorInt(0x123456), 0x123456);
});

test("msColorToSoftubeColorInt: styleClass string maps via MS_STYLECLASS_TO_PALETTE_INDEX", () => {
  const idx = MS_STYLECLASS_TO_PALETTE_INDEX["mixer-green"];
  assert.equal(msColorToSoftubeColorInt("mixer-green"), MS_PALETTE_BASE_COLORS[idx]);
});

test("msColorToSoftubeColorInt: unknown styleClass string returns undefined", () => {
  assert.equal(msColorToSoftubeColorInt("not-a-real-class"), undefined);
});

test("msColorToSoftubeColorInt: out-of-range number returns undefined", () => {
  assert.equal(msColorToSoftubeColorInt(-1), undefined);
  assert.equal(msColorToSoftubeColorInt(0x1000000), undefined);
});
