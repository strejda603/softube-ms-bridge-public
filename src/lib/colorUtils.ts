/** Softube's 24-bit color format is `0xBBGGRR` -- red in the low byte, green in the middle
 * byte, blue in the high byte (the reverse of the more common `0xRRGGBB` layout). Confirmed
 * consistent with `crates/bridge-core/src/midi_color_utils.rs`'s own doc comment. Ported from
 * `app/renderer/renderer.js:311-337`. */
export function rgbToSoftubeInt(hex: string): number | null {
  const m = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!m) return null;
  const rr = parseInt(m[1].slice(0, 2), 16);
  const gg = parseInt(m[1].slice(2, 4), 16);
  const bb = parseInt(m[1].slice(4, 6), 16);
  return (rr & 0xff) | ((gg & 0xff) << 8) | ((bb & 0xff) << 16);
}

export function softubeIntToRgb(intVal: number): string {
  if (typeof intVal !== "number" || !Number.isFinite(intVal)) return "#000000";
  const rr = intVal & 0xff;
  const gg = (intVal >> 8) & 0xff;
  const bb = (intVal >> 16) & 0xff;
  return `#${rr.toString(16).padStart(2, "0")}${gg.toString(16).padStart(2, "0")}${bb
    .toString(16)
    .padStart(2, "0")}`;
}

export function parseHexInt(str: string): number | null {
  const s = String(str || "").trim();
  if (!s) return null;
  const m = /^(0x)?([0-9a-f]+)$/i.exec(s);
  if (!m) return null;
  return parseInt(m[2], 16);
}

/** The 6-hex-digit uppercase `0x`-prefixed display format used throughout the Colors panel
 * (not a standard RGB reinterpretation -- see `rgbToSoftubeInt`'s doc comment above). */
export function formatColorIntHex(intVal: number): string {
  return `0x${intVal.toString(16).padStart(6, "0").toUpperCase()}`;
}
