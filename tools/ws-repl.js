/**
 * Mixing Station WebSocket API REPL
 *
 * Usage: <METHOD> <PATH> [BODY_JSON]
 * BODY_JSON is optional and only needed for POST/PUT/PATCH requests that require a body.
 *
 * Example:
 *   GET /app/mixers/current
 *   POST /console/data/subscribe {"path":"channel.1.volume","format":"val"}
 *
 * Start: node ws-repl.js
 * Exit: Ctrl+C
 */

const WebSocket = require("ws");
const readline = require("readline");

// --- Mixing Station WebSocket connection ---
const MIXING_STATION_WS_URL = "ws://localhost:8080"; // Change if needed
let msWebSocket;
let wsReconnectTimeout = null;
let wsHeartbeatInterval = null;

/**
 * Establish and maintain a WebSocket connection to Mixing Station.
 * Handles auto-reconnect and keep-alive.
 * @example
 * connectMixingStationWebSocket();
 */
function connectMixingStationWebSocket() {
  if (msWebSocket && msWebSocket.readyState === WebSocket.OPEN) {
    msWebSocket.close();
  }
  msWebSocket = new WebSocket(MIXING_STATION_WS_URL);

  msWebSocket.on("open", () => {
    console.log("Connected to Mixing Station WebSocket");
    if (wsReconnectTimeout) {
      clearTimeout(wsReconnectTimeout);
      wsReconnectTimeout = null;
    }
    // Start heartbeat/keep-alive every 4 seconds
    if (wsHeartbeatInterval) clearInterval(wsHeartbeatInterval);
    wsHeartbeatInterval = setInterval(() => {
      if (msWebSocket && msWebSocket.readyState === WebSocket.OPEN) {
        sendToMixingStationWS({ path: "/hi/v", method: "GET" });
      }
    }, 4000);
  });

  msWebSocket.on("close", () => {
    const delay = 2000; // fixed 2 seconds
    console.log(`Mixing Station WebSocket closed, reconnecting in ${delay / 1000}s...`);
    if (wsReconnectTimeout) clearTimeout(wsReconnectTimeout);
    wsReconnectTimeout = setTimeout(connectMixingStationWebSocket, delay);
    if (wsHeartbeatInterval) {
      clearInterval(wsHeartbeatInterval);
      wsHeartbeatInterval = null;
    }
  });

  msWebSocket.on("error", (err) => {
    console.error("Mixing Station WebSocket error:", err.message);
    // Don't reconnect here, let 'close' handle it
  });

  msWebSocket.on("message", (data) => {
    try {
      const msg = JSON.parse(data);
      // Only forward the body of the response if it's not a heartbeat response
      if (msg.body && !msg.path.startsWith("/hi/")) {
        // Prettify JSON output for better readability
        console.log("\n[Mixing Station WS Response]:\n" + JSON.stringify(msg, null, 2));
      }
    } catch (e) {
      console.error("Error parsing Mixing Station WebSocket message:", e.message);
    }
  });
}

// --- Command Line Interface (REPL) ---
const rl = readline.createInterface({
  input: process.stdin,
  output: process.stdout,
  prompt: "WS-API> ",
});

/**
 * Start the WebSocket API REPL for Mixing Station.
 * Accepts commands in the form: <METHOD> <PATH> [BODY_JSON]
 * @example
 *   GET /app/mixers/current
 *   POST /console/data/subscribe {"path":"channel.1.volume","format":"val"}
 */
function startWebSocketApiRepl() {
  connectMixingStationWebSocket();

  rl.prompt();
  rl.on("line", (line) => {
    const trimmed = line.trim();
    if (!trimmed) {
      rl.prompt();
      return;
    }
    if (trimmed.toLowerCase() === "help") {
      console.log("Usage: <METHOD> <PATH> [BODY_JSON]");
      console.log(
        "BODY_JSON is optional and only needed for POST/PUT/PATCH requests that require a body."
      );
      console.log("This prompt sends a raw HTTP request over the WebSocket, like curl:");
      console.log("Example GET:   GET /app/mixers/current");
      console.log(
        'Example POST:  POST /console/data/subscribe {"path":"channel.1.volume","format":"val"}'
      );
      rl.prompt();
      return;
    }
    // Parse: METHOD PATH [BODY]
    const match = trimmed.match(/^(GET|POST|PUT|DELETE|PATCH)\s+(\S+)(\s+(.+))?$/i);
    if (!match) {
      console.log("Invalid input. Type 'help' for usage.");
      rl.prompt();
      return;
    }
    const method = match[1].toUpperCase();
    const path = match[2];
    let body = match[4] ? match[4].trim() : null;

    let req = {
      path: path,
      method: method,
      body: body ? JSON.parse(body) : undefined,
    };
    sendToMixingStationWS(req);
    // Prettify the request for confirmation
    console.log("\n[Sent over WebSocket]:\n" + JSON.stringify(req, null, 2));
    rl.prompt();
  });
  rl.on("SIGINT", () => {
    rl.close();
    process.exit(0);
  });
}

/**
 * Sends a message to Mixing Station via WebSocket if connected.
 * @param {object|string} msg - The message object or raw string to send.
 * @example
 * sendToMixingStationWS({ path: '/console/data/set/...' });
 */
function sendToMixingStationWS(msg) {
  if (msWebSocket && msWebSocket.readyState === WebSocket.OPEN) {
    if (typeof msg === "string") {
      msWebSocket.send(msg);
    } else {
      msWebSocket.send(JSON.stringify(msg));
    }
  }
}

console.log(
  "Type '<METHOD> <PATH> [BODY_JSON]' to send raw HTTP requests over WebSocket (Ctrl+C to exit). BODY_JSON is optional. Type 'help' for usage."
);

startWebSocketApiRepl();
