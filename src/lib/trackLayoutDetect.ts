interface ConsoleInformationChannelType {
  offset?: number;
  count?: number;
  name?: string;
  shortName?: string;
  type?: number;
}

interface ConsoleInformation {
  channelTypes?: ConsoleInformationChannelType[];
}

/** Probes Mixing Station's `/console/information` over a direct WebSocket from the frontend
 * (viable since `tauri.conf.json`'s CSP is unset). Ported from
 * `app/renderer/renderer.js:1234-1279`. */
export async function fetchMixingStationInformation(
  wsUrl: string,
  timeoutMs = 2500
): Promise<unknown> {
  return await new Promise((resolve, reject) => {
    let settled = false;
    const ws = new WebSocket(wsUrl);

    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      try {
        ws.close();
      } catch {
        // ignore
      }
      reject(new Error("Timeout while waiting for /console/information"));
    }, timeoutMs);

    ws.addEventListener("open", () => {
      ws.send(JSON.stringify({ path: "/console/information", method: "GET" }));
    });

    ws.addEventListener("message", (evt: MessageEvent) => {
      if (settled) return;
      try {
        const msg = JSON.parse(typeof evt.data === "string" ? evt.data : String(evt.data));
        if (!msg || msg.path !== "/console/information") return;
        settled = true;
        clearTimeout(timer);
        try {
          ws.close();
        } catch {
          // ignore
        }
        resolve(msg.body ?? msg);
      } catch {
        // ignore non-JSON
      }
    });

    ws.addEventListener("error", () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      reject(new Error("Failed to connect to Mixing Station WebSocket"));
    });
  });
}

function clampInt(n: number, min: number, max: number): number {
  if (!Number.isFinite(n)) return min;
  return Math.max(min, Math.min(max, Math.trunc(n)));
}

/** Ported from `app/renderer/renderer.js:1281-1318`: prefer numeric `channelTypes[].type`
 * (0=input, 4=bus, stable across mixers), fall back to name/shortName substring matching,
 * then offset-0 / first-remaining heuristics. */
export function inferCountsFromInformation(info: unknown): {
  inputCount: number;
  busCount: number;
} {
  const types = Array.isArray((info as ConsoleInformation)?.channelTypes)
    ? (info as ConsoleInformation).channelTypes!
    : [];
  const normalized = types
    .filter((ct) => ct && Number.isFinite(ct.offset) && Number.isFinite(ct.count))
    .map((ct) => ({
      offset: ct.offset as number,
      count: ct.count as number,
      name: String(ct.name || "").toLowerCase(),
      shortName: String(ct.shortName || "").toLowerCase(),
      type: Number.isFinite(ct.type) ? (ct.type as number) : null,
    }))
    .sort((a, b) => a.offset - b.offset);

  const pick = (re: RegExp) => normalized.find((ct) => re.test(ct.name) || re.test(ct.shortName));

  const inputsByType = normalized.find((ct) => ct.type === 0);
  const inputsByName = pick(/\binput\b|\bin\b/);
  const inputsByOffset0 = normalized.find((ct) => ct.offset === 0);
  const inputs = inputsByType || inputsByName || inputsByOffset0 || normalized[0];

  const busesByType = normalized.find((ct) => ct.type === 4);
  const busesByName = pick(/\bbus\b/);
  const busesFallback = pick(/\bmix\b|\baux\b/);
  const buses =
    busesByType ||
    busesByName ||
    busesFallback ||
    normalized.find((ct) => ct.offset !== (inputs?.offset ?? -1) && ct.count > 0);

  const inputCount = inputs ? clampInt(inputs.count, 1, 512) : 32;
  const busCount = buses ? clampInt(buses.count, 1, 512) : 16;

  return { inputCount, busCount };
}
