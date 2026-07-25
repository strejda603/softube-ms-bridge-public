/**
 * Pure helpers for converting Mixing Station color values into Softube's 24-bit color
 * integer format.
 *
 * Kept separate from `index.js` (which self-executes the bridge process on load) so this
 * logic can be unit-tested with the plain Node test runner without spinning up real MIDI/WS
 * connections — same rationale as `console1StatusBank.js` and `app/statusMonitor.js`.
 */

/**
 * Mixing Station color mapping.
 * MS may provide a palette index (0..15), a `styleClass` string, or an already-encoded RGB int.
 * We normalize these into Softube's 24-bit integer format: r in LSB, g in next byte, b in MSB.
 * @type {number[]}
 */
const MS_PALETTE_BASE_COLORS = [
  0x000000, // Black
  0x0000ff, // Red
  0x00ff00, // Green
  0x00ffff, // Yellow
  0xff0000, // Blue
  0xff00ff, // Magenta
  0xffff00, // Cyan
  0xffffff, // White
];

/** @type {Record<string, number>} */
const MS_STYLECLASS_TO_PALETTE_INDEX = {
  "mixer-black": 0,
  "mixer-red": 1,
  "mixer-green": 2,
  "mixer-yellow": 3,
  "mixer-blue": 4,
  "mixer-magenta": 5,
  "mixer-cyan": 6,
  "mixer-white": 7,
  "mixer-black-inv": 8,
  "mixer-red-inv": 9,
  "mixer-green-inv": 10,
  "mixer-yellow-inv": 11,
  "mixer-blue-inv": 12,
  "mixer-magenta-inv": 13,
  "mixer-cyan-inv": 14,
  "mixer-white-inv": 15,
};

/**
 * Lighten a Softube 24-bit color by blending towards white.
 *
 * Softube color format is 0xBBGGRR (red in LSB).
 *
 * @param {number} colorInt - 24-bit color int
 * @param {number} amount01 - 0..1 blend amount
 * @returns {number} 24-bit color int
 * @example
 * tintSoftubeColor(0x0000ff, 0.6); // lighter red
 */
function tintSoftubeColor(colorInt, amount01) {
  const r = colorInt & 0xff;
  const g = (colorInt >> 8) & 0xff;
  const b = (colorInt >> 16) & 0xff;
  const lr = Math.round(r + (255 - r) * amount01);
  const lg = Math.round(g + (255 - g) * amount01);
  const lb = Math.round(b + (255 - b) * amount01);
  return (lr & 0xff) | ((lg & 0xff) << 8) | ((lb & 0xff) << 16);
}

/**
 * Convert a Mixing Station color value (palette index, style class, or RGB int) into Softube's 24-bit int.
 *
 * @param {number|string} value
 * @returns {number|undefined}
 * @example
 * msColorToSoftubeColorInt(1); // 0x0000ff (palette index 1 = Red)
 * msColorToSoftubeColorInt("mixer-green"); // 0x00ff00
 */
function msColorToSoftubeColorInt(value) {
  let colorInt = undefined;
  if (typeof value === "number") {
    if (value >= 0 && value <= 15) {
      const idx = value;
      const base = MS_PALETTE_BASE_COLORS[idx % 8];
      // For "inv" colors use a lighter tint for readability (instead of true inversion).
      colorInt = idx < 8 ? base : tintSoftubeColor(base, 0.6);
    } else if (value >= 0 && value <= 0xffffff) {
      colorInt = value;
    }
  } else if (typeof value === "string") {
    const idx = MS_STYLECLASS_TO_PALETTE_INDEX[value];
    if (idx !== undefined) {
      const base = MS_PALETTE_BASE_COLORS[idx % 8];
      colorInt = idx < 8 ? base : tintSoftubeColor(base, 0.6);
    }
  }
  return typeof colorInt === "number" ? colorInt : undefined;
}

module.exports = {
  MS_PALETTE_BASE_COLORS,
  MS_STYLECLASS_TO_PALETTE_INDEX,
  tintSoftubeColor,
  msColorToSoftubeColorInt,
};
