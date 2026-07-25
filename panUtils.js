/**
 * Pure helpers for Console 1's stereo-linked hybrid pan control.
 *
 * Kept separate from `index.js` (which self-executes the bridge process on load) so this
 * logic can be unit-tested with the plain Node test runner without spinning up real MIDI/WS
 * connections — same rationale as `console1StatusBank.js` and `midiColorUtils.js`.
 */

/** Threshold (0..1) where a stereo-linked pair's width reaches 0 and mono panning begins. */
const STEREO_HYBRID_NARROW_ZONE = 0.25;

/**
 * Clamp a number to 0..1. Non-finite input clamps to 0.
 * @param {number} n
 * @returns {number}
 * @example
 * clamp01(1.5); // 1
 * clamp01(NaN); // 0
 */
function clamp01(n) {
  if (!Number.isFinite(n)) return 0;
  return Math.max(0, Math.min(1, n));
}

/**
 * Convert a single Console 1 pan knob value (0..1) into a stereo dual-mono pan pair.
 *
 * Design goal:
 * - Center: full stereo width (hard L/R)
 * - Small turns: narrow the image towards mono (mid stays centered)
 * - Past the narrow zone: pan the now-mono image left/right (both channels move together)
 *
 * @param {number} pan01 - Console 1 pan knob value in 0..1
 * @param {number} [narrowZone=STEREO_HYBRID_NARROW_ZONE] - 0..1 threshold where width reaches 0
 * @returns {{left:number, right:number, width:number, mid:number}}
 * @example
 * hybridStereoPanToDualMonoPans(0.5); // {left:0,right:1,width:1,mid:0.5}
 */
function hybridStereoPanToDualMonoPans(pan01, narrowZone = STEREO_HYBRID_NARROW_ZONE) {
  const p = clamp01(Number(pan01));
  const x = (p - 0.5) * 2; // -1..1
  const ax = Math.abs(x);
  const t = Math.max(0.001, Math.min(0.99, Number(narrowZone) || STEREO_HYBRID_NARROW_ZONE));

  // Zone A: width reduction only, mid stays centered.
  if (ax <= t) {
    const width = 1 - ax / t; // 1..0
    const mid = 0.5;
    const half = 0.5 * width;
    return {
      left: clamp01(mid - half),
      right: clamp01(mid + half),
      width,
      mid,
    };
  }

  // Zone B: mono (width=0), then pan the mono image.
  const monoBalance01 = (ax - t) / (1 - t); // 0..1
  const balance = Math.sign(x) * monoBalance01; // -1..1
  const mid = clamp01(0.5 + 0.5 * balance);
  return { left: mid, right: mid, width: 0, mid };
}

module.exports = {
  STEREO_HYBRID_NARROW_ZONE,
  clamp01,
  hybridStereoPanToDualMonoPans,
};
