/**
 * Pure helpers for decoding Mixing Station metering2 payloads and converting dB values into
 * Console 1's expected normalized peak meter range.
 *
 * Kept separate from `index.js` (which self-executes the bridge process on load) so this
 * logic can be unit-tested with the plain Node test runner without spinning up real MIDI/WS
 * connections — same rationale as `console1StatusBank.js` and `midiColorUtils.js`.
 */

/**
 * Mixing Station uses non-padded base64 for metering2 binary payloads.
 * @param {string} s
 * @returns {Buffer}
 * @example
 * decodeNonPaddedBase64("AAE"); // Buffer with the decoded bytes
 */
function decodeNonPaddedBase64(s) {
  if (typeof s !== "string") return Buffer.alloc(0);
  // Pad to multiple of 4 chars.
  const pad = (4 - (s.length % 4)) % 4;
  const padded = pad ? s + "=".repeat(pad) : s;
  return Buffer.from(padded, "base64");
}

/**
 * Decode metering2 binary payload.
 * Values are int16 big-endian, scaled by 100 (1.02 dB -> 102).
 *
 * We don't know per-channel stereo/extra meter counts, so we only support cases
 * where each subscribed param returns the same number of values.
 *
 * @param {string} b64
 * @param {number} paramCount
 * @returns {{valuesPerParam:number, maxDbByParam:number[]}|null}
 * @example
 * decodeMetering2Binary("AAA", 1); // null (too short to contain any int16 values)
 */
function decodeMetering2Binary(b64, paramCount) {
  if (!Number.isInteger(paramCount) || paramCount <= 0) return null;
  const buf = decodeNonPaddedBase64(b64);
  if (!buf || buf.length < 2) return null;
  const totalValues = Math.floor(buf.length / 2);
  if (totalValues <= 0) return null;
  if (totalValues % paramCount !== 0) return null;

  const valuesPerParam = totalValues / paramCount;
  /** @type {number[]} */
  const maxDbByParam = new Array(paramCount);
  for (let p = 0; p < paramCount; p++) {
    let maxDb = -Infinity;
    const start = p * valuesPerParam;
    const end = start + valuesPerParam;
    for (let i = start; i < end; i++) {
      const off = i * 2;
      if (off + 1 >= buf.length) break;
      const raw = buf.readInt16BE(off);
      const db = raw / 100;
      if (Number.isFinite(db)) maxDb = Math.max(maxDb, db);
    }
    maxDbByParam[p] = maxDb;
  }

  return { valuesPerParam, maxDbByParam };
}

/**
 * Convert a dB value from Mixing Station metering2 into Console 1's expected normalized peak meter.
 *
 * Console 1 expects a peak-style meter in the range 0..1.
 * Cubase reference script multiplies by $\sqrt{2}$ to convert RMS->peak.
 *
 * @param {number|string} db - Meter value in dB (e.g. -20)
 * @returns {number} Peak-like 0..1 meter value
 * @example
 * dbToConsole1MeterNorm(-6); // ~0.707.. * sqrt(2) => ~1 (clamped)
 */
function dbToConsole1MeterNorm(db) {
  const n = Number(db);
  if (!Number.isFinite(n)) return 0;
  // Values are in dB (per Mixing Station docs). Convert to linear, then to a peak-style
  // meter value (Cubase reference script multiplies by sqrt(2)).
  const lin = Math.pow(10, n / 20);
  if (!Number.isFinite(lin)) return 0;
  const peak = lin * Math.sqrt(2);
  return Math.max(0, Math.min(1, peak));
}

module.exports = {
  decodeNonPaddedBase64,
  decodeMetering2Binary,
  dbToConsole1MeterNorm,
};
