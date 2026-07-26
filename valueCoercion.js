/**
 * Pure value-coercion/normalization helpers shared across the bridge's config loading,
 * Mixing Station <-> Console 1 value translation, and WebSocket payload handling.
 *
 * Kept separate from `index.js` (which self-executes the bridge process on load) so this
 * logic can be unit-tested with the plain Node test runner without spinning up real MIDI/WS
 * connections — same rationale as `console1StatusBank.js` and `midiColorUtils.js`.
 */

/** @param {any} v @returns {v is string} */
function isNonEmptyString(v) {
  return typeof v === "string" && v.trim().length > 0;
}

/**
 * Console 1 sometimes sends momentary button events (often always `true`) instead of the final state.
 *
 * Heuristic: if incoming equals current, treat it as a toggle.
 *
 * @param {boolean} current
 * @param {boolean} incoming
 * @returns {boolean} next state
 * @example
 * // Momentary "press" while already muted => toggle to unmuted
 * resolveNextBooleanFromMomentary(true, true); // false
 */
function resolveNextBooleanFromMomentary(current, incoming) {
  return incoming === current ? !current : incoming;
}

/** dB floor Mixing Station uses to represent silence/-Infinity. */
const MIN_DB_AS_NEGATIVE_INFINITY = -90;
// MS can report floats like -89.999 due to rounding; treat near-floor as -Infinity for Console 1.
const DB_NEGATIVE_INFINITY_EPS = 1e-3;

/**
 * Treat near-floor dB values as Console 1 "-Infinity".
 * Mixing Station can send -89.999 due to rounding.
 * @param {unknown} value
 * @returns {boolean}
 * @example
 * isNegativeInfinityDb(-89.999); // true
 * isNegativeInfinityDb(-40); // false
 */
function isNegativeInfinityDb(value) {
  return (
    typeof value === "number" &&
    Number.isFinite(value) &&
    value <= MIN_DB_AS_NEGATIVE_INFINITY + DB_NEGATIVE_INFINITY_EPS
  );
}

/**
 * Normalize Console 1 level encoding for Mixing Station.
 * Console 1 encodes -Infinity as the JSON string "-Infinity".
 * @param {number|string} value
 * @returns {number|null}
 * @example
 * normalizeConsole1LevelForMs("-Infinity"); // -90
 * normalizeConsole1LevelForMs(-6); // -6
 */
function normalizeConsole1LevelForMs(value) {
  // Console 1 encodes -Infinity as a JSON string.
  if (value === "-Infinity") return MIN_DB_AS_NEGATIVE_INFINITY;
  if (typeof value === "number") {
    if (!Number.isFinite(value)) return MIN_DB_AS_NEGATIVE_INFINITY;
    return value;
  }
  if (typeof value === "string") {
    const n = Number(value);
    if (Number.isFinite(n)) return n;
  }
  return null;
}

/**
 * For stereo-linked channel names coming from Mixing Station, remove trailing side markers.
 *
 * Examples:
 * - "Keys L" -> "Keys"
 * - "Vox R" -> "Vox"
 * - "GTR P" -> "GTR"
 *
 * @param {unknown} name
 * @returns {unknown} Trimmed string if input was a string; otherwise returns input unchanged.
 * @example
 * trimStereoSuffixFromName("Keys L"); // "Keys"
 * trimStereoSuffixFromName("Kuba Vox P MB"); // "Kuba Vox MB"
 */
function trimStereoSuffixFromName(name) {
  if (typeof name !== "string" || name.length < 2) return name;

  const n = name.trimEnd();

  // Common case: names end with the side marker.
  const suffix = n.slice(-2);
  if (suffix === " L" || suffix === " R" || suffix === " P") {
    return n.slice(0, -2);
  }

  // Bus names often look like: "Dr L MixBus" / "Bass R MixBus" / "Kuba Vox P MB".
  // In these cases the side marker is *before* the trailing bus label.
  const m = n.match(/^(.*)\s([LRP])\s+(MixBus|MB)$/);
  if (m) {
    const base = (m[1] || "").trimEnd();
    const tail = m[3];
    return base ? `${base} ${tail}` : tail;
  }

  return n;
}

/**
 * Console 1 SysEx payload may encode numbers as strings.
 * Preserve the special "-Infinity" string (Console 1 OSD expects it as-is).
 * @param {any} value
 * @returns {any}
 * @example
 * coerceConsole1NumericString("-6.5"); // -6.5
 * coerceConsole1NumericString("-Infinity"); // "-Infinity"
 */
function coerceConsole1NumericString(value) {
  if (typeof value === "string" && value !== "-Infinity") {
    const n = Number(value);
    if (Number.isFinite(n)) return n;
  }
  return value;
}

/**
 * Console 1's DSP-section fields (Filter/EQ/Compressor) arrive `{value: ...}`-wrapped,
 * unlike the bare mixer primitives (`volume`, `mute`, `pan`). Unwraps either shape.
 * @param {any} field
 * @returns {any} `undefined` iff the field itself is `undefined` (missing from the
 *   SysEx payload) — the standard "field not present" signal the handlers guard on.
 * @example
 * readC1DspValue({ value: 0.5 }); // 0.5
 * readC1DspValue(0.5); // 0.5
 * readC1DspValue(undefined); // undefined
 */
function readC1DspValue(field) {
  if (field === undefined) return undefined;
  if (field !== null && typeof field === "object" && "value" in field) return field.value;
  return field;
}

/**
 * Coerce a raw WebSocket message payload (string, Buffer, Uint8Array, or ArrayBuffer) into text.
 * @param {string|Buffer|Uint8Array|ArrayBuffer} data
 * @returns {string}
 * @example
 * coerceWsPayloadToText(Buffer.from("hi")); // "hi"
 */
function coerceWsPayloadToText(data) {
  return typeof data === "string"
    ? data
    : Buffer.isBuffer(data)
      ? data.toString("utf8")
      : data instanceof Uint8Array
        ? Buffer.from(data).toString("utf8")
        : data instanceof ArrayBuffer
          ? Buffer.from(new Uint8Array(data)).toString("utf8")
          : String(data);
}

module.exports = {
  MIN_DB_AS_NEGATIVE_INFINITY,
  DB_NEGATIVE_INFINITY_EPS,
  isNonEmptyString,
  resolveNextBooleanFromMomentary,
  isNegativeInfinityDb,
  normalizeConsole1LevelForMs,
  trimStereoSuffixFromName,
  coerceConsole1NumericString,
  readC1DspValue,
  coerceWsPayloadToText,
};
