export function isProbablyJsonLine(line: string): boolean {
  const s = String(line ?? "").trim();
  if (!s) return false;
  const first = s[0];
  if (first !== "{" && first !== "[") return false;
  try {
    JSON.parse(s);
    return true;
  } catch {
    return false;
  }
}

/** Exact port of `app/renderer/renderer.js`'s `classifyLogLine` -- same regexes, same
 * first-match-wins priority order. */
export function classifyLogLine(line: string): string {
  const s = String(line ?? "");

  if (/\b(error|failed|exception|cannot find module)\b/i.test(s)) return "log-error";
  if (/\bwarn(ing)?\b/i.test(s)) return "log-warn";
  if (/\b(reconnecting|reconnect)\b/i.test(s)) return "log-warn";
  if (/\bFailed to load config\b/i.test(s)) return "log-warn";
  if (/\bWebSocket closed\b/i.test(s)) return "log-warn";

  if (/^\[GUI\]/i.test(s)) return "log-gui";
  if (s.includes("[Mode]")) return "log-mode";

  if (/\bConnected to Mixing Station WebSocket\b/i.test(s)) return "log-warn";
  if (/\bConsole 1 handshake sent\b/i.test(s)) return "log-warn";
  if (/\bListening to MIDI port\b/i.test(s)) return "log-warn";
  if (/\bOpened MIDI output port\b/i.test(s)) return "log-warn";
  if (/\bReceived RESET command\b/i.test(s)) return "log-warn";
  if (/\bSoftube-MS-Bridge running\b/i.test(s)) return "log-warn";
  if (/\bShutting down\b/i.test(s)) return "log-warn";

  if (/\b(SysEx JSON|SysEx data|Received MIDI message)\b/i.test(s)) return "log-json";
  if (/\bFinalizing initialization\b/i.test(s)) return "log-warn";

  if (isProbablyJsonLine(s)) return "log-json";

  return "";
}
