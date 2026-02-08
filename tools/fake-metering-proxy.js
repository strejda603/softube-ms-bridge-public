#!/usr/bin/env node
/*
  Fake metering helper.

  Mixing Station WS modes:
  - Standalone fake server (no upstream):
      node tools/fake-metering-proxy.js --listen ws://127.0.0.1:8082
    Point the bridge to ws://127.0.0.1:8082.

  - Proxy mode (forwards to a real Mixing Station, but injects fake metering2):
      node tools/fake-metering-proxy.js --listen ws://127.0.0.1:8081 --upstream ws://127.0.0.1:8080
    Point the bridge to ws://127.0.0.1:8081.

  X32 OSC (UDP) mode (acts like an X32 emulator for clients such as Mixing Station):
      node tools/fake-metering-proxy.js --x32 --listen 10023

  Meter generation:
    --interval 200 --mode sine --minDb -50 --maxDb -3
*/

const WebSocket = require("ws");
const dgram = require("dgram");

function parseArgs(argv) {
  /** @type {Record<string, string|boolean>} */
  const out = {};
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) continue;
    const key = a.slice(2);
    const next = argv[i + 1];
    if (next && !next.startsWith("--")) {
      out[key] = next;
      i++;
    } else {
      out[key] = true;
    }
  }
  return out;
}

function parseWsListen(s, defaultPort = 8080, defaultHost = "0.0.0.0") {
  // Accept:
  // - "8080"
  // - "host:8080"
  // - "ws://host:8080"
  // Note: defaultHost is "0.0.0.0" so phones/tablets can connect.
  if (typeof s !== "string" || s.length === 0)
    return { host: defaultHost, port: Number(defaultPort) };
  if (/^\d+$/.test(s)) return { host: defaultHost, port: Number(s) };

  const hp = s.match(/^([^:\/]+):(\d+)$/);
  if (hp) return { host: hp[1] || defaultHost, port: Number(hp[2]) };
  try {
    const u = new URL(s);
    return {
      host: u.hostname || defaultHost,
      port: Number(u.port || String(defaultPort)),
    };
  } catch {
    return { host: defaultHost, port: Number(defaultPort) };
  }
}

function toNumber(v, def) {
  const n = Number(v);
  return Number.isFinite(n) ? n : def;
}

function clamp(x, a, b) {
  return Math.max(a, Math.min(b, x));
}

function nowMs() {
  return Date.now();
}

function makeDbGenerator({ mode, minDb, maxDb, hz, seed }) {
  const lo = Math.min(minDb, maxDb);
  const hi = Math.max(minDb, maxDb);
  const range = hi - lo;

  // Tiny deterministic RNG (LCG) for "random" when seed is provided.
  let rngState = Number.isFinite(seed) ? seed >>> 0 : null;
  const rand = () => {
    if (rngState === null) return Math.random();
    rngState = (1664525 * rngState + 1013904223) >>> 0;
    return rngState / 0x100000000;
  };

  const m = String(mode || "sine").toLowerCase();
  const freq = Number.isFinite(hz) && hz > 0 ? hz : 0.25;

  return (tMs, channelIndex) => {
    const phase = (channelIndex || 0) * 0.37;

    if (m === "random") {
      return lo + rand() * range;
    }

    if (m === "saw") {
      const p = (tMs / 1000) * freq + phase;
      const frac = p - Math.floor(p);
      return lo + frac * range;
    }

    if (m === "triangle") {
      const p = (tMs / 1000) * freq + phase;
      const frac = p - Math.floor(p);
      const tri = frac < 0.5 ? frac * 2 : 2 - frac * 2;
      return lo + tri * range;
    }

    // default: sine
    const s = Math.sin((tMs / 1000) * freq * 2 * Math.PI + phase);
    const norm = (s + 1) / 2;
    return lo + norm * range;
  };
}

function safeJsonParse(s) {
  try {
    return JSON.parse(s);
  } catch {
    return null;
  }
}

function safeSend(ws, obj) {
  if (!ws || ws.readyState !== WebSocket.OPEN) return;
  ws.send(JSON.stringify(obj));
}

function safeUdpSend(sock, buf, port, address) {
  if (!sock || !Buffer.isBuffer(buf)) return;
  sock.send(buf, port, address, (err) => {
    if (err) console.warn("[fake-x32] UDP send error:", err.message);
  });
}

function pad4(n) {
  return (n + 3) & ~3;
}

function oscReadString(buf, offset) {
  let i = offset;
  while (i < buf.length && buf[i] !== 0) i++;
  const s = buf.slice(offset, i).toString("utf8");
  const next = pad4(i + 1);
  return { value: s, nextOffset: next };
}

function oscReadBlob(buf, offset) {
  if (offset + 4 > buf.length) return { value: Buffer.from([]), nextOffset: offset + 4 };
  const len = buf.readInt32BE(offset);
  const start = offset + 4;
  const end = start + Math.max(0, len);
  const data = buf.slice(start, Math.min(end, buf.length));
  return { value: data, nextOffset: pad4(end) };
}

function decodeOscMessage(buf) {
  if (!Buffer.isBuffer(buf) || buf.length < 8) return null;
  let off = 0;
  const addr = oscReadString(buf, off);
  off = addr.nextOffset;
  const tt = oscReadString(buf, off);
  off = tt.nextOffset;
  const typeTags = tt.value;
  if (!typeTags || typeTags[0] !== ",") return { address: addr.value, types: "", args: [] };

  const types = typeTags.slice(1);
  const args = [];
  for (let i = 0; i < types.length; i++) {
    const t = types[i];
    if (t === "s") {
      const r = oscReadString(buf, off);
      off = r.nextOffset;
      args.push(r.value);
    } else if (t === "i") {
      if (off + 4 > buf.length) {
        args.push(0);
        off += 4;
      } else {
        args.push(buf.readInt32BE(off));
        off += 4;
      }
    } else if (t === "f") {
      if (off + 4 > buf.length) {
        args.push(0);
        off += 4;
      } else {
        args.push(buf.readFloatBE(off));
        off += 4;
      }
    } else if (t === "b") {
      const r = oscReadBlob(buf, off);
      off = r.nextOffset;
      args.push(r.value);
    } else {
      // Unsupported type tag.
      break;
    }
  }
  return { address: addr.value, types, args };
}

function oscWriteString(s) {
  const str = String(s);
  const raw = Buffer.from(str + "\0", "utf8");
  const padded = Buffer.alloc(pad4(raw.length));
  raw.copy(padded);
  return padded;
}

function encodeOscMessage(address, types, args) {
  const parts = [];
  parts.push(oscWriteString(address));
  parts.push(oscWriteString("," + (types || "")));

  const t = types || "";
  for (let i = 0; i < t.length; i++) {
    const tag = t[i];
    const v = args?.[i];
    if (tag === "s") {
      parts.push(oscWriteString(v ?? ""));
    } else if (tag === "i") {
      const b = Buffer.alloc(4);
      b.writeInt32BE(Number(v) || 0, 0);
      parts.push(b);
    } else if (tag === "f") {
      const b = Buffer.alloc(4);
      b.writeFloatBE(Number(v) || 0, 0);
      parts.push(b);
    } else if (tag === "b") {
      const data = Buffer.isBuffer(v) ? v : Buffer.from([]);
      const len = Buffer.alloc(4);
      len.writeInt32BE(data.length, 0);
      const padded = Buffer.alloc(pad4(data.length));
      data.copy(padded);
      parts.push(len);
      parts.push(padded);
    }
  }

  return Buffer.concat(parts);
}

function dbToUnit(db) {
  const n = Number(db);
  if (!Number.isFinite(n)) return 0;
  const lin = Math.pow(10, n / 20);
  if (!Number.isFinite(lin)) return 0;
  return clamp(lin, 0, 1);
}

function makeX32MeterBlob(meterId, floatCount, meterGen, tMs, blobEndian) {
  const little = String(blobEndian || "le").toLowerCase() !== "be";
  const blob = Buffer.alloc((floatCount + 1) * 4);
  if (little) blob.writeInt32LE(floatCount, 0);
  else blob.writeInt32BE(floatCount, 0);

  for (let i = 0; i < floatCount; i++) {
    const db = meterGen(tMs, meterId * 1000 + i);
    const val = dbToUnit(db);
    const off = 4 + i * 4;
    if (little) blob.writeFloatLE(val, off);
    else blob.writeFloatBE(val, off);
  }
  return blob;
}

function makeX32OscServer({ host, port, meterGen, blobEndian, verbose }) {
  // Float counts based on the X32 emulator source (Xprepmeter l values).
  const meterFloatCounts = new Map([
    [0, 70],
    [1, 96],
    [2, 49],
    [3, 22],
    [4, 82],
    [5, 27],
    [6, 4],
    [7, 16],
    [8, 6],
    [9, 32],
    [10, 32],
    [11, 5],
    [12, 4],
    [13, 48],
    [14, 80],
    [15, 50],
    [16, 48],
  ]);

  const sock = dgram.createSocket("udp4");
  console.log(`[fake-x32] Listening on udp://${host}:${port}`);

  /** @type {Map<string, Map<number, {intervalMs:number, lastSentMs:number}>>} */
  const clientMeters = new Map();

  const tick = setInterval(() => {
    const t = nowMs();
    for (const [clientKey, meters] of clientMeters.entries()) {
      const sep = clientKey.lastIndexOf(":");
      if (sep < 0) continue;
      const address = clientKey.slice(0, sep);
      const cPort = Number(clientKey.slice(sep + 1));
      if (!address || !Number.isFinite(cPort)) continue;

      for (const [meterId, sub] of meters.entries()) {
        const intervalMs = Math.max(20, sub.intervalMs);
        if (t - sub.lastSentMs < intervalMs) continue;
        sub.lastSentMs = t;

        const floatCount = meterFloatCounts.get(meterId) ?? 0;
        const blob = makeX32MeterBlob(meterId, floatCount, meterGen, t, blobEndian);
        const pkt = encodeOscMessage(`/meters/${meterId}`, "b", [blob]);
        safeUdpSend(sock, pkt, cPort, address);
      }
    }
  }, 20);

  sock.on("message", (msg, rinfo) => {
    const parsed = decodeOscMessage(msg);
    if (!parsed) return;

    if (verbose) {
      console.log(
        `[fake-x32] <- ${rinfo.address}:${rinfo.port} ${parsed.address} ${parsed.types || ""}`,
      );
    }

    const clientKey = `${rinfo.address}:${rinfo.port}`;

    // Mixing Station typically sends /xremote as a keep-alive / registration.
    // Real X32 doesn't require a reply.
    if (parsed.address === "/xremote") {
      return;
    }

    if (parsed.address === "/info" || parsed.address === "/xinfo") {
      // Provide a somewhat realistic multi-line payload so clients parsing key/value pairs
      // don't crash on an unexpected format.
      const payload =
        parsed.address === "/xinfo"
          ? "model=X32\nfw=4.06\nname=FAKE-X32\noscsupport=1\n"
          : "mfr=Behringer\nmodel=X32\nfw=4.06\nname=FAKE-X32\n";
      const reply = encodeOscMessage(parsed.address, "s", [payload]);
      safeUdpSend(sock, reply, rinfo.port, rinfo.address);
      return;
    }

    if (parsed.address === "/meters") {
      const target = typeof parsed.args?.[0] === "string" ? parsed.args[0] : null;
      const m = typeof target === "string" ? target.match(/^\/meters\/(\d+)$/) : null;
      if (!m) return;
      const meterId = Number(m[1]);
      if (!Number.isInteger(meterId) || meterId < 0 || meterId > 16) return;

      // Time factor: 50ms * tf (clamp 1..99).
      const lastInt = parsed.args
        .slice(1)
        .reverse()
        .find((x) => Number.isInteger(x));
      const tf = clamp(toNumber(lastInt, 1), 1, 99);
      const intervalMs = 50 * tf;

      let meters = clientMeters.get(clientKey);
      if (!meters) {
        meters = new Map();
        clientMeters.set(clientKey, meters);
      }
      meters.set(meterId, { intervalMs, lastSentMs: 0 });

      console.log(
        `[fake-x32] /meters subscribe from ${rinfo.address}:${rinfo.port} id=${meterId} interval=${intervalMs}ms floats=${
          meterFloatCounts.get(meterId) ?? 0
        }`,
      );

      const floatCount = meterFloatCounts.get(meterId) ?? 0;
      const blob = makeX32MeterBlob(meterId, floatCount, meterGen, nowMs(), blobEndian);
      const pkt = encodeOscMessage(`/meters/${meterId}`, "b", [blob]);
      safeUdpSend(sock, pkt, rinfo.port, rinfo.address);
    }
  });

  sock.on("error", (err) => {
    console.error("[fake-x32] UDP server error:", err.message);
  });

  sock.bind(port, host);

  const shutdown = () => {
    clearInterval(tick);
    try {
      sock.close();
    } catch {}
    process.exit(0);
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
}

function formatDataGetPath(ch, paramPath, format) {
  return `/console/data/get/ch.${ch}.${paramPath}/${format || "val"}`;
}

function extractDataGet(msgPath) {
  // /console/data/get/ch.<n>.<param>/<format>
  const prefix = "/console/data/get/";
  if (typeof msgPath !== "string" || !msgPath.startsWith(prefix)) return null;
  const rest = msgPath.slice(prefix.length);
  const [pathPart, formatPart] = rest.split("/");
  const m = String(pathPart || "").match(/^ch\.(\d+)\.(.+)$/);
  if (!m) return null;
  return {
    channelIndex: Number(m[1]),
    paramPath: m[2],
    format: formatPart || "val",
  };
}

function isChStarPath(p) {
  return typeof p === "string" && p.startsWith("ch.*.");
}

function expandChStar(p, totalChannels) {
  // ch.*.cfg.name -> { paramPath: "cfg.name" }
  const m = String(p).match(/^ch\.\*\.(.+)$/);
  if (!m) return [];
  const paramPath = m[1];
  const out = [];
  for (let ch = 0; ch < totalChannels; ch++) {
    out.push({ ch, paramPath });
  }
  return out;
}

function makeDefaultNameForChannel(ch) {
  // Matches bridge constants (index.js): inputs 0..31, buses 48..63, main 70/71.
  if (ch >= 0 && ch <= 31) return `Input ${ch + 1}`;
  if (ch >= 48 && ch <= 63) {
    const busNum = ch - 48 + 1;
    const baseNum = busNum % 2 === 0 ? busNum - 1 : busNum;
    const side = busNum % 2 === 0 ? "R" : "L";
    return `Bus ${baseNum} ${side} MixBus`;
  }
  if (ch === 70) return "Main L";
  if (ch === 71) return "Main R";
  return `Ch ${ch + 1}`;
}

function makeInitialState(totalChannels) {
  /** @type {Record<number, Record<string, any>>} */
  const state = {};
  for (let ch = 0; ch < totalChannels; ch++) {
    state[ch] = {
      "cfg.name": makeDefaultNameForChannel(ch),
      "cfg.color": ch % 16,
      "mix.lvl": -12,
      "mix.on": true,
      "mix.pan": 0.5,
      solo: false,
      selected: false,
    };
  }
  return state;
}

function applySetToState(state, ch, paramPath, value) {
  if (!state[ch]) state[ch] = {};
  state[ch][paramPath] = value;
}

function makeStandaloneServer({
  host,
  port,
  totalChannels,
  meterGen,
  intervalMs,
  stereoMeterValues,
  verbose,
}) {
  const wss = new WebSocket.Server({ host, port });
  console.log(`[fake-ms] Listening on ws://${host}:${port}`);

  const channelState = makeInitialState(totalChannels);

  /** @type {{id:number, interval:number, params:{type:number,index:number}[], binary:boolean}|null} */
  let meteringSub = null;

  /** @type {Array<{path:string, format:string}>} */
  const dataSubs = [];

  const broadcast = (obj) => {
    for (const client of wss.clients) {
      safeSend(client, obj);
    }
  };

  const sendInitialForSub = (client, path, format) => {
    if (!isChStarPath(path)) return;
    for (const { ch, paramPath } of expandChStar(path, totalChannels)) {
      const value = channelState[ch]?.[paramPath];
      safeSend(client, {
        path: formatDataGetPath(ch, paramPath, format),
        body: { value },
      });
    }
  };

  let lastMeterLogMs = 0;
  const meterTimer = setInterval(() => {
    if (!meteringSub || !Array.isArray(meteringSub.params) || meteringSub.params.length === 0)
      return;
    const t = nowMs();
    const v = meteringSub.params.map((p) => {
      const ch = Number(p.index);
      const db = meterGen(t, ch);
      if (stereoMeterValues) return [db, db - 3];
      return [db];
    });
    if (verbose && t - lastMeterLogMs > 1000) {
      lastMeterLogMs = t;
      console.log(`[fake-ms] metering2 id=${meteringSub.id} channels=${v.length}`);
    }
    broadcast({ path: `/console/metering2/${meteringSub.id}`, body: { v } });
  }, intervalMs);

  wss.on("connection", (ws) => {
    console.log("[fake-ms] Client connected");

    ws.on("message", (buf) => {
      const text = Buffer.isBuffer(buf) ? buf.toString("utf8") : String(buf);
      if (verbose) console.log("[fake-ms] <-", text);
      const msg = safeJsonParse(text);
      if (!msg || typeof msg.path !== "string") return;

      // Keep-alive
      if (msg.path === "/hi/v") {
        safeSend(ws, { path: "/hi/v", body: { value: 1 } });
        return;
      }

      // Data subscribe/unsubscribe.
      if (msg.path === "/console/data/subscribe" && msg.method === "POST") {
        const path = msg.body?.path;
        const format = msg.body?.format || "val";
        if (typeof path === "string") {
          dataSubs.push({ path, format });
          sendInitialForSub(ws, path, format);
        }
        return;
      }
      if (msg.path === "/console/data/unsubscribe" && msg.method === "POST") {
        const path = msg.body?.path;
        const format = msg.body?.format || "val";
        if (typeof path === "string") {
          for (let i = dataSubs.length - 1; i >= 0; i--) {
            if (dataSubs[i].path === path && dataSubs[i].format === format) dataSubs.splice(i, 1);
          }
        }
        return;
      }

      // One-shot get.
      if (msg.method === "GET" && msg.path.startsWith("/console/data/get/ch.")) {
        const parsed = extractDataGet(msg.path);
        if (!parsed) return;
        const ch = parsed.channelIndex;
        if (!Number.isInteger(ch) || ch < 0 || ch >= totalChannels) return;
        const value = channelState[ch]?.[parsed.paramPath];
        safeSend(ws, { path: msg.path, body: { value } });
        return;
      }

      // Set: update state + echo back as an update.
      if (msg.method === "POST" && msg.path.startsWith("/console/data/set/ch.")) {
        // /console/data/set/ch.<n>.<param>/<format>
        const setPrefix = "/console/data/set/";
        const rest = msg.path.startsWith(setPrefix) ? msg.path.slice(setPrefix.length) : "";
        const [pathPart, formatPart] = rest.split("/");
        const m = String(pathPart || "").match(/^ch\.(\d+)\.(.+)$/);
        if (!m) return;
        const ch = Number(m[1]);
        const paramPath = m[2];
        const value = msg.body?.value;
        if (!Number.isInteger(ch) || ch < 0 || ch >= totalChannels) return;
        applySetToState(channelState, ch, paramPath, value);
        safeSend(ws, {
          path: formatDataGetPath(ch, paramPath, formatPart || "val"),
          body: { value },
        });
        return;
      }

      // Metering2 subscription.
      if (msg.path === "/console/metering2/subscribe" && msg.method === "POST") {
        const body = msg.body || {};
        const id = Number(body.id);
        const params = Array.isArray(body.params) ? body.params : [];
        meteringSub = {
          id: Number.isFinite(id) ? id : 0,
          interval: Number(body.interval) || intervalMs,
          binary: !!body.binary,
          params,
        };
        console.log(`[fake-ms] metering2 subscribed id=${meteringSub.id} params=${params.length}`);
        // No explicit ack needed; MS just starts sending /console/metering2/{id}.
        return;
      }
    });

    ws.on("close", () => {
      console.log("[fake-ms] Client disconnected");
    });
  });

  const shutdown = () => {
    clearInterval(meterTimer);
    wss.close(() => process.exit(0));
    for (const c of wss.clients) {
      try {
        c.close();
      } catch {}
    }
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
}

function makeProxyServer({
  host,
  port,
  upstreamUrl,
  meterGen,
  intervalMs,
  stereoMeterValues,
  verbose,
}) {
  const wss = new WebSocket.Server({ host, port });
  console.log(`[meter-proxy] Listening on ws://${host}:${port}`);
  console.log(`[meter-proxy] Upstream: ${upstreamUrl}`);

  wss.on("connection", (clientWs) => {
    console.log("[meter-proxy] Client connected");

    const upstream = new WebSocket(upstreamUrl);

    /** @type {{id:number, params:{type:number,index:number}[]}|null} */
    let meteringSub = null;

    let lastMeterLogMs = 0;
    const meterTimer = setInterval(() => {
      if (!meteringSub || !Array.isArray(meteringSub.params) || meteringSub.params.length === 0)
        return;
      const t = nowMs();
      const v = meteringSub.params.map((p) => {
        const ch = Number(p.index);
        const db = meterGen(t, ch);
        if (stereoMeterValues) return [db, db - 3];
        return [db];
      });
      if (verbose && t - lastMeterLogMs > 1000) {
        lastMeterLogMs = t;
        console.log(`[meter-proxy] metering2 id=${meteringSub.id} channels=${v.length}`);
      }
      safeSend(clientWs, { path: `/console/metering2/${meteringSub.id}`, body: { v } });
    }, intervalMs);

    const closeBoth = () => {
      clearInterval(meterTimer);
      try {
        clientWs.close();
      } catch {}
      try {
        upstream.close();
      } catch {}
    };

    upstream.on("open", () => {
      console.log("[meter-proxy] Connected to upstream");
    });

    upstream.on("message", (buf) => {
      const text = Buffer.isBuffer(buf) ? buf.toString("utf8") : String(buf);
      if (verbose) console.log("[meter-proxy] upstream <-", text);
      const msg = safeJsonParse(text);
      if (msg && typeof msg.path === "string" && msg.path.startsWith("/console/metering2/")) {
        // Drop upstream metering2; we inject our own.
        return;
      }
      if (clientWs.readyState === WebSocket.OPEN) clientWs.send(text);
    });

    upstream.on("close", () => {
      console.log("[meter-proxy] Upstream closed");
      closeBoth();
    });

    upstream.on("error", (e) => {
      console.warn("[meter-proxy] Upstream error:", e?.message || e);
    });

    clientWs.on("message", (buf) => {
      const text = Buffer.isBuffer(buf) ? buf.toString("utf8") : String(buf);
      if (verbose) console.log("[meter-proxy] client <-", text);
      const msg = safeJsonParse(text);

      // Capture metering2 subscription so we know what channels to generate.
      if (msg && msg.path === "/console/metering2/subscribe" && msg.method === "POST") {
        const id = Number(msg.body?.id);
        const params = Array.isArray(msg.body?.params) ? msg.body.params : [];
        meteringSub = { id: Number.isFinite(id) ? id : 0, params };
        console.log(
          `[meter-proxy] metering2 subscribed id=${meteringSub.id} params=${params.length}`,
        );
      }

      if (upstream.readyState === WebSocket.OPEN) upstream.send(text);
    });

    clientWs.on("close", () => {
      console.log("[meter-proxy] Client disconnected");
      closeBoth();
    });

    clientWs.on("error", (e) => {
      console.warn("[meter-proxy] Client error:", e?.message || e);
    });
  });

  process.on("SIGINT", () => wss.close(() => process.exit(0)));
  process.on("SIGTERM", () => wss.close(() => process.exit(0)));
}

function printHelpAndExit() {
  console.log(
    `\nFake metering helper for Softube-MS-Bridge\n\nUsage:\n  node tools/fake-metering-proxy.js [options]\n\nOptions:\n  --listen <port|host:port|ws://host:port>  Where to listen (default host is 0.0.0.0)\n  --upstream <ws://host:port>              If set, run as proxy to real Mixing Station\n  --x32                                    Run a minimal X32 OSC (UDP) emulator (responds to /info, /xinfo, /meters)\n  --x32BlobEndian <be|le>                   Endianness inside the /meters blob payload (default be)\n  --interval <ms>                          Meter update interval (MS: global; X32: shape only)\n  --mode <sine|random|saw|triangle>         Meter shape (default sine)\n  --minDb <n>                              Minimum dB (default -60)\n  --maxDb <n>                              Maximum dB (default -3)\n  --hz <n>                                 Oscillation frequency for sine/saw/triangle (default 0.25)\n  --seed <n>                               Deterministic random seed (only for mode=random)\n  --stereo                                 Send 2 values per meter entry (L/R) [Mixing Station only]\n  --verbose                                Log incoming messages and meter send stats\n\nExamples:\n  # Standalone fake Mixing Station server on 8082 (avoids clashing with real MS on 8080)\n  node tools/fake-metering-proxy.js --listen 8082 --interval 200 --mode sine --minDb -50 --maxDb -3\n\n  # Proxy real Mixing Station (8080) and inject meters; bridge connects to 8081\n  node tools/fake-metering-proxy.js --listen 8081 --upstream ws://127.0.0.1:8080 --interval 100 --mode random --minDb -45 --maxDb -6\n\n  # X32 emulator (UDP OSC) on 10023\n  node tools/fake-metering-proxy.js --x32 --listen 10023 --mode sine --minDb -50 --maxDb -3\n`,
  );
  process.exit(0);
}

function main() {
  const args = parseArgs(process.argv);
  if (args.help || args.h) printHelpAndExit();

  const x32Mode = !!args.x32;
  const verbose = !!args.verbose;
  const upstreamUrl = typeof args.upstream === "string" ? String(args.upstream) : null;

  // Defaults:
  // - Standalone WS: 8082 (avoid conflict with real Mixing Station on 8080)
  // - Proxy WS: 8081 (common pattern: upstream 8080, proxy 8081)
  // - X32 OSC: 10023
  const defaultPort = x32Mode ? 10023 : upstreamUrl ? 8081 : 8082;
  const listen = parseWsListen(String(args.listen || String(defaultPort)), defaultPort);

  if (x32Mode) {
    console.log(`[fake-x32] verbose=${verbose ? "on" : "off"}`);
  } else {
    console.log(`[fake-ms] verbose=${verbose ? "on" : "off"}`);
  }

  const intervalMs = clamp(toNumber(args.interval, 100), 20, 2000);
  const minDb = toNumber(args.minDb, -60);
  const maxDb = toNumber(args.maxDb, -3);
  const hz = toNumber(args.hz, 0.25);
  const seed = args.seed !== undefined ? toNumber(args.seed, 12345) : NaN;
  const stereoMeterValues = !!args.stereo;

  const meterGen = makeDbGenerator({
    mode: args.mode,
    minDb,
    maxDb,
    hz,
    seed: Number.isFinite(seed) ? seed : NaN,
  });

  if (x32Mode) {
    makeX32OscServer({
      host: listen.host,
      port: listen.port,
      meterGen,
      // Default to big-endian: if a client interprets the blob length as BE but we send LE,
      // it can read a huge float count and crash.
      blobEndian: typeof args.x32BlobEndian === "string" ? args.x32BlobEndian : "be",
      verbose,
    });
    return;
  }

  if (upstreamUrl) {
    makeProxyServer({
      host: listen.host,
      port: listen.port,
      upstreamUrl,
      meterGen,
      intervalMs,
      stereoMeterValues,
      verbose,
    });
  } else {
    // Standalone server: default to 80 channels (matches bridge MS_TOTAL_CHANNELS).
    makeStandaloneServer({
      host: listen.host,
      port: listen.port,
      totalChannels: 80,
      meterGen,
      intervalMs,
      stereoMeterValues,
      verbose,
    });
  }
}

main();
