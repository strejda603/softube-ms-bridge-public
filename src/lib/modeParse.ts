/** Exact port of `app/renderer/renderer.js`'s `updateModeFromLogLine` parsing logic (the
 * DOM-update half lives in Topbar.svelte instead). Returns the new mode text to display, or
 * `null` if the line isn't a recognized `[Mode]` line. */
export function parseModeFromLogLine(line: string): string | null {
  const s = String(line ?? "");
  if (!s.includes("[Mode]")) return null;
  if (s.includes("[Mode] STANDARD active")) return "Standard";
  const m = /\[Mode\]\s+SENDS\s+active\s+\(.*\bbus=(\d+)\b.*\)/i.exec(s);
  if (m) return `Sends (Bus ${m[1]})`;
  return null;
}
