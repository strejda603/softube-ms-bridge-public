#!/usr/bin/env node
/* eslint-disable no-console */

/**
 * Capture a real Mixing Station metering2 message.
 *
 * Why this exists:
 * Mixing Station can send metering2 updates in multiple shapes (scalar, flat array, nested arrays, or binary).
 * When debugging mapping issues, having ONE real raw frame is the fastest way to confirm the payload format.
 *
 * Usage examples:
 *   node tools/capture-metering2-message.js
 *   node tools/capture-metering2-message.js --url ws://127.0.0.1:8080 --channels 0-15 --count 3
 *   node tools/capture-metering2-message.js --channels 0,1,2,3 --interval 50 --id 0
 *   node tools/capture-metering2-message.js --binary
 *
 * Args:
 *   --url <wsUrl>         Mixing Station WS URL (default: ws://127.0.0.1:8080)
 *   --id <n>              Subscription id (default: 0)
 *   --interval <ms>       Update interval (default: 50)
 *   --type <n>            Meter type (default: 0)
 *   --channels <list>     Comma list ("0,1,2") or range ("0-15") (default: 0-15)
 *   --count <n>           How many metering frames to print (default: 1)
 *   --binary              Request binary payloads (default: false)
 *   --timeout <ms>        Exit with error if no frame arrives (default: 5000)
 */

const WebSocket = require("ws");

function parseArgs(argv) {
  /** @type {Record<string, string|boolean>} */
  const out = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) continue;
    const key = a.slice(2);
    const next = argv[i + 1];
    if (!next || next.startsWith("--")) {
      out[key] = true;
    } else {
      out[key] = next;
      i++;
    }
  }
  return out;
}

function parseChannelList(s) {
  const str = String(s || "").trim();
  if (!str) return [];

  // Range: 0-15
  const m = str.match(/^(-?\d+)\s*-\s*(-?\d+)$/);
  if (m) {
    const a = parseInt(m[1], 10);
    const b = parseInt(m[2], 10);
    if (!Number.isFinite(a) || !Number.isFinite(b)) return [];
    const start = Math.min(a, b);
    const end = Math.max(a, b);
    const res = [];
    for (let i = start; i <= end; i++) res.push(i);
    return res;
  }

  // Comma list: 0,1,2
  return str
    .split(",")
    .map((x) => parseInt(x.trim(), 10))
    .filter((n) => Number.isFinite(n));
}

function coerceWsPayloadToText(data) {
  if (typeof data === "string") return data;
  // ws may provide Buffer
  if (Buffer.isBuffer(data)) return data.toString("utf8");
  // ArrayBuffer
  if (
    data &&
    typeof data === "object" &&
    data.constructor &&
    data.constructor.name === "ArrayBuffer"
  ) {
    return Buffer.from(data).toString("utf8");
  }
  return String(data);
}

function usageAndExit(code) {
  console.log("Capture a real Mixing Station /console/metering2/{id} frame.");
  console.log("\nExamples:");
  console.log("  node tools/capture-metering2-message.js");
  console.log("  node tools/capture-metering2-message.js --channels 0-31 --count 3");
  console.log("  node tools/capture-metering2-message.js --url ws://127.0.0.1:8080 --binary");
  process.exit(code);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help || args.h) usageAndExit(0);

  const url = String(args.url || "ws://127.0.0.1:8080");
  const id = Number.isFinite(Number(args.id)) ? Number(args.id) : 0;
  const interval = Number.isFinite(Number(args.interval)) ? Number(args.interval) : 50;
  const type = Number.isFinite(Number(args.type)) ? Number(args.type) : 0;
  const channels = parseChannelList(args.channels || "0-15");
  const count = Number.isFinite(Number(args.count)) ? Math.max(1, Number(args.count)) : 1;
  const binary = !!args.binary;
  const timeoutMs = Number.isFinite(Number(args.timeout))
    ? Math.max(100, Number(args.timeout))
    : 5000;

  if (channels.length === 0) {
    console.error("No channels parsed from --channels");
    process.exit(2);
  }

  /** @type {WebSocket} */
  const ws = new WebSocket(url);

  let printed = 0;
  let timeoutId = null;

  const armTimeout = () => {
    if (timeoutId) clearTimeout(timeoutId);
    timeoutId = setTimeout(() => {
      console.error(`Timed out after ${timeoutMs}ms waiting for /console/metering2/${id}`);
      try {
        ws.close();
      } catch {
        // ignore
      }
      process.exit(3);
    }, timeoutMs);
  };

  ws.on("open", () => {
    console.log(`[capture-metering2] connected: ${url}`);

    const params = channels.map((ch) => ({ type, index: ch }));
    const req = {
      path: "/console/metering2/subscribe",
      method: "POST",
      body: {
        id,
        interval,
        binary,
        params,
      },
    };

    console.log("[capture-metering2] subscribing:");
    console.log(JSON.stringify(req, null, 2));

    ws.send(JSON.stringify(req));
    armTimeout();
  });

  ws.on("message", (data) => {
    const text = coerceWsPayloadToText(data);
    let msg;
    try {
      msg = JSON.parse(text);
    } catch {
      // ignore non-JSON
      return;
    }

    if (!msg || typeof msg.path !== "string") return;
    if (!msg.path.startsWith("/console/metering2/")) return;
    if (msg.path !== `/console/metering2/${id}`) return;

    printed++;
    console.log(`\n[capture-metering2] frame ${printed}/${count}:`);
    console.log(JSON.stringify(msg, null, 2));

    // Also print a compact one-liner that's easy to paste into chat/issues.
    console.log("\n[capture-metering2] compact:");
    console.log(JSON.stringify(msg));

    if (printed >= count) {
      if (timeoutId) clearTimeout(timeoutId);
      ws.close();
      process.exit(0);
    }

    armTimeout();
  });

  ws.on("close", () => {
    // If we already printed enough frames, we'll have exited.
    if (printed === 0) {
      console.error("[capture-metering2] socket closed before any frame arrived");
      process.exit(4);
    }
  });

  ws.on("error", (err) => {
    console.error("[capture-metering2] websocket error:", err && err.message ? err.message : err);
  });
}

main().catch((e) => {
  console.error("[capture-metering2] fatal:", e);
  process.exit(1);
});
