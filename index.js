/**
 * Softube Console 1 <-> Mixing Station Bridge
 *
 * Listens for SysEx MIDI messages from Softube Console 1, parses JSON payloads,
 * and sends corresponding commands to Mixing Station via WebSocket.
 *
 * Usage:
 *   1. Install Mixing Station and Softube Console 1 software.
 *   2. Install Node.js and dependencies: `npm install`.
 *   3. Connect Softube Console 1 (Fader Mk III) to your computer via USB.
 *   4. Start Mixing Station with WebSocket API enabled (default ws://localhost:8080).
 *   5. Run this script: `node index.js` or `npm run gui` for GUI mode.
 *
 * Author: Strejda603
 */
let midi;
try {
  midi = require("@julusian/midi");
} catch (err) {
  console.error("Failed to load '@julusian/midi' module:", err);
  process.exit(1);
}
const WebSocket = require("ws");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const { msColorToSoftubeColorInt } = require("./midiColorUtils");
const { clamp01, hybridStereoPanToDualMonoPans } = require("./panUtils");
const { decodeMetering2Binary, dbToConsole1MeterNorm } = require("./meteringUtils");
const {
  isNonEmptyString,
  resolveNextBooleanFromMomentary,
  isNegativeInfinityDb,
  normalizeConsole1LevelForMs,
  trimStereoSuffixFromName,
  coerceConsole1NumericString,
  coerceWsPayloadToText,
  readC1DspValue,
} = require("./valueCoercion");

// ###################################
// ############ CONSTANTS ############
// ###################################
// --- MIDI SysEx constants ---
const SYSEX_START = 0xf0;
const SYSEX_STOP = 0xf7;
const SYSEX_MANUFACTURER = 0x7d;
const SYSEX_MAGIC = [115, 116, 99, 49]; // "stc1"

// --- Mixing Station API config ---
let MIXING_STATION_WS_URL = "ws://localhost:8080";
const NUMBER_OF_SENDS = 6; // Console 1 has 6 send slots
const EQ_BAND_COUNT = 4; // Console 1 has 4 parametric EQ bands
let LOG_JSON = false; // Set to true to log all JSON messages to/from Mixing Station
let LOG_METERING = false; // Set to true to log metering subscription + incoming metering frames

// --- Metering (Mixing Station metering2) config ---
// Console 1 requests meters via `activeMeters`.
// Mixing Station docs: interval min=30, max=1000 (global per client, last one wins).
const METERING2_SUBSCRIPTION_ID = 0;
let METERING2_INTERVAL_MS = 100;
const METERING2_BINARY = false;

// --- Track layout config ---
// Mixing Station channel architecture can be discovered via `/console/information`.
// Defaults match Behringer X32-style layouts and are used as fallbacks.
let MS_TOTAL_CHANNELS = 80;
// Console 1 Fader Mk III: 10 faders. We build 10-wide banks of tracks.
// Current layout places Main only once (10th fader on the last bus bank).
const FADER_BANK_SIZE = 10;
const INPUTS_PER_BANK = 10; // Input banks use all faders; Main is placed only once (see `rebuildTrackLayout`).
let INPUT_CHANNEL_COUNT = 32; // Input channels 0..31
let BUS_CHANNEL_START = 48; // Bus channels 48..63
let BUS_CHANNEL_COUNT = 16;
let MAIN_STEREO_CHANNELS = [70, 71]; // Main L/R

// --- Console1 colors ---
// Console 1-only colors for non-input "virtual" tracks.
// These are intentionally NOT part of Mixing Station's 16-color palette (`baseColors`/styleClass).
// They are used only as visual markers on Console 1 and are never sent to Mixing Station.
// Softube color integer encoding: r in LSB, g in next byte, b in MSB. (BGR order in hex.)
let CONSOLE1_MAIN_COLOR = 0x00a5ff; // Orange-ish (r=255,g=165,b=0)
let CONSOLE1_BUS_COLOR = 0x800080; // Purple (r=128,g=0,b=128)

// ####################################
// ###### C1 <-> MS SEND MAPPING ######
// ####################################
// Console 1 exposes only 6 sends, while the mixer can have 16 buses.
// Map each C1 send slot (1..6) to the target MS bus number (1..16).
// Example: Send1 -> Bus1, Send2 -> Bus2, Send3 -> Bus3, Send4 -> Bus7, Send5 -> Bus9, Send6 -> Bus13
// let C1_SEND_TO_MS_BUS_NUMBER = [1, 2, 3, 7, 9, 13];
let C1_SEND_TO_MS_BUS_NUMBER = [];

/**
 * Build the mapping between Console 1 send slots (1..6) and Mixing Station send indices (0..15).
 *
 * - `c1ToMsSendIndex[i]` maps Console 1 send slot (i+1) -> MS send index.
 * - `msSendIndexToC1Slot` maps MS send index -> Console 1 slot index (0-based).
 *
 * If the configuration contains invalid values, the mapping falls back to 1:1 for that slot.
 *
 * @example
 * // C1 Send1 -> Bus1, Send2 -> Bus4
 * // translates to MS send indices [0, 3, ...]
 * const { c1ToMsSendIndex } = buildSendMapping();
 * console.log(c1ToMsSendIndex[0]); // 0
 * console.log(c1ToMsSendIndex[1]); // 3
 *
 * @returns {{ c1ToMsSendIndex: number[], msSendIndexToC1Slot: Map<number, number> }}
 */
function buildSendMapping() {
  const c1ToMsSendIndex = [];
  /** @type {Map<number, number>} */
  const msSendIndexToC1Slot = new Map();

  for (let i = 0; i < NUMBER_OF_SENDS; i++) {
    const busNum = Number(C1_SEND_TO_MS_BUS_NUMBER[i]);
    const msSendIndex = Number.isFinite(busNum) ? busNum - 1 : NaN;
    // If invalid, fall back to 1:1 mapping.
    const idx =
      Number.isInteger(msSendIndex) && msSendIndex >= 0 && msSendIndex < BUS_CHANNEL_COUNT
        ? msSendIndex
        : i;
    c1ToMsSendIndex[i] = idx;
    // If multiple C1 slots map to same MS send index, keep the first.
    if (!msSendIndexToC1Slot.has(idx)) msSendIndexToC1Slot.set(idx, i);
  }
  return { c1ToMsSendIndex, msSendIndexToC1Slot };
}

/** @type {number[]} */
let C1_SEND_TO_MS_SEND_INDEX = [];
/** @type {Map<number, number>} */
let MS_SEND_INDEX_TO_C1_SLOT = new Map();

/**
 * Rebuild and apply the Console 1 <-> Mixing Station send mapping from the current
 * `C1_SEND_TO_MS_BUS_NUMBER` config, replacing `C1_SEND_TO_MS_SEND_INDEX` and
 * `MS_SEND_INDEX_TO_C1_SLOT` in place.
 * @example
 * C1_SEND_TO_MS_BUS_NUMBER = [1, 2, 3, 7, 9, 13];
 * rebuildSendMapping();
 */
function rebuildSendMapping() {
  const { c1ToMsSendIndex, msSendIndexToC1Slot } = buildSendMapping();
  C1_SEND_TO_MS_SEND_INDEX = c1ToMsSendIndex;
  MS_SEND_INDEX_TO_C1_SLOT = msSendIndexToC1Slot;
}

rebuildSendMapping();

// ##################################
// ####### TRACK ORDER CONFIG #######
// ##################################
// Input track order on Console 1.
//
// Supports stereo-linked pairs by grouping two MS channels into one Console 1 track.
// For grouped stereo tracks, pan is handled by a hybrid width/balance control (writes both channels).
// prettier-ignore
// let INPUT_TRACK_ORDER = [16, 17, 18, 19, 20, 21, 22, 23, [14, 15], 9, 7, 8, [10, 11], [12, 13], 0, 1, 2, 4, 5, 6];
let INPUT_TRACK_ORDER = [];
// Bus master order on Console 1 (1-based bus numbers).
// Supports stereo-linked pairs by grouping two MS bus channels into one Console 1 track.
// prettier-ignore
// let BUS_TRACK_ORDER = [1, 2, [7, 8], 3, [9, 10], [13, 14], 4, [5, 6]];
let BUS_TRACK_ORDER = [];

// ###################################
// ###### BRIDGE CONFIG LOADING ######
// ###################################
/**
 * Load bridge configuration from JSON file and environment variables.
 *
 * Supported:
 * - `BRIDGE_CONFIG_PATH` (JSON file)
 * - `MIXING_STATION_WS_URL`
 * - `LOG_JSON` ("1"/"true")
 *
 * JSON file keys:
 * - mixingStationWsUrl
 * - logJson
 * - inputTrackOrder
 * - busTrackOrder
 * - c1SendToMsBusNumber
 * - metering2IntervalMs
 * - console1MainColor
 * - console1BusColor
 */
function loadBridgeConfig() {
  /** @type {any} */
  let fileConfig = {};

  const configPath = process.env.BRIDGE_CONFIG_PATH
    ? path.resolve(process.env.BRIDGE_CONFIG_PATH)
    : path.resolve(process.cwd(), "bridge-config.json");

  try {
    if (fs.existsSync(configPath)) {
      const raw = fs.readFileSync(configPath, "utf8");
      fileConfig = JSON.parse(raw);
    }
  } catch (e) {
    console.warn(`Failed to load config from ${configPath}: ${e.message || e}`);
  }

  const cfg = {
    mixingStationWsUrl: fileConfig.mixingStationWsUrl,
    logJson: fileConfig.logJson,
    inputTrackOrder: fileConfig.inputTrackOrder,
    busTrackOrder: fileConfig.busTrackOrder,
    c1SendToMsBusNumber: fileConfig.c1SendToMsBusNumber,
    metering2IntervalMs: fileConfig.metering2IntervalMs,
    console1MainColor: fileConfig.console1MainColor,
    console1BusColor: fileConfig.console1BusColor,
  };

  if (isNonEmptyString(process.env.MIXING_STATION_WS_URL)) {
    cfg.mixingStationWsUrl = process.env.MIXING_STATION_WS_URL.trim();
  }
  if (typeof process.env.LOG_JSON === "string") {
    const v = process.env.LOG_JSON.trim().toLowerCase();
    if (v === "1" || v === "true") cfg.logJson = true;
    if (v === "0" || v === "false") cfg.logJson = false;
  }

  if (isNonEmptyString(cfg.mixingStationWsUrl)) {
    MIXING_STATION_WS_URL = cfg.mixingStationWsUrl.trim();
  }
  if (typeof cfg.logJson === "boolean") LOG_JSON = cfg.logJson;

  if (Array.isArray(cfg.inputTrackOrder)) INPUT_TRACK_ORDER = cfg.inputTrackOrder;
  if (Array.isArray(cfg.busTrackOrder)) BUS_TRACK_ORDER = cfg.busTrackOrder;

  if (Array.isArray(cfg.c1SendToMsBusNumber)) {
    C1_SEND_TO_MS_BUS_NUMBER = cfg.c1SendToMsBusNumber;
    rebuildSendMapping();
  }

  if (cfg.metering2IntervalMs !== undefined) {
    const n = Number(cfg.metering2IntervalMs);
    if (Number.isFinite(n)) {
      METERING2_INTERVAL_MS = Math.max(30, Math.min(1000, Math.trunc(n)));
    }
  }

  if (typeof cfg.console1MainColor === "number") CONSOLE1_MAIN_COLOR = cfg.console1MainColor;
  if (typeof cfg.console1BusColor === "number") CONSOLE1_BUS_COLOR = cfg.console1BusColor;
}

loadBridgeConfig();

// ###################################
// ####### LIVE RUNTIME CONFIG #######
// ###################################
/**
 * Force a full Console 1 re-sync.
 *
 * This resets initialization state and schedules a full track dump flush.
 * Useful after reconnects or when layout/mapping changes.
 *
 * @param {string} reason
 */
function forceConsole1FullResync(reason) {
  // Always force, even if already initializing.
  isInitializing = true;
  initMessageBuffer = [];
  hasSentInitialTrackDump = false;

  if (initFlushTimeoutId) {
    clearTimeout(initFlushTimeoutId);
    initFlushTimeoutId = null;
  }

  const delay = Math.min(500, INIT_FLUSH_TIMEOUT_MS);
  initFlushTimeoutId = setTimeout(() => finalizeInitialization(reason), delay);
}

/**
 * Create a stable string key for the currently effective runtime config.
 * Used for change detection (avoid expensive rebuilds when no-op).
 * @returns {string}
 */
function stableConfigKeyFromRuntime() {
  return JSON.stringify({
    mixingStationWsUrl: String(MIXING_STATION_WS_URL || "ws://localhost:8080"),
    logJson: !!LOG_JSON,
    inputTrackOrder: Array.isArray(INPUT_TRACK_ORDER) ? INPUT_TRACK_ORDER : [],
    busTrackOrder: Array.isArray(BUS_TRACK_ORDER) ? BUS_TRACK_ORDER : [],
    c1SendToMsBusNumber: Array.isArray(C1_SEND_TO_MS_BUS_NUMBER)
      ? C1_SEND_TO_MS_BUS_NUMBER
      : [1, 2, 3, 4, 5, 6],
    console1MainColor: typeof CONSOLE1_MAIN_COLOR === "number" ? CONSOLE1_MAIN_COLOR : 0x00a5ff,
    console1BusColor: typeof CONSOLE1_BUS_COLOR === "number" ? CONSOLE1_BUS_COLOR : 0x800080,
  });
}

let lastAppliedRuntimeConfigKey = stableConfigKeyFromRuntime();

/**
 * Apply a config object to the current runtime globals.
 *
 * This updates in-memory settings but does not reconnect/resubscribe by itself.
 * Use `applyRuntimeConfigAndResync()` for the full live-apply behavior.
 *
 * @param {BridgeConfig} cfg
 * @returns {{urlChanged: boolean, anythingChanged: boolean}}
 * @example
 * const { urlChanged } = applyRuntimeConfig({ mixingStationWsUrl: "ws://127.0.0.1:8080" });
 * if (urlChanged) connectMixingStationWebSocket();
 */
function applyRuntimeConfig(cfg) {
  const prevKey = lastAppliedRuntimeConfigKey;
  const prevUrl = MIXING_STATION_WS_URL;

  // Apply supported config keys (same as loadBridgeConfig, but from an object).
  if (typeof cfg?.mixingStationWsUrl === "string" && cfg.mixingStationWsUrl.trim()) {
    MIXING_STATION_WS_URL = cfg.mixingStationWsUrl.trim();
  }
  if (typeof cfg?.logJson === "boolean") LOG_JSON = cfg.logJson;
  if (Array.isArray(cfg?.inputTrackOrder)) INPUT_TRACK_ORDER = cfg.inputTrackOrder;
  if (Array.isArray(cfg?.busTrackOrder)) BUS_TRACK_ORDER = cfg.busTrackOrder;

  if (Array.isArray(cfg?.c1SendToMsBusNumber)) {
    C1_SEND_TO_MS_BUS_NUMBER = cfg.c1SendToMsBusNumber;
    rebuildSendMapping();
  }

  if (typeof cfg?.console1MainColor === "number") CONSOLE1_MAIN_COLOR = cfg.console1MainColor;
  if (typeof cfg?.console1BusColor === "number") CONSOLE1_BUS_COLOR = cfg.console1BusColor;

  const nextKey = stableConfigKeyFromRuntime();
  lastAppliedRuntimeConfigKey = nextKey;

  const urlChanged = String(prevUrl || "") !== String(MIXING_STATION_WS_URL || "");
  const anythingChanged = prevKey !== nextKey;
  return { urlChanged, anythingChanged };
}

/**
 * Live-apply config updates and refresh bridge state without restarting the process.
 *
 * Behavior:
 * - If WS URL changes: reconnects the Mixing Station WebSocket.
 * - Otherwise: rebuilds track layout, clears caches, re-subscribes, and forces a full
 *   Console 1 resync (names/colors/mapping updates).
 *
 * @param {BridgeConfig} cfg
 * @param {string} [reason]
 * @returns {{changed:boolean,reconnected:boolean,resynced:boolean,beforeKey?:string}}
 * @example
 * applyRuntimeConfigAndResync({ logJson: true }, "enable logs");
 */
function applyRuntimeConfigAndResync(cfg, reason = "config apply") {
  const before = stableConfigKeyFromRuntime();
  const beforeUrl = MIXING_STATION_WS_URL;

  const { urlChanged, anythingChanged } = applyRuntimeConfig(cfg);
  if (!anythingChanged) {
    console.log("[config] No changes to apply");
    return { changed: false, reconnected: false, resynced: false };
  }

  console.log(
    `[config] Applied updates (wsUrlChanged=${urlChanged ? "yes" : "no"})` +
      (String(beforeUrl) !== String(MIXING_STATION_WS_URL)
        ? ` ${beforeUrl} -> ${MIXING_STATION_WS_URL}`
        : ""),
  );

  // If WS url changed, just reconnect. This is still much lighter than restarting the whole process.
  if (urlChanged) {
    connectMixingStationWebSocket();
    return { changed: true, reconnected: true, resynced: false };
  }
  // Otherwise: rebuild layout and force a full Console 1 re-sync so names/colors/mappings refresh.
  rebuildTrackLayout();
  // Reset per-track caches so we don't keep stale layout-derived objects.
  tracksByObjectId = {};
  objectIdByTrackId = new Map();
  trackIdByObjectId.clear();
  // Intentionally do NOT clear `usedTrackIds`: this prevents accidental trackId reuse
  // across live rebuilds within the same process.
  meteredObjectIds = new Set();
  metering2ParamMsChannels = [];
  msChannelMeterDb.clear();
  initSeenMsChannels = new Set();
  // Reset MS subscription bookkeeping so re-subscribing works without restarting.
  // (Subscriptions are per-WS connection; our in-memory map should reflect that.)
  wsDataSubscriptions = {};
  sendsModeMsSendIndex = null;
  sendsModeSubscribedMsSendIndex = null;
  inputSendState = new Map();

  if (msWebSocket && msWebSocket.readyState === WebSocket.OPEN) {
    // Reconnect rather than re-subscribing on the same live connection: Mixing Station only
    // pushes a channel's *current* value in response to a genuinely new subscription — a
    // redundant subscribe to a path it already considers subscribed is a no-op on its side.
    // Re-subscribing here without reconnecting left Console 1 showing stale/default values
    // until the next full reconnect (e.g. a Stop then Start). connectMixingStationWebSocket's
    // own "open" handler repeats the cache resets above (harmless) and re-runs the handshake,
    // subscribe, and finalizeInitialization sequence — the same proven path Start already uses.
    connectMixingStationWebSocket();
    return { changed: true, reconnected: true, resynced: true, beforeKey: before };
  }

  forceConsole1FullResync(reason);
  return { changed: true, reconnected: false, resynced: true, beforeKey: before };
}

// ###################################
// ##### RUNTIME CONTROL CHANNEL #####
// ###################################
/**
 * Listen for newline-delimited JSON control messages on stdin.
 *
 * Currently supported messages:
 * - `{ type: "config:apply", config: BridgeConfig }`
 */
function installRuntimeControlChannel() {
  // Allow the Electron GUI (or other parent processes) to apply config without restarting.
  if (!process.stdin || typeof process.stdin.on !== "function") return;
  try {
    process.stdin.setEncoding("utf8");
  } catch {
    // ignore
  }

  let buffer = "";
  process.stdin.on("data", (chunk) => {
    buffer += String(chunk || "");
    while (true) {
      const idx = buffer.indexOf("\n");
      if (idx < 0) break;
      const line = buffer.slice(0, idx).trim();
      buffer = buffer.slice(idx + 1);
      if (!line) continue;

      let msg;
      try {
        msg = JSON.parse(line);
      } catch {
        continue;
      }

      if (msg && msg.type === "config:apply") {
        // Standby-leak guard: applyRuntimeConfigAndResync() forces a full resync (creates
        // and activates real channel tracks), same concern as the RESET/handshake-ack/
        // open-handler guards elsewhere in this file. The GUI is only supposed to send this
        // while running, but that's an assumption about the caller, not something this
        // process can verify — a caller whose own state has drifted (e.g. it optimistically
        // marked itself "running" before this process actually confirmed it) could still
        // send one during standby, so guard defensively here too.
        if (bridgeLifecycle !== "running") {
          console.warn("[Lifecycle] Ignoring config:apply — not running.");
        } else {
          try {
            const cfg = msg.config || {};
            const res = applyRuntimeConfigAndResync(cfg, "config apply");
            if (LOG_JSON) console.log("[config] apply result", res);
          } catch (e) {
            console.warn("[config] Failed to apply config:", e?.message || e);
          }
        }
      }

      if (msg && msg.type === "lifecycle:start") {
        if (!midiInput || !midiOutput) {
          console.warn(
            "[Lifecycle] Ignoring lifecycle:start — Console 1 Fader MIDI ports not ready yet."
          );
        } else {
          try {
            enterRunningState(msg.config || {});
          } catch (e) {
            console.warn("[Lifecycle] Failed to enter running state:", e?.message || e);
          }
        }
      }

      if (msg && msg.type === "lifecycle:stop") {
        if (!midiInput || !midiOutput) {
          console.warn(
            "[Lifecycle] Ignoring lifecycle:stop — Console 1 Fader MIDI ports not ready yet."
          );
        } else {
          try {
            enterStandbyState();
          } catch (e) {
            console.warn("[Lifecycle] Failed to enter standby state:", e?.message || e);
          }
        }
      }

    }
  });
}

installRuntimeControlChannel();

// ============================================================================
// Everything below this line is free-standing function/state declarations —
// none of it executes at module-load time (only the bootstrap call at the very
// end does), so ordering here is purely about readability, not correctness.
// ============================================================================

// ####################################
// ###### C1 TRACK ID MANAGEMENT ######
// ####################################
// Console 1 identifies tracks by `trackId`.
// Keep them stable for the lifetime of the process (so incremental updates keep working),
// and ensure they're unique within this process (also across live layout rebuilds).
/** @type {Map<number, string>} objectId -> trackId */
const trackIdByObjectId = new Map();
/** @type {Set<string>} */
const usedTrackIds = new Set();

/** @returns {string} 8-hex uppercase */
function generateUniqueTrackId8Hex() {
  // 32-bit random => 8 hex chars.
  // Collision odds are extremely low; still guard with a Set.
  while (true) {
    const id = crypto.randomBytes(4).toString("hex").toUpperCase();
    if (!usedTrackIds.has(id)) {
      usedTrackIds.add(id);
      return id;
    }
  }
}

/**
 * Get or create a stable, unique trackId for a layout slot.
 *
 * @param {number} objectId
 * @returns {string}
 */
function getOrCreateTrackIdForObjectId(objectId) {
  const hit = trackIdByObjectId.get(objectId);
  if (hit) return hit;
  const id = generateUniqueTrackId8Hex();
  trackIdByObjectId.set(objectId, id);
  return id;
}

// ####################################
// ########### TRACK LAYOUT ###########
// ####################################
/** @type {TrackLayoutSlot[]} */
let trackLayout = [];
/** @type {Map<number, number[]>} */
let objectIdsByMsChannel = new Map();
let isInitializing = true;
/** @type {{channelIndex: number, paramPath: string, value: any}[]} */
let initMessageBuffer = [];
/** @type {Record<number, TrackInfo>} */
let tracksByObjectId = {};
let meteredObjectIds = new Set();
let initSeenMsChannels = new Set();

// Sends mode is latched by selecting a Bus master, and cleared by selecting Main.
// In sends mode, for input tracks:
// - Console 1 "Mute" button controls `mix.sends.<active>.on` (inverted: false => mute ON)
// - `pan` controls `mix.sends.<active>.pan`
// - Volume is not remapped for inputs. (Console 1 reads sends-mode fader data from `send<N>` fields.)
//
// For Bus/Main tracks we must keep behavior "standard" even while sends mode is active.
// Console 1 still reads fader state from `send<N>` + `send<N>On` in sends mode, so we proxy the
// fader display by mirroring `volume` into `send1..send6` and forcing `sendNOn=true`.
// Note: While sends mode is active, we reuse the Console 1 `mute` field as a *LED state*
// for the Mute button (derived as `!sendOn`). Standard mute state is refreshed when leaving sends mode.
let sendsModeMsSendIndex = null; // 0..15 or null
let sendsModeSubscribedMsSendIndex = null;

/** @returns {boolean} */
function isSendsModeActive() {
  return Number.isInteger(sendsModeMsSendIndex);
}

/**
 * @param {TrackLayoutSlot|null|undefined} slot
 * @returns {boolean}
 */
function isBusOrMain(slot) {
  return !!slot && (slot.kind === "bus" || slot.kind === "main");
}

/** @type {Map<number, {mixOn?: any, mixPan?: any}>} */
const inputStdState = new Map();

/** @type {Map<number, {on?: any, pan?: any}>} */
let inputSendState = new Map();

/**
 * Map an MS send index (0..15) to the Console 1 `sendNOn` field name, based on the current mapping.
 *
 * @param {number} msSendIndex
 * @returns {string|null} e.g. `send1On`, or null if the active send isn't mapped to any slot
 * @example
 * getConsole1SendOnFieldForMsSendIndex(0); // "send1On" (depends on mapping)
 */
function getConsole1SendOnFieldForMsSendIndex(msSendIndex) {
  if (!Number.isInteger(msSendIndex)) return null;
  const c1Slot = MS_SEND_INDEX_TO_C1_SLOT.get(msSendIndex);
  if (c1Slot === undefined) return null;
  return `send${c1Slot + 1}On`;
}

/**
 * Mirror a track's `volume` into all Console 1 send slots.
 *
 * Console 1 reads fader state from `send<N>` + `send<N>On` in sends mode.
 * For Bus/Main tracks (which must remain "standard"), we proxy the fader display by:
 * - forcing all `send<N>On` to true
 * - copying `volume` into each `send<N>`
 *
 * @param {TrackInfo} track
 * @param {any} volumeValue - Console 1 representation (number or "-Infinity")
 * @returns {Record<string, any>} partial of fields changed
 */
function mirrorConsole1SendSlotsFromVolume(track, volumeValue) {
  /** @type {Record<string, any>} */
  const partial = {};
  for (let i = 1; i <= NUMBER_OF_SENDS; i++) {
    const lvlKey = `send${i}`;
    const onKey = `send${i}On`;
    if (track[onKey] !== true) {
      track[onKey] = true;
      partial[onKey] = true;
    }
    if (track[lvlKey] !== volumeValue) {
      track[lvlKey] = volumeValue;
      partial[lvlKey] = volumeValue;
    }
  }
  return partial;
}

/**
 * Clear all Console 1 send slots (used when leaving sends mode for Bus/Main).
 * @param {TrackInfo} track
 * @returns {Record<string, any>} partial of fields changed
 */
function clearConsole1SendSlots(track) {
  /** @type {Record<string, any>} */
  const partial = {};
  for (let i = 1; i <= NUMBER_OF_SENDS; i++) {
    const lvlKey = `send${i}`;
    const onKey = `send${i}On`;
    if (track[onKey] !== false) {
      track[onKey] = false;
      partial[onKey] = false;
    }
    if (track[lvlKey] !== 0) {
      track[lvlKey] = 0;
      partial[lvlKey] = 0;
    }
  }
  return partial;
}

/**
 * Keep Bus/Main send slots in sync for Console 1's sends-mode fader display.
 * @param {boolean} isActive
 */
function syncBusMainSendSlotsForSendsMode(isActive) {
  for (let objectId = 0; objectId < trackLayout.length; objectId++) {
    const slot = trackLayout[objectId];
    if (!isBusOrMain(slot)) continue;

    const track = getOrCreateTrackInfo(objectId);
    const partial = isActive
      ? mirrorConsole1SendSlotsFromVolume(track, track.volume)
      : clearConsole1SendSlots(track);
    if (Object.keys(partial).length > 0) queueConsole1TrackUpdate(track.trackId, partial);
  }
}

/**
 * Request a single value from Mixing Station. This does not subscribe; it only triggers a one-shot update.
 *
 * @param {string} path - Data path relative to `/console/data/get/` (e.g. `ch.0.mix.lvl`)
 * @param {"val"|"norm"} format
 * @example
 * requestMixingStationValue("ch.0.mix.on", "val");
 */
function requestMixingStationValue(path, format) {
  // Mixing Station accepts GET requests for `/console/data/get/<path>/<format>`.
  sendToMixingStationWS({ path: `/console/data/get/${path}/${format}`, method: "GET" });
}

/**
 * Refresh input channels for sends mode.
 * @param {number} msSendIndex - The send index to refresh.
 */
function refreshInputsForSendsMode(msSendIndex) {
  if (!Number.isInteger(msSendIndex)) return;
  for (let ch = 0; ch < INPUT_CHANNEL_COUNT; ch++) {
    requestMixingStationValue(`ch.${ch}.mix.sends.${msSendIndex}.on`, "val");
    requestMixingStationValue(`ch.${ch}.mix.sends.${msSendIndex}.pan`, "norm");
  }
}

/**
 * Refresh input channels for standard mode.
 */
function refreshInputsForStandardMode() {
  for (let ch = 0; ch < INPUT_CHANNEL_COUNT; ch++) {
    requestMixingStationValue(`ch.${ch}.mix.on`, "val");
    requestMixingStationValue(`ch.${ch}.mix.pan`, "norm");
  }
}

/**
 * Apply standard mute and pan settings to input channels from the cache.
 */
function applyStandardMutePanToInputsFromCache() {
  for (let objectId = 0; objectId < trackLayout.length; objectId++) {
    const slot = trackLayout[objectId];
    if (!slot || slot.kind !== "input" || slot.msPrimary === null) continue;
    const track = tracksByObjectId[objectId];
    if (!track) continue;

    const st = inputStdState.get(slot.msPrimary);
    if (st && st.mixOn !== undefined) {
      const changed = applyMsParamToTrack(track, "mix.on", st.mixOn);
      queueConsole1TrackUpdate(track.trackId, changed);
    }
    if (slot.panLocked && Array.isArray(slot.msChannels) && slot.msChannels.length === 2) {
      // Stereo-linked pair: `track.pan` is a hybrid control; don't override it on mode switches.
    } else if (slot.panLocked) {
      const changed = applyMsParamToTrack(track, "mix.pan", 0.5);
      queueConsole1TrackUpdate(track.trackId, changed);
    } else if (st && st.mixPan !== undefined) {
      const changed = applyMsParamToTrack(track, "mix.pan", st.mixPan);
      queueConsole1TrackUpdate(track.trackId, changed);
    }
  }
}

/**
 * Enter/leave sends mode.
 *
 * When entering: dynamically subscribes to `mix.sends.<idx>.on` and `mix.sends.<idx>.pan`.
 * When leaving: unsubscribes and refreshes standard `mix.on` + `mix.pan`.
 *
 * @param {number|null} nextMsSendIndex - 0..15 for send index, or null to exit.
 */
function setSendsMode(nextMsSendIndex) {
  const normalized = Number.isInteger(nextMsSendIndex) ? nextMsSendIndex : null;
  if (normalized === sendsModeMsSendIndex) return;

  if (Number.isInteger(normalized)) {
    // 0-based MS send index, displayed as 1-based for readability.
    console.log(`[Mode] SENDS active (msSendIndex=${normalized}, bus=${normalized + 1})`);
  } else {
    console.log("[Mode] STANDARD active");
  }

  sendsModeMsSendIndex = normalized;
  inputSendState = new Map();

  // Console 1 reads fader values from send slots in sends mode.
  // For Bus/Main tracks, we keep them "standard" by proxying their fader display via send slots.
  syncBusMainSendSlotsForSendsMode(Number.isInteger(normalized));

  // Keep WS subscriptions minimal: only subscribe to the active send index on demand.
  if (Number.isInteger(sendsModeSubscribedMsSendIndex)) {
    unsubscribeFromChannelData(`ch.*.mix.sends.${sendsModeSubscribedMsSendIndex}.on`, "val");
    unsubscribeFromChannelData(`ch.*.mix.sends.${sendsModeSubscribedMsSendIndex}.pan`, "norm");
    sendsModeSubscribedMsSendIndex = null;
  }
  if (Number.isInteger(normalized)) {
    subscribeToChannelData(`ch.*.mix.sends.${normalized}.on`, "val");
    subscribeToChannelData(`ch.*.mix.sends.${normalized}.pan`, "norm");
    sendsModeSubscribedMsSendIndex = normalized;
    refreshInputsForSendsMode(normalized);
  } else {
    // Leaving sends mode: quickly re-sync standard mute/pan.
    applyStandardMutePanToInputsFromCache();
    refreshInputsForStandardMode();
  }
}

// ####################################
// ############# METERING #############
// ####################################
/** @type {number[]} Maps metering v[] index -> MS channel index */
let metering2ParamMsChannels = [];

/** @type {Map<number, number>} MS channel index -> latest dB value */
const msChannelMeterDb = new Map();

function computeSlotMeterNorm(slot) {
  if (!slot || !Array.isArray(slot.msChannels) || slot.msChannels.length === 0) return 0;
  let maxDb = -Infinity;
  for (const ch of slot.msChannels) {
    const db = msChannelMeterDb.get(ch);
    if (Number.isFinite(db)) maxDb = Math.max(maxDb, db);
  }
  if (!Number.isFinite(maxDb)) return 0;
  return dbToConsole1MeterNorm(maxDb);
}

/**
 * (Re)subscribe to Mixing Station metering2.
 *
 * Subscription is derived from `meteredObjectIds` (Console 1 activeMeters).
 * If no meters are enabled, sends an empty params list to clear the subscription.
 *
 * @example
 * meteredObjectIds = new Set([0, 1, 2]);
 * updateMetering2Subscription();
 */
function updateMetering2Subscription() {
  if (!msWebSocket || msWebSocket.readyState !== WebSocket.OPEN) return;

  /** @type {Set<number>} */
  const msChannelsSet = new Set();
  for (const objectId of meteredObjectIds) {
    const slot = trackLayout[objectId];
    if (!slot) continue;
    for (const ch of slot.msChannels) msChannelsSet.add(ch);
  }
  const msChannels = Array.from(msChannelsSet.values()).sort((a, b) => a - b);
  metering2ParamMsChannels = msChannels;

  const params = msChannels.map((ch) => ({ type: 0, index: ch }));
  if (LOG_JSON || LOG_METERING) {
    console.log(
      `[Metering] subscribe id=${METERING2_SUBSCRIPTION_ID} interval=${METERING2_INTERVAL_MS} binary=${METERING2_BINARY} params=${params.length}`,
    );
  }
  sendToMixingStationWS({
    path: "/console/metering2/subscribe",
    method: "POST",
    body: {
      id: METERING2_SUBSCRIPTION_ID,
      interval: METERING2_INTERVAL_MS,
      binary: METERING2_BINARY,
      params,
    },
  });
}

/**
 * Apply metering updates for the given MS channels.
 * @param {number[]} msChannels
 */
function applyMeterUpdatesForMsChannels(msChannels) {
  /** @type {Set<number>} */
  const affectedObjectIds = new Set();
  for (const ch of msChannels) {
    const objectIds = objectIdsByMsChannel.get(ch);
    if (!objectIds) continue;
    for (const objectId of objectIds) {
      if (meteredObjectIds.has(objectId)) affectedObjectIds.add(objectId);
    }
  }
  for (const objectId of affectedObjectIds) {
    const slot = trackLayout[objectId];
    if (!slot) continue;
    const track = getOrCreateTrackInfo(objectId);
    if (!track.isActive) continue;
    const next = computeSlotMeterNorm(slot);
    const prev = Array.isArray(track.meter) ? track.meter[0] : undefined;
    if (typeof prev !== "number" || Math.abs(prev - next) > 0.001) {
      track.meter = [next];
      queueConsole1TrackUpdate(track.trackId, { meter: track.meter });
    }
  }
}

/**
 * Handle websocket messages sent by Mixing Station for metering2 subscriptions.
 *
 * Expected json payload shapes (binary=false):
 * - Documented: `{ path: "/console/metering2/0", body: { v: [ [ -20 ], [ -18, -18 ], ... ] } }`
 * - Observed (single param): `{ path: "/console/metering2/0", body: { v: -90 } }`
 * - Observed (single param with multiple values): `{ path: "/console/metering2/0", body: { v: [ -20, -20 ] } }`
 * - Observed (multi param, flattened): `{ path: "/console/metering2/0", body: { v: [ -20, -19, ... ] } }`
 * - Observed (multi param, flattened with N values/param): `v` is a flat array with length = params * N
 *
 * @param {{path?:string, body?: any}} msg
 */
function handleMeteringMessage(msg) {
  const idStr = String(msg.path || "")
    .split("/")
    .pop();
  const id = parseInt(idStr, 10);
  if (!Number.isFinite(id) || id !== METERING2_SUBSCRIPTION_ID) return;

  const body = msg.body || {};
  if (metering2ParamMsChannels.length === 0) return;

  /** @type {number[]} */
  let maxDbByParam = [];

  // Mixing Station JSON payload variants seen in the wild:
  // - Docs: v = [ [db...], [db...], ... ] (one entry per params.*)
  // - Sometimes (especially with a single param): v = -90
  // - Sometimes (single param, stereo/extra meters): v = [ -20, -20 ]
  if (body.v !== undefined) {
    /** @type {any} */
    const rawV = body.v;

    /** @type {any[][]} */
    let vNormalized;

    if (Array.isArray(rawV)) {
      // If it's already nested (v[0] is an array), keep as-is.
      // Otherwise it can be either:
      // - per-param scalar list: v = [dbForParam0, dbForParam1, ...]
      // - single-param multi-value list: v = [dbL, dbR, ...]
      // - per-param flat list with N values per param: length = params * N
      if (rawV.length > 0 && Array.isArray(rawV[0])) {
        vNormalized = rawV;
      } else if (rawV.length === metering2ParamMsChannels.length) {
        vNormalized = rawV.map((x) => [x]);
      } else if (
        metering2ParamMsChannels.length > 0 &&
        rawV.length > metering2ParamMsChannels.length &&
        rawV.length % metering2ParamMsChannels.length === 0
      ) {
        const valuesPerParam = rawV.length / metering2ParamMsChannels.length;
        vNormalized = new Array(metering2ParamMsChannels.length);
        for (let i = 0; i < metering2ParamMsChannels.length; i++) {
          const start = i * valuesPerParam;
          vNormalized[i] = rawV.slice(start, start + valuesPerParam);
        }
      } else {
        vNormalized = [rawV];
      }
    } else {
      // Scalar dB value.
      vNormalized = [[rawV]];
    }

    const count = Math.min(vNormalized.length, metering2ParamMsChannels.length);
    maxDbByParam = new Array(count);
    for (let i = 0; i < count; i++) {
      const values = vNormalized[i];
      if (!Array.isArray(values)) {
        maxDbByParam[i] = -Infinity;
        continue;
      }
      let maxDb = -Infinity;
      for (const x of values) {
        const n = Number(x);
        if (Number.isFinite(n)) maxDb = Math.max(maxDb, n);
      }
      maxDbByParam[i] = maxDb;
    }
  } else if (typeof body.b === "string") {
    // Some MS versions/configs may send binary payloads (body.b) even if we asked for JSON.
    const decoded = decodeMetering2Binary(body.b, metering2ParamMsChannels.length);
    if (!decoded) {
      if (LOG_JSON || LOG_METERING) {
        console.log(
          `[Metering] metering2 binary payload received but could not be decoded (params=${metering2ParamMsChannels.length}, b64len=${body.b.length})`,
        );
      }
      return;
    }
    maxDbByParam = decoded.maxDbByParam;
    if (LOG_JSON || LOG_METERING) {
      console.log(
        `[Metering] metering2 binary received: params=${metering2ParamMsChannels.length} valuesPerParam=${decoded.valuesPerParam}`,
      );
    }
  } else {
    return;
  }

  /** @type {number[]} */
  const changedMsChannels = [];
  const count = Math.min(maxDbByParam.length, metering2ParamMsChannels.length);
  for (let i = 0; i < count; i++) {
    const msCh = metering2ParamMsChannels[i];
    const maxDb = maxDbByParam[i];
    if (!Number.isFinite(maxDb)) continue;
    const prevDb = msChannelMeterDb.get(msCh);
    if (prevDb === undefined || Math.abs(prevDb - maxDb) > 0.05) {
      msChannelMeterDb.set(msCh, maxDb);
      changedMsChannels.push(msCh);
    }
  }

  if (changedMsChannels.length > 0) applyMeterUpdatesForMsChannels(changedMsChannels);
}

// ###################################
// ############ MS WRITES ############
// ###################################
// Coalesce fast Console1 -> Mixing Station updates (faders, pans) to avoid WS spam.
const MS_WRITE_FLUSH_MS = 15;
let msWriteFlushTimer = null;
/** @type {Map<string, any>} */
const msWriteQueue = new Map();

/**
 * Queue a write to Mixing Station.
 *
 * By default we write using `/val`. Some parameters (like pan) are more stable
 * when written using `/norm`.
 *
 * @param {string} msPath - e.g. `ch.0.mix.lvl`
 * @param {any} value
 * @param {"val"|"norm"} [format="val"]
 */
function queueMsWrite(msPath, value, format = "val") {
  const key = `${msPath}|${format}`;
  msWriteQueue.set(key, value);
  if (msWriteFlushTimer) return;
  msWriteFlushTimer = setTimeout(() => {
    msWriteFlushTimer = null;
    if (!msWebSocket || msWebSocket.readyState !== WebSocket.OPEN) {
      msWriteQueue.clear();
      return;
    }
    const entries = Array.from(msWriteQueue.entries());
    msWriteQueue.clear();
    for (const [keyWithFormat, v] of entries) {
      const sep = keyWithFormat.lastIndexOf("|");
      const msPath2 = sep >= 0 ? keyWithFormat.slice(0, sep) : keyWithFormat;
      const fmt = sep >= 0 ? keyWithFormat.slice(sep + 1) : "val";
      noteMsWrite(`${msPath2}|${fmt}`, v);
      sendToMixingStationWS({
        path: `/console/data/set/${msPath2}/${fmt}`,
        method: "POST",
        body: { value: v },
      });
    }
  }, MS_WRITE_FLUSH_MS);
}

// ###################################
// ###### MS DATA SUBSCRIPTIONS ######
// ###################################
/**
 * Active Mixing Station data subscriptions keyed by `${path}|${format}`.
 * @type {Record<string, {path: string, format: string, timestamp: number}>}
 */
let wsDataSubscriptions = {};

/**
 * Subscribes to all Mixing Station data needed to drive the Console 1 OSD.
 * Uses the current send mapping to only subscribe to required send indices.
 */
function subscribeToRequiredChannelData() {
  // Core display + fader.
  subscribeToChannelData("ch.*.cfg.name", "val");
  subscribeToChannelData("ch.*.cfg.color", "val");
  subscribeToChannelData("ch.*.mix.lvl", "val");

  // Candidates / commonly used paths.
  subscribeToChannelData("ch.*.mix.pan", "norm");
  subscribeToChannelData("ch.*.solo", "val");
  subscribeToChannelData("ch.*.mix.on", "val");
  subscribeToChannelData("ch.*.selected", "val");

  for (const msSendIndex of C1_SEND_TO_MS_SEND_INDEX) {
    subscribeToChannelData(`ch.*.mix.sends.${msSendIndex}.lvl`, "val");
    subscribeToChannelData(`ch.*.mix.sends.${msSendIndex}.on`, "val");
  }
}

/**
 * Subscribe to channel data updates from Mixing Station.
 * @param {string} path
 * @param {string} format
 */
function subscribeToChannelData(path, format) {
  const subKey = `${path}|${format}`;
  if (wsDataSubscriptions[subKey]) return; // Already subscribed

  const req = {
    path: "/console/data/subscribe",
    method: "POST",
    body: { path: path, format: format },
  };
  sendToMixingStationWS(req);
  wsDataSubscriptions[subKey] = {
    path: path,
    format: format,
    timestamp: Date.now(),
  };
}

/**
 * Unsubscribe from channel data updates.
 * @param {string} path
 * @param {string} format
 */
function unsubscribeFromChannelData(path, format) {
  const subKey = `${path}|${format}`;
  if (!wsDataSubscriptions[subKey]) return; // Not subscribed

  const req = {
    path: "/console/data/unsubscribe",
    method: "POST",
    body: { path: path, format: format },
  };
  sendToMixingStationWS(req);
  delete wsDataSubscriptions[subKey];
}

// ###################################
// ###### TRACK LAYOUT BUILDING ######
// ###################################
/**
 * Build the Console 1 track layout.
 *
 * Layout rules:
 * - Input banks: 10-fader banks of reordered inputs (no repeated Main)
 * - Bus banks: reordered buses (with optional stereo groups); Main is placed only once on the
 *   10th fader of the last bus bank
 */
function rebuildTrackLayout() {
  /** @type {TrackLayoutSlot[]} */
  const slots = [];

  // Build ordered input tracks (mono or grouped stereo) and then append remaining inputs.
  /** @type {Set<number>} */
  const usedInputs = new Set();
  /** @type {Array<{msChannels:number[], msPrimary:number, panLocked:boolean}>} */
  const orderedInputTracks = [];

  const pushMono = (ch) => {
    if (!Number.isInteger(ch) || ch < 0 || ch >= INPUT_CHANNEL_COUNT) return;
    if (usedInputs.has(ch)) return;
    usedInputs.add(ch);
    orderedInputTracks.push({ msChannels: [ch], msPrimary: ch, panLocked: false });
  };

  const pushStereo = (left, right) => {
    if (!Number.isInteger(left) || !Number.isInteger(right)) return;
    if (left < 0 || left >= INPUT_CHANNEL_COUNT) return;
    if (right < 0 || right >= INPUT_CHANNEL_COUNT) return;
    if (left === right) return;
    if (usedInputs.has(left) || usedInputs.has(right)) return;
    usedInputs.add(left);
    usedInputs.add(right);
    orderedInputTracks.push({ msChannels: [left, right], msPrimary: left, panLocked: true });
  };

  for (const entry of INPUT_TRACK_ORDER) {
    if (Array.isArray(entry)) {
      if (entry.length === 2) pushStereo(Number(entry[0]), Number(entry[1]));
      continue;
    }
    pushMono(Number(entry));
  }

  for (let ch = 0; ch < INPUT_CHANNEL_COUNT; ch++) {
    if (!usedInputs.has(ch)) pushMono(ch);
  }

  // Input banks: 10 inputs.
  const inputBanks = Math.ceil(orderedInputTracks.length / INPUTS_PER_BANK);
  for (let bank = 0; bank < inputBanks; bank++) {
    for (let i = 0; i < INPUTS_PER_BANK; i++) {
      const inputPos = bank * INPUTS_PER_BANK + i;
      if (inputPos < orderedInputTracks.length) {
        const inputTrack = orderedInputTracks[inputPos];
        slots.push({
          objectId: slots.length,
          kind: "input",
          msChannels: inputTrack.msChannels,
          msPrimary: inputTrack.msPrimary,
          panLocked: inputTrack.panLocked,
        });
      } else {
        slots.push({ objectId: slots.length, kind: "empty", msChannels: [], msPrimary: null });
      }
    }
  }

  // Build ordered bus masters (mono or grouped stereo) and then append remaining buses.
  /** @type {Set<number>} */
  const usedBusNums = new Set();
  /** @type {Array<{msChannels:number[], msPrimary:number, panLocked:boolean}>} */
  const orderedBusTracks = [];

  const pushBusMono = (busNum) => {
    if (!Number.isInteger(busNum) || busNum < 1 || busNum > BUS_CHANNEL_COUNT) return;
    if (usedBusNums.has(busNum)) return;
    usedBusNums.add(busNum);
    const msCh = BUS_CHANNEL_START + (busNum - 1);
    orderedBusTracks.push({ msChannels: [msCh], msPrimary: msCh, panLocked: false });
  };

  const pushBusStereo = (leftBusNum, rightBusNum) => {
    if (!Number.isInteger(leftBusNum) || !Number.isInteger(rightBusNum)) return;
    if (leftBusNum < 1 || leftBusNum > BUS_CHANNEL_COUNT) return;
    if (rightBusNum < 1 || rightBusNum > BUS_CHANNEL_COUNT) return;
    if (leftBusNum === rightBusNum) return;
    if (usedBusNums.has(leftBusNum) || usedBusNums.has(rightBusNum)) return;
    usedBusNums.add(leftBusNum);
    usedBusNums.add(rightBusNum);
    const leftCh = BUS_CHANNEL_START + (leftBusNum - 1);
    const rightCh = BUS_CHANNEL_START + (rightBusNum - 1);
    orderedBusTracks.push({ msChannels: [leftCh, rightCh], msPrimary: leftCh, panLocked: true });
  };

  for (const entry of BUS_TRACK_ORDER) {
    if (Array.isArray(entry)) {
      if (entry.length === 2) pushBusStereo(Number(entry[0]), Number(entry[1]));
      continue;
    }
    pushBusMono(Number(entry));
  }

  for (let busNum = 1; busNum <= BUS_CHANNEL_COUNT; busNum++) {
    if (!usedBusNums.has(busNum)) pushBusMono(busNum);
  }

  // Bus masters: fill 10-fader banks with ordered buses, and put Main on the 10th fader
  // of the last bus bank.
  const busBanks = Math.max(1, Math.ceil((orderedBusTracks.length + 1) / FADER_BANK_SIZE));
  let busTrackIndex = 0;
  for (let bank = 0; bank < busBanks; bank++) {
    for (let i = 0; i < FADER_BANK_SIZE; i++) {
      const isLastSlotOfLastBusBank = bank === busBanks - 1 && i === FADER_BANK_SIZE - 1;
      if (isLastSlotOfLastBusBank) {
        slots.push({
          objectId: slots.length,
          kind: "main",
          msChannels: [...MAIN_STEREO_CHANNELS],
          msPrimary: MAIN_STEREO_CHANNELS[0],
        });
        continue;
      }

      if (busTrackIndex < orderedBusTracks.length) {
        const busTrack = orderedBusTracks[busTrackIndex++];
        slots.push({
          objectId: slots.length,
          kind: "bus",
          msChannels: busTrack.msChannels,
          msPrimary: busTrack.msPrimary,
          panLocked: busTrack.panLocked,
        });
      } else {
        slots.push({ objectId: slots.length, kind: "empty", msChannels: [], msPrimary: null });
      }
    }
  }

  trackLayout = slots;

  const map = new Map();
  for (const slot of slots) {
    for (const ch of slot.msChannels) {
      if (!map.has(ch)) map.set(ch, []);
      map.get(ch).push(slot.objectId);
    }
  }
  objectIdsByMsChannel = map;
}

// ###################################
// ############ OSD STATE ############
// ###################################
let osdEnabled = false;

/**
 * Enables On-Screen Display (OSD) integration.
 * @example
 * enableOSD();
 */
function enableOSD() {
  osdEnabled = true;
}

/**
 * Disables On-Screen Display (OSD) integration.
 * @example
 * disableOSD();
 */
function disableOSD() {
  osdEnabled = false;
}

// ###################################
// ##### C1 MIDI PORT MANAGEMENT #####
// ###################################
/**
 * Finds and opens the first MIDI input port matching Console 1 Fader Mk III DAW or MIDI.
 * Ensures SysEx, timing, and active sensing messages are not ignored.
 * @param {string[]} [preferredNames] - Optional array of preferred port names (exact match or substring).
 * @returns {midi.Input} Opened MIDI input instance.
 * @throws If no matching port is found.
 * @example
 * const midiIn = openSoftubeMidiInput();
 */
function openSoftubeMidiInput(preferredNames = ["Console 1 Fader Mk III DAW"]) {
  const input = new midi.Input();
  const portCount = input.getPortCount();
  for (let i = 0; i < portCount; i++) {
    const name = input.getPortName(i);
    if (preferredNames.some((pn) => name.includes(pn))) {
      input.openPort(i);
      // Enable SysEx, timing, and active sensing messages
      if (typeof input.ignoreTypes === "function") {
        input.ignoreTypes(false, false, false);
      }
      console.log("Listening to MIDI port:", name);
      return input;
    }
  }
  throw new Error("Console 1 Fader Mk III MIDI port not found!");
}

/** @type {import('@julusian/midi').Input | null} */
let midiInput = null;

/**
 * Finds and opens the first MIDI output port matching Console 1 Fader Mk III DAW or MIDI.
 * @param {string[]} [preferredNames] - Optional array of preferred port names (exact match or substring).
 * @returns {midi.Output} Opened MIDI output instance.
 * @throws If no matching port is found.
 * @example
 * const midiOut = openSoftubeMidiOutput();
 */
function openSoftubeMidiOutput(preferredNames = ["Console 1 Fader Mk III DAW"]) {
  const output = new midi.Output();
  const portCount = output.getPortCount();
  for (let i = 0; i < portCount; i++) {
    const name = output.getPortName(i);
    if (preferredNames.some((pn) => name.includes(pn))) {
      output.openPort(i);
      console.log("Opened MIDI output port:", name);
      return output;
    }
  }
  throw new Error("Console 1 Fader Mk III MIDI output port not found!");
}

/** @type {import('@julusian/midi').Output | null} */
let midiOutput = null;

const MIDI_PORT_RETRY_INTERVAL_MS = 2000;

/**
 * Try to open both Console 1 Fader MIDI ports once.
 * @returns {boolean} true if both ports are now open
 */
function tryOpenConsole1MidiPorts() {
  if (!midiInput) {
    try {
      midiInput = openSoftubeMidiInput();
    } catch {
      // not found yet; keep retrying
    }
  }
  if (!midiOutput) {
    try {
      midiOutput = openSoftubeMidiOutput();
    } catch {
      // not found yet; keep retrying
    }
  }
  return !!midiInput && !!midiOutput;
}

/**
 * Wait (retrying every `MIDI_PORT_RETRY_INTERVAL_MS`) until both Console 1 Fader MIDI ports
 * are found and opened. Unlike the bridge's original behavior, this never exits the
 * process — the GUI now spawns this process before the user has necessarily connected the
 * Console 1 Fader.
 * @returns {Promise<void>}
 */
function waitForConsole1MidiPorts() {
  return new Promise((resolve) => {
    if (tryOpenConsole1MidiPorts()) {
      resolve();
      return;
    }
    console.log("Waiting for Console 1 Fader MIDI ports...");
    const timer = setInterval(() => {
      if (tryOpenConsole1MidiPorts()) {
        clearInterval(timer);
        console.log("Console 1 Fader MIDI ports found.");
        resolve();
      }
    }, MIDI_PORT_RETRY_INTERVAL_MS);
  });
}

// ###################################
// ##### MS WEBSOCKET CONNECTION #####
// ###################################
let msWebSocket;
let wsReconnectTimeout = null;
let wsHeartbeatInterval = null;

/**
 * Runtime console architecture info (from `/console/information`).
 * We only apply it when explicitly requested during a WS connect, to avoid
 * changing channel mapping mid-session.
 */
let consoleInfoRequestState = { pending: false, accepted: false, resolve: null };

/**
 * Apply Mixing Station `/console/information` response to our channel constants.
 * Uses best-effort heuristics and keeps existing defaults as fallback.
 *
 * @param {{totalChannels?:number, channelTypes?:Array<any>}} info
 */
function applyConsoleInformation(info) {
  if (!info || typeof info !== "object") return;

  const total = Number(info.totalChannels);
  if (Number.isFinite(total) && total > 0) MS_TOTAL_CHANNELS = total;

  const types = Array.isArray(info.channelTypes) ? info.channelTypes : [];
  const findType = (re) =>
    types.find(
      (t) =>
        t &&
        (re.test(String(t.name || "")) || re.test(String(t.shortName || ""))) &&
        Number.isFinite(Number(t.offset)) &&
        Number.isFinite(Number(t.count)) &&
        Number(t.count) > 0,
    );

  // Inputs: prefer the type named like "Input" with offset 0 if available.
  const inputCandidates = types
    .filter(
      (t) =>
        t &&
        (/\binput\b|\bch\b/i.test(String(t.name || "")) ||
          /\binput\b|\bch\b/i.test(String(t.shortName || ""))) &&
        Number.isFinite(Number(t.offset)) &&
        Number.isFinite(Number(t.count)) &&
        Number(t.count) > 0,
    )
    .sort((a, b) => Number(a.offset) - Number(b.offset));
  const inputType = inputCandidates.find((t) => Number(t.offset) === 0) || inputCandidates[0];
  if (inputType) {
    const inputOffset = Number(inputType.offset);
    const inputCount = Number(inputType.count);
    // Bridge currently assumes inputs are indexed from 0.
    if (inputOffset === 0 && Number.isFinite(inputCount) && inputCount > 0) {
      INPUT_CHANNEL_COUNT = inputCount;
    }
  }

  // Buses.
  const busType = findType(/\bbus\b/i);
  if (busType) {
    const off = Number(busType.offset);
    const cnt = Number(busType.count);
    if (Number.isFinite(off) && off >= 0) BUS_CHANNEL_START = off;
    if (Number.isFinite(cnt) && cnt > 0) BUS_CHANNEL_COUNT = cnt;
  }

  // Main.
  const mainType = findType(/\bmain\b/i);
  if (mainType) {
    const off = Number(mainType.offset);
    const cnt = Number(mainType.count);
    if (Number.isFinite(off) && off >= 0 && Number.isFinite(cnt) && cnt > 0) {
      MAIN_STEREO_CHANNELS = cnt >= 2 ? [off, off + 1] : [off];
    }
  }

  if (LOG_JSON) {
    console.log(
      `[ConsoleInfo] total=${MS_TOTAL_CHANNELS} inputs=0..${INPUT_CHANNEL_COUNT - 1} bus=${BUS_CHANNEL_START}..${BUS_CHANNEL_START + BUS_CHANNEL_COUNT - 1} main=${MAIN_STEREO_CHANNELS.join(",")}`,
    );
  }
}

/**
 * Fetch `/console/information` once and apply it before building subscriptions.
 * Resolves even on timeout/failure (keeps fallbacks).
 *
 * @param {number} timeoutMs
 */
function fetchAndApplyConsoleInformation(timeoutMs = 500) {
  if (!msWebSocket || msWebSocket.readyState !== WebSocket.OPEN) {
    return Promise.resolve(false);
  }
  if (consoleInfoRequestState.pending) return Promise.resolve(false);

  consoleInfoRequestState.pending = true;
  consoleInfoRequestState.accepted = false;

  return new Promise((resolve) => {
    consoleInfoRequestState.resolve = (ok) => {
      if (!consoleInfoRequestState.pending) return;
      consoleInfoRequestState.pending = false;
      consoleInfoRequestState.accepted = !!ok;
      consoleInfoRequestState.resolve = null;
      resolve(!!ok);
    };

    sendToMixingStationWS({ path: "/console/information", method: "GET" });

    setTimeout(() => {
      if (
        consoleInfoRequestState.pending &&
        typeof consoleInfoRequestState.resolve === "function"
      ) {
        // Timeout: proceed with fallbacks and ignore any late reply.
        consoleInfoRequestState.resolve(false);
      }
    }, timeoutMs);
  });
}

/**
 * Establishes and maintains a WebSocket connection to Mixing Station.
 * Handles auto-reconnect and keep-alive.
 * @example
 * connectMixingStationWebSocket();
 */
function connectMixingStationWebSocket() {
  if (msWebSocket && msWebSocket.readyState === WebSocket.OPEN) {
    msWebSocket.close();
  }
  msWebSocket = new WebSocket(MIXING_STATION_WS_URL);
  // Captured so the close handler below can tell a stale socket's close event (e.g. the
  // just-closed previous connection, if its "close" fires after this new one is already
  // assigned to `msWebSocket`) from the current socket's own close — an unguarded stale
  // close event would otherwise schedule a spurious extra reconnect.
  const socket = msWebSocket;

  socket.on("open", async () => {
    console.log("Connected to Mixing Station WebSocket");

    // Best-effort: discover mixer channel architecture before building the layout/subscriptions.
    // If this times out or fails, we proceed with the built-in defaults.
    try {
      await fetchAndApplyConsoleInformation(600);
    } catch {
      // ignore
    }

    // Standby may have been entered while this await was in flight (e.g. Start then Stop
    // within the ~600ms console-information round trip). Same standby-leak concern as the
    // other guards in this file: resetting init state / scheduling finalizeInitialization
    // below would re-activate real channel tracks during standby. Bail out and let the
    // (already-triggered) close handle teardown.
    if (bridgeLifecycle !== "running") {
      try {
        msWebSocket.close();
      } catch {
        // ignore
      }
      return;
    }

    // Reset init state on each connect/reconnect.
    rebuildTrackLayout();
    isInitializing = true;
    initMessageBuffer = [];
    tracksByObjectId = {};
    meteredObjectIds = new Set();
    metering2ParamMsChannels = [];
    msChannelMeterDb.clear();
    objectIdByTrackId = new Map();
    initSeenMsChannels = new Set();
    hasSentInitialTrackDump = false;

    // Reset subscription bookkeeping so we re-subscribe on each new WS connection.
    wsDataSubscriptions = {};
    sendsModeMsSendIndex = null;
    sendsModeSubscribedMsSendIndex = null;
    inputSendState = new Map();

    if (initFlushTimeoutId) clearTimeout(initFlushTimeoutId);
    initFlushTimeoutId = setTimeout(() => finalizeInitialization("timeout"), INIT_FLUSH_TIMEOUT_MS);

    subscribeToRequiredChannelData();
    disableOSD();
    startHandshake();
    console.log("Console 1 handshake sent.");
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

  socket.on("close", () => {
    // Ignore a stale socket's close event — `msWebSocket` already points at a newer
    // connection (see the `socket` capture above), so this one's teardown is moot and must
    // not clear the newer socket's already-running heartbeat/init-flush timers.
    if (socket !== msWebSocket) return;

    if (wsHeartbeatInterval) {
      clearInterval(wsHeartbeatInterval);
      wsHeartbeatInterval = null;
    }
    // Clear regardless of branch below: a pending init-flush timeout left over from a very
    // recent connect (e.g. rapid Start-then-Stop) must not fire during standby — it would
    // force-send a stale full track dump via `finalizeInitialization()`.
    if (initFlushTimeoutId) {
      clearTimeout(initFlushTimeoutId);
      initFlushTimeoutId = null;
    }

    // Standby intentionally closes this socket (see enterStandbyState) — don't schedule
    // the usual auto-reconnect in that case, or standby would silently reconnect ~2s later.
    if (bridgeLifecycle !== "running") {
      console.log("Mixing Station WebSocket closed (standby, not reconnecting).");
      return;
    }

    const delay = 2000; // fixed 2 seconds
    console.warn(`Mixing Station WebSocket closed, reconnecting in ${delay / 1000}s...`);
    if (wsReconnectTimeout) clearTimeout(wsReconnectTimeout);
    wsReconnectTimeout = setTimeout(connectMixingStationWebSocket, delay);
  });

  msWebSocket.on("error", (err) => {
    console.error("Mixing Station WebSocket error:", err.message);
    // Don't reconnect here, let 'close' handle it
  });

  msWebSocket.on("message", (data) => {
    handleWSMessage(data);
  });
}

/**
 * Sends a message to Mixing Station via WebSocket if connected.
 * If not connected, the message is dropped (callers may retry later).
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

// ###################################
// ########### TRACK CACHE ###########
// ###################################
/**
 * Fast lookup from Console 1 `trackId` to our internal `objectId`.
 * Rebuilt as tracks are (re)created.
 * @type {Map<string, number>}
 */
let objectIdByTrackId = new Map();

/**
 * Creates the default Console 1 track object for a given layout slot.
 * @param {number} objectId - Layout slot index (0-based)
 * @returns {TrackInfo} New track object in Console 1 OSD format
 */
function createDefaultTrackForSlot(objectId) {
  const slot = trackLayout[objectId];
  const trackId = getOrCreateTrackIdForObjectId(objectId);
  if (LOG_JSON) console.log("Creating track slot:", objectId, "with trackId:", trackId);

  // Keep the fast lookup in sync.
  objectIdByTrackId.set(String(trackId), objectId);

  const kind = slot ? slot.kind : "empty";
  let isActive = true;
  let color = 6842214;
  let name = getDefaultNameForObjectId(objectId);
  let send1On = false;
  let send2On = false;
  let send3On = false;
  let send4On = false;
  let send5On = false;
  let send6On = false;
  if (kind === "bus") color = CONSOLE1_BUS_COLOR;
  else if (kind === "main") color = CONSOLE1_MAIN_COLOR;
  else if (kind === "empty") isActive = false;

  return {
    track: objectId + 1,
    isActive,
    trackId: trackId,
    color,
    name,
    volume: 0,
    meter: [0],
    mute: false,
    solo: false,
    selected: false,
    maxVolumeValue: 10.0,
    maxSendValue: 10.0,
    pan: 0.5,
    send1On: send1On,
    send1: 0,
    send2On: send2On,
    send2: 0,
    send3On: send3On,
    send3: 0,
    send4On: send4On,
    send4: 0,
    send5On: send5On,
    send5: 0,
    send6On: send6On,
    send6: 0,
    filterLcOn: false,
    filterLcFreq: 0,
    eq1On: false,
    eq1Freq: 0.5,
    eq1Gain: 0.5,
    eq1Q: 0.5,
    eq1Type: 0,
    eq2On: false,
    eq2Freq: 0.5,
    eq2Gain: 0.5,
    eq2Q: 0.5,
    eq2Type: 0,
    eq3On: false,
    eq3Freq: 0.5,
    eq3Gain: 0.5,
    eq3Q: 0.5,
    eq3Type: 0,
    eq4On: false,
    eq4Freq: 0.5,
    eq4Gain: 0.5,
    eq4Q: 0.5,
    eq4Type: 0,
    compOn: false,
    compRatio: 0,
    compAttack: 0,
    compRelease: 0,
    compMakeup: 0,
    compComp: 0,
    compKnee: 0,
    compWetdry: 0,
  };
}

/**
 * Compute a default display name for a layout slot.
 *
 * This is the fallback used when the mixer provides an empty channel name.
 *
 * @param {number} objectId - Layout slot index (0-based)
 * @returns {string}
 * @example
 * // Slot 0 representing MS channel 0
 * getDefaultNameForObjectId(0); // "Ch 1"
 */
function getDefaultNameForObjectId(objectId) {
  const slot = trackLayout[objectId];
  if (!slot) return "";

  const kind = slot.kind;
  const msPrimary = slot.msPrimary;

  if (kind === "input" && msPrimary !== null) {
    if (Array.isArray(slot.msChannels) && slot.msChannels.length === 2) {
      return `Ch ${slot.msChannels[0] + 1}+${slot.msChannels[1] + 1}`;
    }
    return `Ch ${msPrimary + 1}`;
  }

  if (kind === "bus" && msPrimary !== null) {
    if (Array.isArray(slot.msChannels) && slot.msChannels.length === 2) {
      return `Bus ${slot.msChannels[0] - BUS_CHANNEL_START + 1}+${
        slot.msChannels[1] - BUS_CHANNEL_START + 1
      }`;
    }
    return `Bus ${msPrimary - BUS_CHANNEL_START + 1}`;
  }

  if (kind === "main") return "Main";
  return "";
}

/**
 * Resolve the default layout-derived name for a cached track.
 *
 * @param {TrackInfo} track
 * @returns {string}
 */
function getDefaultNameForTrack(track) {
  // Prefer the resolved objectId map; fall back to the 1-based `track.track` index.
  let objectId = getObjectIdForTrackId(track.trackId);
  if (!Number.isInteger(objectId) && Number.isInteger(track.track)) objectId = track.track - 1;
  if (!Number.isInteger(objectId) || objectId < 0 || objectId >= trackLayout.length) return "";
  return getDefaultNameForObjectId(objectId);
}

/**
 * Get the cached Console 1 track object for a layout slot, creating it if needed.
 *
 * @param {number} objectId
 * @returns {TrackInfo}
 * @example
 * const track = getOrCreateTrackInfo(0);
 * console.log(track.trackId);
 */
function getOrCreateTrackInfo(objectId) {
  return (
    tracksByObjectId[objectId] || (tracksByObjectId[objectId] = createDefaultTrackForSlot(objectId))
  );
}

/**
 * Resolve a Console 1 `trackId` to our internal `objectId`.
 *
 * @param {string|number} trackId
 * @returns {number|undefined}
 */
function getObjectIdForTrackId(trackId) {
  const key = String(trackId);
  const hit = objectIdByTrackId.get(key);
  if (hit !== undefined) return hit;

  // Fallback for edge cases where cache was externally mutated.
  for (const objectIdStr of Object.keys(tracksByObjectId)) {
    const cached = tracksByObjectId[objectIdStr];
    if (!cached) continue;
    if (String(cached.trackId) === key) {
      const objectId = parseInt(objectIdStr, 10);
      if (Number.isFinite(objectId)) objectIdByTrackId.set(key, objectId);
      return objectId;
    }
  }
  return undefined;
}

// ####################################
// ######### BRIDGE LIFECYCLE #########
// ####################################
/**
 * Bridge lifecycle state (distinct from the unrelated "Standard"/"Sends" mode concept
 * tracked via `setSendsMode`/console logs prefixed `[Mode]` — this one uses `[Lifecycle]`).
 * - `"standby"`: Console 1 Fader MIDI held, no Mixing Station connection.
 * - `"running"`: full bridging active.
 * @type {"standby"|"running"}
 */
let bridgeLifecycle = "standby";

/**
 * Enter `standby`: disconnect from Mixing Station (if connected) and deactivate the real
 * channel banks' display. This is also used as the bridge's initial state at process start.
 */
function enterStandbyState() {
  // Set before closing the socket: the WS "close" handler below reads this to decide
  // whether to schedule its usual auto-reconnect, and that event fires asynchronously —
  // this must already read "standby" by the time it does.
  bridgeLifecycle = "standby";
  console.log("[Lifecycle] standby");

  if (
    msWebSocket &&
    (msWebSocket.readyState === WebSocket.OPEN || msWebSocket.readyState === WebSocket.CONNECTING)
  ) {
    msWebSocket.close();
  }
  if (wsReconnectTimeout) {
    clearTimeout(wsReconnectTimeout);
    wsReconnectTimeout = null;
  }

  batchDeactivateAllTracks(true);
}

/**
 * Enter `running`: apply the given config, connect to Mixing Station, and rebuild the full
 * track layout.
 * @param {BridgeConfig} config
 */
function enterRunningState(config) {
  bridgeLifecycle = "running";
  console.log("[Lifecycle] running");

  applyRuntimeConfig(config || {});
  rebuildTrackLayout();
  forceConsole1FullResync("lifecycle:start");
  connectMixingStationWebSocket();
}

// ####################################
// ############ MS UPDATES ############
// ####################################
// Suppress echo when we set a value in MS and MS immediately broadcasts it back.
// Keep this small; we delete entries as soon as we suppress the first matching echo.
const MS_ECHO_SUPPRESS_MS = 150;
/** @type {Map<string, {value:any, ts:number}>} */
const recentMsWrites = new Map();

/**
 * Apply a single Mixing Station update (path + value) onto a cached Console 1 track.
 * Returns a partial object containing only fields that changed.
 *
 * @param {TrackInfo} track - Cached track object to update
 * @param {string} paramPath - Parameter path relative to `ch.<n>.` (e.g. `mix.lvl`)
 * @param {any} value
 * @returns {Record<string, any>} Changed fields, suitable for `queueConsole1TrackUpdate()`
 */
function applyMsParamToTrack(track, paramPath, value) {
  /** @type {Record<string, any>} */
  const changed = {};

  const setIfChanged = (field, next) => {
    if (track[field] !== next) {
      track[field] = next;
      changed[field] = next;
    }
  };

  switch (paramPath) {
    case "cfg.name":
      if (typeof value !== "string") break;
      // Mixing Station may emit empty names (e.g. unassigned channels). In that case,
      // fall back to our layout-derived default label ("Ch N"/"Bus N"/etc.).
      if (value.trim().length === 0) {
        setIfChanged("name", getDefaultNameForTrack(track));
      } else {
        setIfChanged("name", value);
      }
      break;
    case "cfg.color":
      // Mixing Station may provide either a small palette index (0..15), a styleClass string,
      // or an already-encoded color. Convert to a Softube-style 24-bit integer.
      {
        const colorInt = msColorToSoftubeColorInt(value);
        if (typeof colorInt === "number") setIfChanged("color", colorInt);
      }
      break;
    case "mix.lvl":
      if (isNegativeInfinityDb(value)) {
        setIfChanged("volume", -Infinity);
      } else {
        setIfChanged("volume", value);
      }
      break;
    case "mix.on":
      setIfChanged("mute", !value);
      break;
    case "solo":
      setIfChanged("solo", !!value);
      break;
    case "mix.pan":
      setIfChanged("pan", value);
      break;
    case "info.isActive":
      setIfChanged("isActive", !!value);
      break;
    case "selected":
      setIfChanged("selected", !!value);
      break;
    default: {
      const sendMatch = paramPath.match(/^mix\.sends\.(\d+)\.(lvl|on)$/);
      if (sendMatch) {
        const msSendIndex = parseInt(sendMatch[1], 10);
        const kind = sendMatch[2];
        const c1Slot = MS_SEND_INDEX_TO_C1_SLOT.get(msSendIndex);
        if (c1Slot === undefined) break;
        const sendNumber = c1Slot + 1;

        if (kind === "lvl") {
          if (isNegativeInfinityDb(value)) {
            setIfChanged(`send${sendNumber}`, -Infinity);
          } else {
            setIfChanged(`send${sendNumber}`, value);
          }
        }
        if (kind === "on") setIfChanged(`send${sendNumber}On`, !!value);
      }
      break;
    }
  }

  return changed;
}

/**
 * Suppress echoes when we set a value in MS and MS immediately broadcasts it back.
 *
 * We suppress exactly one matching echo within `MS_ECHO_SUPPRESS_MS`, then delete the entry.
 * Key must include format to avoid pan `/val` vs `/norm` mismatches.
 *
 * @param {string} msKey - e.g. `ch.0.mix.pan|norm`
 * @param {any} value
 */
function shouldSuppressMsEcho(msKey, value) {
  const rec = recentMsWrites.get(msKey);
  if (!rec) return false;
  if (Date.now() - rec.ts > MS_ECHO_SUPPRESS_MS) {
    recentMsWrites.delete(msKey);
    return false;
  }
  // Loose equality is intentional: MS can represent booleans as 0/1.
  // eslint-disable-next-line eqeqeq
  if (rec.value == value) {
    // Suppress exactly one matching echo, then stop suppressing.
    recentMsWrites.delete(msKey);
    return true;
  }
  return false;
}

/**
 * Record a recent MS write so we can suppress its immediate echo.
 * @param {string} msKey
 * @param {any} value
 */
function noteMsWrite(msKey, value) {
  recentMsWrites.set(msKey, { value, ts: Date.now() });
}

// ###################################
// ######### C1 UPDATE QUEUE #########
// ###################################
// Batch incremental updates to Console 1 to avoid SysEx spam.
const CONSOLE1_FLUSH_MS = 20;
let console1FlushTimer = null;
/** @type {Map<string, object>} trackId -> partial track update */
const console1UpdateQueue = new Map();

/**
 * Queue a partial track update to Console 1 (batched + throttled).
 *
 * @example
 * queueConsole1TrackUpdate(track.trackId, { volume: -Infinity, mute: true });
 *
 * @param {string} trackId
 * @param {Record<string, any>} partial
 */
function queueConsole1TrackUpdate(trackId, partial) {
  if (!partial || Object.keys(partial).length === 0) return;
  const prev = console1UpdateQueue.get(trackId) || { trackId };
  console1UpdateQueue.set(trackId, { ...prev, ...partial, trackId });

  if (console1FlushTimer) return;
  console1FlushTimer = setTimeout(() => {
    console1FlushTimer = null;
    if (!osdEnabled || console1UpdateQueue.size === 0) return;
    const batch = Array.from(console1UpdateQueue.values());
    console1UpdateQueue.clear();
    for (let i = 0; i < batch.length; i += 100) {
      sendSysexToConsole1({ trackBatch: batch.slice(i, i + 100) });
    }
  }, CONSOLE1_FLUSH_MS);
}

// ####################################
// ########## INITIALIZATION ##########
// ####################################
// Initial bootstrap can stall if some channels never report; use a timeout flush.
const INIT_FLUSH_TIMEOUT_MS = 2500;
let initFlushTimeoutId = null;
let hasSentInitialTrackDump = false;

/**
 * End the initialization buffering phase and send a single full track dump.
 *
 * During initialization, Mixing Station can flood updates (especially at app startup).
 * We buffer them briefly, apply them to cache, then do one forced `trackBatch`.
 *
 * @param {string} reason
 */
function finalizeInitialization(reason) {
  if (!isInitializing) return;
  isInitializing = false;

  if (initFlushTimeoutId) {
    clearTimeout(initFlushTimeoutId);
    initFlushTimeoutId = null;
  }

  // Ensure all layout slots exist.
  for (let i = 0; i < trackLayout.length; i++) {
    if (!tracksByObjectId[i]) tracksByObjectId[i] = createDefaultTrackForSlot(i);
  }

  if (Array.isArray(initMessageBuffer)) {
    for (const m of initMessageBuffer) {
      if (!m) continue;
      const objectIds = objectIdsByMsChannel.get(m.channelIndex);
      if (!objectIds) continue;
      for (const objectId of objectIds) {
        const slot = trackLayout[objectId];
        if (!slot) continue;
        // For stereo Main, only read from the primary channel to avoid L/R fighting.
        if (slot.msPrimary !== null && m.channelIndex !== slot.msPrimary) continue;

        let valueForApply = m.value;
        if (m.paramPath === "cfg.name") {
          if (slot.kind === "main") valueForApply = "Main";
          else if (slot.panLocked && slot.msChannels.length === 2)
            valueForApply =
              typeof valueForApply === "string"
                ? trimStereoSuffixFromName(valueForApply)
                : valueForApply;
        }
        if (m.paramPath === "cfg.color") {
          if (slot.kind === "main") valueForApply = CONSOLE1_MAIN_COLOR;
          else if (slot.kind === "bus") valueForApply = CONSOLE1_BUS_COLOR;
        }

        // For stereo-linked pairs, ignore MS pan updates (we drive pan via the Console 1 hybrid knob).
        if (m.paramPath === "mix.pan" && slot.panLocked && slot.msChannels.length === 2) {
          continue;
        }
        // For other pan-locked tracks (if any), keep them centered on the OSD.
        if (m.paramPath === "mix.pan" && slot.panLocked) {
          valueForApply = 0.5;
        }
        applyMsParamToTrack(tracksByObjectId[objectId], m.paramPath, valueForApply);
      }
    }
  }

  initMessageBuffer = null;

  if (!hasSentInitialTrackDump) {
    if (LOG_JSON)
      console.log(`Finalizing initialization (${reason}). Sending initial track dump...`);
    batchSendAllTracks(true);
    hasSentInitialTrackDump = true;
  }
}

/**
 * Force a full re-sync of Console 1 state.
 *
 * Console 1 may send a `RESET` command when it wants the DAW/bridge to resend
 * track objects. We briefly re-enter initialization buffering, then send a
 * forced full `trackBatch` dump.
 *
 * @param {string} reason
 */
function scheduleConsole1FullResync(reason) {
  // If we're already initializing, let the existing timer handle the dump.
  if (isInitializing) return;

  isInitializing = true;
  initMessageBuffer = [];
  hasSentInitialTrackDump = false;

  if (initFlushTimeoutId) {
    clearTimeout(initFlushTimeoutId);
    initFlushTimeoutId = null;
  }

  // Use a short window; we primarily want to force a fresh full dump.
  const delay = Math.min(500, INIT_FLUSH_TIMEOUT_MS);
  initFlushTimeoutId = setTimeout(() => finalizeInitialization(reason), delay);
}

// ###################################
// ########### SYSEX TO C1 ###########
// ###################################
/**
 * Sends a JSON object as a SysEx message to Console 1 via MIDI output.
 * Handles encoding, escaping, and -Infinity serialization.
 * @param {object} jsonObj - The JSON object to send.
 * @param {boolean} [forceSend=false] - If true, send even if OSD is disabled.
 * @example
 * sendSysexToConsole1({ cmd: "ENABLE" });
 */
function sendSysexToConsole1(jsonObj, forceSend = false) {
  let data = [];
  data.push(SYSEX_START);
  data.push(SYSEX_MANUFACTURER);
  data = data.concat(SYSEX_MAGIC);

  function fixNegativeInf(key, value) {
    // -Infinity is not supported by JSON, but OSD expects "-Infinity" as a string
    if (typeof value === "number" && value === -Infinity) {
      return "-Infinity";
    }
    return value;
  }

  const jsonStr = JSON.stringify(jsonObj, fixNegativeInf);
  // Iterate by codepoints (handles surrogate pairs correctly).
  for (const ch of jsonStr) {
    const c = ch.codePointAt(0);
    if (c >= 0x20 && c < 0x7f) {
      data.push(c);
      continue;
    }

    // Use JSON standard code point escapes for non-ASCII.
    // Note: JSON escape is \uXXXX; for codepoints > 0xFFFF we emit surrogate pairs.
    const escape = (cp) => {
      const hex = cp.toString(16).padStart(4, "0");
      return `\\u${hex}`;
    };

    let escapeStr = "";
    if (c <= 0xffff) {
      escapeStr = escape(c);
    } else {
      const code = c - 0x10000;
      const hi = 0xd800 + ((code >> 10) & 0x3ff);
      const lo = 0xdc00 + (code & 0x3ff);
      escapeStr = escape(hi) + escape(lo);
    }
    for (let i = 0; i < escapeStr.length; i++) {
      data.push(escapeStr.codePointAt(i));
    }
  }
  data.push(SYSEX_STOP);
  if (osdEnabled || forceSend) {
    if (LOG_JSON) {
      console.log("Sending SysEx JSON to Console 1:", jsonObj);
      console.log("SysEx data:", Buffer.from(data).toString("hex"));
    }
    // Ports are opened lazily now (see waitForConsole1MidiPorts) and may still be null,
    // e.g. during a SIGINT shutdown that arrives before the Console 1 Fader was ever found.
    if (midiOutput) midiOutput.sendMessage(data);
  }
}

// --- Handshake and full-batch sends ---
/**
 * Starts handshake with Console 1 (OSD protocol).
 * @example
 * startHandshake();
 */
function startHandshake() {
  sendSysexToConsole1({ handshake: { dawName: "Mixing Station", protocolVersion: [1, 2] } }, true);
}

/**
 * Sends all tracks in the track cache in batches of 100 as SysEx messages.
 * @param {boolean} [forceSend=false] - If true, sends even when OSD is disabled.
 */
function batchSendAllTracks(forceSend = false) {
  let allTracksMsg = {
    trackBatch: [],
  };
  const keys = Object.keys(tracksByObjectId);
  for (let i = 0; i < keys.length; i++) {
    allTracksMsg.trackBatch.push(tracksByObjectId[keys[i]]);
    if (allTracksMsg.trackBatch.length >= 100) {
      sendSysexToConsole1(allTracksMsg, forceSend);
      allTracksMsg.trackBatch.length = 0;
    }
  }
  if (allTracksMsg.trackBatch.length > 0) {
    sendSysexToConsole1(allTracksMsg, forceSend);
  }
}

/**
 * Sends "remove" messages for all known tracks, like Cubase's `removeObject`.
 * This tells Console 1 OSD that each `trackId` is no longer active.
 *
 * @param {boolean} [forceSend=false] - If true, sends even when OSD is disabled.
 */
function batchDeactivateAllTracks(forceSend = false) {
  const keys = Object.keys(tracksByObjectId);
  /** @type {{trackId: string, isActive: boolean}[]} */
  let batch = [];

  for (let i = 0; i < keys.length; i++) {
    const track = tracksByObjectId[keys[i]];
    if (!track || track.trackId === undefined || track.trackId === null) continue;
    batch.push({ trackId: track.trackId, isActive: false });
    if (batch.length >= 100) {
      sendSysexToConsole1({ trackBatch: batch }, forceSend);
      batch = [];
    }
  }

  if (batch.length > 0) {
    sendSysexToConsole1({ trackBatch: batch }, forceSend);
  }
}

/**
 * Push current meter values for all Console 1-metered tracks (`meteredObjectIds`) in one
 * SysEx batch. No-ops if nothing is currently metered.
 * @example
 * batchSendChangedMeters();
 */
function batchSendChangedMeters() {
  if (meteredObjectIds.size === 0) return;
  let changed = {
    trackBatch: [],
  };

  for (const objectId of meteredObjectIds) {
    const track = getOrCreateTrackInfo(objectId);
    if (track === undefined || !track.isActive) continue;
    if (!Array.isArray(track.meter)) track.meter = [0];
    changed.trackBatch.push({ trackId: track.trackId, meter: track.meter });
  }

  if (changed.trackBatch.length > 0) {
    sendSysexToConsole1(changed);
  }
}

// ###################################
// ####### C1 CONTROL MESSAGES #######
// ###################################
/**
 * Handle Console 1 control messages (ENABLE/DISABLE/RESET/handshake ack).
 *
 * Note: Track-level MIDI updates are handled by `handleMidiMessage()`.
 *
 * @param {any} parsed
 */
function handleConsole1ControlJson(parsed) {
  // Extend this logic as needed for your protocol.
  try {
    if (parsed.cmd === "RESET") {
      console.log("Received RESET command.");
      // RESET usually means Console 1 cleared its object list and wants a full resend.
      // Re-send handshake, then force a full track dump.
      disableOSD();
      startHandshake();
      if (bridgeLifecycle === "running") {
        scheduleConsole1FullResync("console reset");
      }
      // In standby there's no Mixing Station data to resync, and `finalizeInitialization()`
      // (which `scheduleConsole1FullResync` schedules) would otherwise create AND activate
      // default tracks for every real channel slot in `trackLayout`, undoing standby's
      // deactivation. Nothing else to do — standby has no active tracks to re-affirm.
      return;
    }
    if (parsed.cmd === "ENABLE") {
      enableOSD();
      console.log("Enabling OSD.");
      return;
    }
    if (parsed.cmd === "DISABLE") {
      disableOSD();
      console.log("Disabling OSD.");
      return;
    }
    if (parsed.handshake && parsed.handshake.ack === true) {
      enableOSD();
      // If init hasn't completed yet, allow it to complete naturally; otherwise force a dump.
      // Same standby-leak concern as the RESET branch above: finalizeInitialization() would
      // activate every real channel slot, so only do this while actually running. This path
      // is reachable during standby because the RESET branch's startHandshake() solicits
      // this very ack.
      if (!hasSentInitialTrackDump && bridgeLifecycle === "running") {
        finalizeInitialization("handshake ack");
      }
    }
    if (parsed.activeMeters) {
      meteredObjectIds = new Set();
      for (const trackId of parsed.activeMeters) {
        const objectId = getObjectIdForTrackId(trackId);
        if (objectId !== undefined) {
          meteredObjectIds.add(objectId);
        }
      }
      updateMetering2Subscription();
      batchSendChangedMeters();
    }
  } catch (e) {
    console.log("Error parsing received JSON: " + e.toString());
  }
}

// ###################################
// #### MS WEBSOCKET MSG HANDLING ####
// ###################################
/**
 * Extract a `/console/data/get/ch.<n>.<param>/<format>` update from a WS message.
 *
 * @param {any} msg
 * @returns {{channelIndex:number,paramPath:string,format:"val"|"norm",value:any}|null}
 */
function parseChannelDataGetMessage(msg) {
  if (!msg || typeof msg.path !== "string") return null;
  if (!msg.path.startsWith("/console/data/get/ch.")) return null;

  const prefix = "/console/data/get/";
  const rest = msg.path.startsWith(prefix) ? msg.path.slice(prefix.length) : msg.path;
  const [pathPart, formatPart] = rest.split("/");
  const format = /** @type {"val"|"norm"} */ (formatPart === "norm" ? "norm" : "val");

  const m = pathPart.match(/^ch\.(\d+)\.(.+)$/);
  if (!m) return null;

  const channelIndex = parseInt(m[1], 10);
  const paramPath = m[2];
  const value =
    (msg.body && Object.prototype.hasOwnProperty.call(msg.body, "value")
      ? msg.body.value
      : undefined) ?? (Object.prototype.hasOwnProperty.call(msg, "value") ? msg.value : undefined);

  return { channelIndex, paramPath, format, value };
}

/**
 * Ensure cached tracks exist for all referenced layout slots.
 * @param {number[]} objectIds
 */
function ensureTracksForObjectIds(objectIds) {
  for (const objectId of objectIds) getOrCreateTrackInfo(objectId);
}

function shouldFinalizeInitializationEarly() {
  // Finish init early if we have seen at least one update for every MS channel
  // referenced by the current layout.
  let requiredCount = 0;
  for (const ch of objectIdsByMsChannel.keys()) requiredCount++;
  return initSeenMsChannels.size >= requiredCount;
}

/**
 * Buffer a channel update during initialization.
 * @param {number} channelIndex
 * @param {string} paramPath
 * @param {any} value
 */
function bufferInitUpdate(channelIndex, paramPath, value) {
  initMessageBuffer.push({ channelIndex, paramPath, value });
  initSeenMsChannels.add(channelIndex);
  if (shouldFinalizeInitializationEarly()) {
    finalizeInitialization("received all layout channels");
  }
}

/**
 * Apply "selected" based mode latching.
 * @param {TrackLayoutSlot} slot
 * @param {string} paramPath
 * @param {any} value
 */
function maybeLatchSendsModeFromSelection(slot, paramPath, value) {
  if (paramPath !== "selected" || !value) return;
  if (slot.kind === "bus" && slot.msPrimary !== null) {
    setSendsMode(slot.msPrimary - BUS_CHANNEL_START);
  } else if (slot.kind === "main") {
    setSendsMode(null);
  }
}

/**
 * Maintain a small cache of standard input mute/pan so we can restore quickly.
 * @param {TrackLayoutSlot} slot
 * @param {string} paramPath
 * @param {any} value
 */
function maybeUpdateInputStdState(slot, paramPath, value) {
  if (slot.kind !== "input" || slot.msPrimary === null) return;
  if (paramPath === "mix.on") {
    const prev = inputStdState.get(slot.msPrimary) || {};
    inputStdState.set(slot.msPrimary, { ...prev, mixOn: value });
  } else if (paramPath === "mix.pan") {
    const prev = inputStdState.get(slot.msPrimary) || {};
    inputStdState.set(slot.msPrimary, { ...prev, mixPan: value });
  }
}

/**
 * Handle sends-mode updates for input tracks.
 *
 * Returns true if the update was handled and no further processing should happen.
 *
 * @param {{objectId:number,slot:TrackLayoutSlot,channelIndex:number,paramPath:string,value:any,suppress:boolean}} args
 * @returns {boolean}
 */
function handleSendsModeInputUpdate(args) {
  const { objectId, slot, channelIndex, paramPath, value, suppress } = args;
  if (slot.kind !== "input" || !Number.isInteger(sendsModeMsSendIndex)) return false;

  const sendOnPath = `mix.sends.${sendsModeMsSendIndex}.on`;
  const sendPanPath = `mix.sends.${sendsModeMsSendIndex}.pan`;

  if (paramPath === "mix.on" || paramPath === "mix.pan") {
    // Ignore standard channel mute/pan while sends mode is active.
    return true;
  }

  if (paramPath === sendOnPath) {
    const st = inputSendState.get(channelIndex) || {};
    st.on = value;
    inputSendState.set(channelIndex, st);

    // Reflect the active send "on" state using Console 1's send slot field.
    // Also send a derived `mute` state so the Console 1 Mute LED reflects send on/off.
    const field = getConsole1SendOnFieldForMsSendIndex(sendsModeMsSendIndex);
    if (field) {
      const nextOn = !!value;
      const nextMuted = !nextOn;

      /** @type {Record<string, any>} */
      const partial = {};
      if (tracksByObjectId[objectId][field] !== nextOn) {
        tracksByObjectId[objectId][field] = nextOn;
        partial[field] = nextOn;
      }
      // In sends mode, `mute` is used as the send on/off LED state (!sendOn).
      // This must be updated even if `sendNOn` hasn't changed (e.g. first activation).
      if (tracksByObjectId[objectId].mute !== nextMuted) {
        tracksByObjectId[objectId].mute = nextMuted;
        partial.mute = nextMuted;
      }
      if (!suppress) queueConsole1TrackUpdate(tracksByObjectId[objectId].trackId, partial);
    }
    return true;
  }

  if (paramPath === sendPanPath) {
    const st = inputSendState.get(channelIndex) || {};
    st.pan = value;
    inputSendState.set(channelIndex, st);

    // For stereo-linked pairs, we drive pan from the Console 1 hybrid knob and write
    // dual-mono values to MS; avoid snapping the knob back from per-channel MS echoes.
    if (!(slot.panLocked && slot.msChannels.length === 2)) {
      const nextPan = value;
      if (tracksByObjectId[objectId].pan !== nextPan) {
        tracksByObjectId[objectId].pan = nextPan;
        if (!suppress)
          queueConsole1TrackUpdate(tracksByObjectId[objectId].trackId, { pan: nextPan });
      }
    }
    return true;
  }

  return false;
}

/**
 * Apply per-slot overrides for certain fields (name/color/pan).
 *
 * @param {TrackLayoutSlot} slot
 * @param {string} paramPath
 * @param {any} value
 * @returns {{handled:boolean,valueForApply:any}}
 */
function getValueForApplyWithSlotOverrides(slot, paramPath, value) {
  let valueForApply = value;

  // For virtual Main/Bus tracks, keep a fixed Console 1-only color regardless of MS.
  if (paramPath === "cfg.name") {
    if (slot.kind === "main") valueForApply = "Main";
    else if (slot.panLocked && slot.msChannels.length === 2) {
      valueForApply =
        typeof valueForApply === "string" ? trimStereoSuffixFromName(valueForApply) : valueForApply;
    }
  }
  if (paramPath === "cfg.color") {
    if (slot.kind === "main") valueForApply = CONSOLE1_MAIN_COLOR;
    else if (slot.kind === "bus") valueForApply = CONSOLE1_BUS_COLOR;
  }

  // For stereo-linked pairs, ignore MS pan updates (we drive pan via the Console 1 hybrid knob).
  if (paramPath === "mix.pan" && slot.panLocked && slot.msChannels.length === 2) {
    return { handled: true, valueForApply };
  }

  // For other pan-locked tracks (if any), keep them centered on the OSD.
  if (paramPath === "mix.pan" && slot.panLocked) {
    valueForApply = 0.5;
  }

  return { handled: false, valueForApply };
}

/**
 * Apply a single channel update to one layout slot.
 *
 * @param {{objectId:number,channelIndex:number,paramPath:string,format:string,value:any,suppress:boolean}} args
 */
function applyChannelUpdateToSlot(args) {
  const { objectId, channelIndex, paramPath, format, value } = args;
  const suppress = !!args.suppress;

  const slot = trackLayout[objectId];
  if (!slot) return;
  if (slot.msPrimary !== null && channelIndex !== slot.msPrimary) return;

  // Bus/Main tracks should always behave like standard tracks, even while sends mode is active.
  // Also, we don't want send-slot state (mix.sends.*) to show up on Bus/Main tracks at all.
  if (isBusOrMain(slot) && /^mix\.sends\./.test(paramPath)) {
    return;
  }

  // Bus masters: ignore Solo from both Console 1 and Mixing Station.
  if (slot.kind === "bus" && paramPath === "solo") {
    return;
  }

  maybeLatchSendsModeFromSelection(slot, paramPath, value);
  maybeUpdateInputStdState(slot, paramPath, value);

  if (handleSendsModeInputUpdate({ objectId, slot, channelIndex, paramPath, value, suppress })) {
    return;
  }

  const { handled, valueForApply } = getValueForApplyWithSlotOverrides(slot, paramPath, value);
  if (handled) return;

  const track = tracksByObjectId[objectId];
  const changed = applyMsParamToTrack(track, paramPath, valueForApply);

  // While in sends mode, proxy Bus/Main fader display via send slots.
  if (isSendsModeActive() && isBusOrMain(slot) && paramPath === "mix.lvl") {
    const extra = mirrorConsole1SendSlotsFromVolume(track, track.volume);
    Object.assign(changed, extra);
  }

  if (!suppress) queueConsole1TrackUpdate(track.trackId, changed);
}

/**
 * Handles incoming WebSocket messages, parses them, and dispatches updates to the appropriate handlers.
 *
 * - Parses the incoming data and determines the message type.
 * - Handles metering updates separately.
 * - For channel data messages, validates and applies updates to relevant track slots.
 * - Buffers updates if the system is initializing.
 * - Suppresses echo updates if necessary.
 *
 * @param {string|Buffer|ArrayBuffer} data - The raw WebSocket message payload.
 */
function handleWSMessage(data) {
  // Standby closes the WS, but close() is async — a message already in flight (or queued
  // in the event loop) can still arrive after enterStandbyState() has run. Processing it
  // could re-trigger finalizeInitialization() (via bufferInitUpdate's early-finalize path)
  // and re-activate/dump the real channel tracks standby just deactivated. Same bug class
  // as the RESET/handshake-ack/close-handler guards above; this is the WS-message entry
  // point they all ultimately fed through.
  if (bridgeLifecycle !== "running") return;
  try {
    const msg = JSON.parse(coerceWsPayloadToText(data));
    if (!msg || typeof msg.path !== "string") return;

    // Console architecture info.
    if (msg.path === "/console/information") {
      if (consoleInfoRequestState.pending && !consoleInfoRequestState.accepted) {
        applyConsoleInformation(msg.body);
        if (typeof consoleInfoRequestState.resolve === "function") {
          consoleInfoRequestState.resolve(true);
        }
      }
      return;
    }

    // Metering updates arrive as: /console/metering2/{id}
    if (msg.path.startsWith("/console/metering2/")) {
      handleMeteringMessage(msg);
      return;
    }

    const parsed = parseChannelDataGetMessage(msg);
    if (parsed) {
      const { channelIndex, paramPath, format, value } = parsed;
      if (channelIndex < 0 || channelIndex >= MS_TOTAL_CHANNELS) return;

      const objectIds = objectIdsByMsChannel.get(channelIndex);
      if (!objectIds || objectIds.length === 0) return;

      ensureTracksForObjectIds(objectIds);

      if (isInitializing) {
        bufferInitUpdate(channelIndex, paramPath, value);
        return;
      }

      // Apply to cache for each affected track slot.
      const echoKey = `ch.${channelIndex}.${paramPath}|${format}`;
      const suppress = shouldSuppressMsEcho(echoKey, value);
      for (const objectId of objectIds) {
        applyChannelUpdateToSlot({ objectId, channelIndex, paramPath, format, value, suppress });
      }
    }
    // Any other path (e.g. the heartbeat GET's reply) is intentionally ignored — we only
    // act on console info, metering2, and per-channel data-get messages.
  } catch (e) {
    console.error("Error parsing Mixing Station WebSocket message:", e.message);
  }
}

// ####################################
// ####### C1 MIDI MSG HANDLING #######
// ####################################
/**
 * Parses a SysEx MIDI message and extracts the JSON payload.
 * @param {number[]} message - MIDI message bytes.
 * @returns {object|null} Parsed JSON object or null if invalid.
 * @example
 * const obj = parseSysexJson([0xF0, 0x7D, ...]);
 */
function parseSysexJson(message) {
  // Minimum: start + manufacturer + magic + "{}" + stop.
  const minLen = 2 + SYSEX_MAGIC.length + 2 + 1;
  if (!Array.isArray(message) || message.length < minLen) return null;
  if (
    message[0] !== SYSEX_START ||
    message[1] !== SYSEX_MANUFACTURER ||
    message[message.length - 1] !== SYSEX_STOP
  ) {
    return null;
  }
  for (let i = 0; i < SYSEX_MAGIC.length; i++) {
    if (message[i + 2] !== SYSEX_MAGIC[i]) return null;
  }
  // Extract JSON string from SysEx
  const jsonBytes = message.slice(2 + SYSEX_MAGIC.length, -1);
  let jsonStr = "";
  for (let i = 0; i < jsonBytes.length; i++) {
    jsonStr += String.fromCharCode(jsonBytes[i]);
  }
  try {
    return JSON.parse(jsonStr);
  } catch (e) {
    console.error("Error parsing SysEx JSON:", e.message);
    return null;
  }
}

/**
 * Build a Console 1 update payload for Bus/Main fader display while sends mode is active.
 *
 * Console 1 reads fader state from `send<N>` + `send<N>On` in sends mode.
 * For Bus/Main tracks we keep semantics standard (write `mix.lvl`), but proxy the fader UI
 * by mirroring `volume` into all send slots.
 *
 * @param {TrackInfo} track
 * @param {any} nextVolumeForConsole1
 * @returns {Record<string, any>}
 */
function buildBusMainSendsModeFaderDisplayPartial(track, nextVolumeForConsole1) {
  track.volume = nextVolumeForConsole1;
  /** @type {Record<string, any>} */
  const partial = { volume: nextVolumeForConsole1 };
  Object.assign(partial, mirrorConsole1SendSlotsFromVolume(track, nextVolumeForConsole1));
  return partial;
}

/**
 * Write a Console 1 fader value to Mixing Station `mix.lvl` for all channels in a slot.
 *
 * @param {TrackLayoutSlot} slot
 * @param {Array<{msPath:string,value:any,format?:string}>} writes
 * @param {any} console1LevelValue - number, numeric string, or "-Infinity"
 */
function writeMixLvlForSlotFromConsole1(slot, writes, console1LevelValue) {
  const v = normalizeConsole1LevelForMs(console1LevelValue);
  if (v === null) return;
  for (const ch of slot.msChannels) {
    writes.push({ msPath: `ch.${ch}.mix.lvl`, value: v });
  }
}

/**
 * Handle `volume` updates from Console 1.
 *
 * - Echoes the value back to Console 1 immediately to avoid snap-back.
 * - Writes `mix.lvl` to ALL MS channels in the slot (mono or stereo pair).
 *
 * @param {any} parsed
 * @param {TrackLayoutSlot} slot
 * @param {TrackInfo} track
 * @param {Array<{msPath:string,value:any,format?:string}>} writes
 */
function handleMidiVolumeUpdate(parsed, slot, track, writes) {
  if (parsed.volume === undefined) return;

  const nextVolumeForConsole1 = coerceConsole1NumericString(parsed.volume);

  /** @type {Record<string, any>} */
  const partial =
    isSendsModeActive() && isBusOrMain(slot)
      ? buildBusMainSendsModeFaderDisplayPartial(track, nextVolumeForConsole1)
      : ((track.volume = nextVolumeForConsole1), { volume: nextVolumeForConsole1 });

  queueConsole1TrackUpdate(track.trackId, partial);

  writeMixLvlForSlotFromConsole1(slot, writes, parsed.volume);
}

/**
 * Find the first Console 1 send level key present in a parsed SysEx payload.
 *
 * @param {any} parsed
 * @returns {string|null} e.g. "send1" or null if none present
 */
function findFirstConsole1SendLevelKey(parsed) {
  for (let i = 1; i <= NUMBER_OF_SENDS; i++) {
    const lvlKey = `send${i}`;
    if (parsed && parsed[lvlKey] !== undefined) return lvlKey;
  }
  return null;
}

/**
 * Check whether any Console 1 send on/off keys are present in a parsed SysEx payload.
 *
 * @param {any} parsed
 * @returns {boolean}
 */
function hasAnyConsole1SendOnKey(parsed) {
  for (let i = 1; i <= NUMBER_OF_SENDS; i++) {
    const onKey = `send${i}On`;
    if (parsed && parsed[onKey] !== undefined) return true;
  }
  return false;
}

/**
 * In sends mode, Console 1 reads fader changes from `send<N>` for the selected track.
 * For Bus/Main tracks we want to keep semantics standard (write `mix.lvl`), so we treat
 * incoming send level moves as volume changes and also keep send slots mirrored.
 *
 * @param {any} parsed
 * @param {TrackLayoutSlot} slot
 * @param {TrackInfo} track
 * @param {Array<{msPath:string,value:any,format?:string}>} writes
 */
function handleMidiBusMainSendsModeFaderProxy(parsed, slot, track, writes) {
  if (!isSendsModeActive()) return;
  if (!isBusOrMain(slot)) return;

  const foundLevelKey = findFirstConsole1SendLevelKey(parsed);
  // If Console 1 toggles a send on/off while in sends mode, we keep it forced on.
  const sawAnyOnKey = hasAnyConsole1SendOnKey(parsed);
  if (!foundLevelKey && !sawAnyOnKey) return;

  const nextVolumeForConsole1 =
    foundLevelKey !== null ? coerceConsole1NumericString(parsed[foundLevelKey]) : track.volume;

  const partial = buildBusMainSendsModeFaderDisplayPartial(track, nextVolumeForConsole1);
  queueConsole1TrackUpdate(track.trackId, partial);

  // If we got a level, propagate to Mixing Station as a standard fader write.
  if (foundLevelKey !== null) {
    writeMixLvlForSlotFromConsole1(slot, writes, parsed[foundLevelKey]);
  }
}

/**
 * Handle `mute` updates from Console 1.
 *
 * In standard mode this maps to `mix.on` for input channels.
 * In sends mode this maps to toggling the active send on/off.
 *
 * @param {any} parsed
 * @param {TrackLayoutSlot} slot
 * @param {TrackInfo} track
 * @param {Array<{msPath:string,value:any,format?:string}>} writes
 */
function handleMidiMuteUpdate(parsed, slot, track, writes) {
  if (parsed.mute === undefined) return;
  if (slot.kind !== "input") {
    // Bus/Main: always treat mute as standard channel mute (not sends-mode toggle).
    if (!isBusOrMain(slot)) return;

    const currentMute = !!track.mute;
    const incomingMute = !!parsed.mute;
    const nextMute = resolveNextBooleanFromMomentary(currentMute, incomingMute);

    track.mute = nextMute;
    queueConsole1TrackUpdate(track.trackId, { mute: nextMute });

    for (const ch of slot.msChannels) {
      writes.push({ msPath: `ch.${ch}.mix.on`, value: nextMute ? 0 : 1 });
    }
    return;
  }

  if (isSendsModeActive()) {
    // Sends mode:
    // - Console 1 sends `mute` changes
    // - We interpret them as toggling the ACTIVE send's on/off
    // - We echo back using the Console 1 send slot field `sendNOn` (not `mute`)
    // - Mixing Station `mix.sends.<idx>.on` uses normal semantics (1 = on)

    const onField = getConsole1SendOnFieldForMsSendIndex(sendsModeMsSendIndex);
    if (onField) {
      // Console 1 mute semantics are inverted relative to "send on": mute=true means send off.
      const currentOn = !!track[onField];
      const currentMuted = !currentOn;
      const incomingMuted = !!parsed.mute;
      const nextMuted = resolveNextBooleanFromMomentary(currentMuted, incomingMuted);
      const nextOn = !nextMuted;

      // Optimistically update Console 1 immediately to avoid snap-back while waiting for MS.
      track[onField] = nextOn;
      track.mute = nextMuted;
      // Some Console 1 firmware still expects a `mute` field for the Mute button LED even
      // while we're using Mute as a sends on/off toggle
      queueConsole1TrackUpdate(track.trackId, { mute: nextMuted, [onField]: nextOn });

      for (const ch of slot.msChannels) {
        writes.push({
          msPath: `ch.${ch}.mix.sends.${sendsModeMsSendIndex}.on`,
          value: nextOn ? 1 : 0,
        });
      }
      // Note: in sends mode we intentionally repurpose `track.mute` as the Mute LED state
      // (derived from send on/off) so the Console 1 UI stays consistent.
      return;
    }

    // If the active send isn't part of the 6-slot mapping, we can still write to MS,
    // but there's no correct `sendNOn` field to reflect on Console 1.
    const currentMuteFallback = !!track.mute;
    const incomingMuteFallback = !!parsed.mute;
    const nextMuteFallback = resolveNextBooleanFromMomentary(
      currentMuteFallback,
      incomingMuteFallback,
    );
    for (const ch of slot.msChannels) {
      writes.push({
        msPath: `ch.${ch}.mix.sends.${sendsModeMsSendIndex}.on`,
        value: nextMuteFallback ? 0 : 1,
      });
    }
    return;
  }

  // Standard mode: `mix.on` = channel is ON (1) / OFF (0). OFF means muted.
  // Console 1 mute button may send either the new state (true/false), or a momentary
  // "pressed" event (often always `true`). To support both, treat "same as current" as toggle.
  const currentMute = !!track.mute;
  const incomingMute = !!parsed.mute;
  const nextMute = resolveNextBooleanFromMomentary(currentMute, incomingMute);

  // Optimistically update Console 1 immediately to avoid snap-back while waiting for MS.
  track.mute = nextMute;
  queueConsole1TrackUpdate(track.trackId, { mute: nextMute });

  for (const ch of slot.msChannels) {
    writes.push({ msPath: `ch.${ch}.mix.on`, value: nextMute ? 0 : 1 });
  }
}

/**
 * Handle `solo` updates from Console 1.
 * @param {any} parsed
 * @param {TrackLayoutSlot} slot
 * @param {TrackInfo} track
 * @param {Array<{msPath:string,value:any,format?:string}>} writes
 */
function handleMidiSoloUpdate(parsed, slot, track, writes) {
  if (parsed.solo === undefined) return;
  if (slot.kind !== "input" && !isBusOrMain(slot)) return;

  // Bus masters: ignore Solo from both Console 1 and Mixing Station.
  if (slot.kind === "bus") return;

  // Like mute, solo can arrive as state or as a momentary "pressed" event.
  const currentSolo = !!track.solo;
  const incomingSolo = !!parsed.solo;
  const nextSolo = incomingSolo === currentSolo ? !currentSolo : incomingSolo;

  track.solo = nextSolo;
  queueConsole1TrackUpdate(track.trackId, { solo: nextSolo });

  // Normalize to 0/1; some mixers don't accept boolean reliably.
  for (const ch of slot.msChannels) {
    writes.push({ msPath: `ch.${ch}.solo`, value: nextSolo ? 1 : 0 });
  }
}

/**
 * Handle `pan` updates from Console 1.
 *
 * - In sends mode: writes `mix.sends.<idx>.pan`.
 * - In standard mode: writes `mix.pan`.
 * - For stereo-linked pairs: uses the hybrid width/balance control and writes BOTH channels.
 *
 * @param {any} parsed
 * @param {TrackLayoutSlot} slot
 * @param {TrackInfo} track
 * @param {number} primaryChannel
 * @param {Array<{msPath:string,value:any,format?:string}>} writes
 */
function handleMidiPanUpdate(parsed, slot, track, primaryChannel, writes) {
  if (parsed.pan === undefined) return;
  if (slot.kind !== "input") {
    // Bus/Main: always treat pan as standard channel pan (not sends-mode send-pan).
    if (!isBusOrMain(slot)) return;

    const knobPan = clamp01(Number(parsed.pan));
    track.pan = knobPan;
    queueConsole1TrackUpdate(track.trackId, { pan: knobPan });

    const isStereoLinkedPair =
      !!slot.panLocked && Array.isArray(slot.msChannels) && slot.msChannels.length === 2;

    if (isStereoLinkedPair) {
      const { left, right } = hybridStereoPanToDualMonoPans(knobPan);
      const [leftCh, rightCh] = slot.msChannels;
      writes.push({ msPath: `ch.${leftCh}.mix.pan`, value: left, format: "norm" });
      writes.push({ msPath: `ch.${rightCh}.mix.pan`, value: right, format: "norm" });
    } else {
      writes.push({ msPath: `ch.${primaryChannel}.mix.pan`, value: knobPan, format: "norm" });
    }
    return;
  }

  const isStereoLinkedPair =
    !!slot.panLocked && Array.isArray(slot.msChannels) && slot.msChannels.length === 2;

  if (isSendsModeActive()) {
    // Sends mode: pan controls send pan.
    // For stereo-linked pairs, apply the hybrid width/balance control as dual-mono panning.
    const knobPan = clamp01(Number(parsed.pan));
    track.pan = knobPan;
    queueConsole1TrackUpdate(track.trackId, { pan: knobPan });

    if (isStereoLinkedPair) {
      const { left, right, width, mid } = hybridStereoPanToDualMonoPans(knobPan);
      const [leftCh, rightCh] = slot.msChannels;

      if (LOG_JSON) {
        console.log("[hybrid-pan] sends", {
          trackId: track.trackId,
          msChannels: [leftCh, rightCh],
          msSendIndex: sendsModeMsSendIndex,
          knobPan,
          left,
          right,
          width,
          mid,
        });
      }

      writes.push({
        msPath: `ch.${leftCh}.mix.sends.${sendsModeMsSendIndex}.pan`,
        value: left,
        format: "norm",
      });
      writes.push({
        msPath: `ch.${rightCh}.mix.sends.${sendsModeMsSendIndex}.pan`,
        value: right,
        format: "norm",
      });
    } else {
      for (const ch of slot.msChannels) {
        writes.push({
          msPath: `ch.${ch}.mix.sends.${sendsModeMsSendIndex}.pan`,
          value: knobPan,
          format: "norm",
        });
      }
    }
    return;
  }

  // Standard mode: for stereo-linked pairs, use the hybrid control to write BOTH channel pans.
  const knobPan = clamp01(Number(parsed.pan));
  if (isStereoLinkedPair) {
    track.pan = knobPan;
    queueConsole1TrackUpdate(track.trackId, { pan: knobPan });

    const { left, right, width, mid } = hybridStereoPanToDualMonoPans(knobPan);
    const [leftCh, rightCh] = slot.msChannels;

    if (LOG_JSON) {
      console.log("[hybrid-pan] standard", {
        trackId: track.trackId,
        msChannels: [leftCh, rightCh],
        knobPan,
        left,
        right,
        width,
        mid,
      });
    }

    writes.push({ msPath: `ch.${leftCh}.mix.pan`, value: left, format: "norm" });
    writes.push({ msPath: `ch.${rightCh}.mix.pan`, value: right, format: "norm" });
  } else {
    // Pan is written as normalized value (0..1) to avoid mixer-specific scale issues.
    writes.push({ msPath: `ch.${primaryChannel}.mix.pan`, value: knobPan, format: "norm" });
  }
}

/**
 * Handle `selected` updates from Console 1.
 * @param {any} parsed
 * @param {TrackLayoutSlot} slot
 * @param {number} primaryChannel
 * @param {Array<{msPath:string,value:any,format?:string}>} writes
 */
function handleMidiSelectedUpdate(parsed, slot, primaryChannel, writes) {
  if (parsed.selected === undefined) return;

  // Make sends-mode switching feel immediate: Bus selection enters sends mode, Main exits.
  if (parsed.selected) {
    if (slot.kind === "bus" && slot.msPrimary !== null)
      setSendsMode(slot.msPrimary - BUS_CHANNEL_START);
    else if (slot.kind === "main") setSendsMode(null);
  }
  writes.push({ msPath: `ch.${primaryChannel}.selected`, value: parsed.selected ? 1 : 0 });
}

/**
 * Handle send level/on updates (`send1..send6`, `send1On..send6On`) from Console 1.
 *
 * @param {any} parsed
 * @param {TrackLayoutSlot} slot
 * @param {TrackInfo} track
 * @param {Array<{msPath:string,value:any,format?:string}>} writes
 */
function handleMidiSendSlotsUpdate(parsed, slot, track, writes) {
  if (slot.kind !== "input") return;

  for (let i = 1; i <= NUMBER_OF_SENDS; i++) {
    const lvlKey = `send${i}`;
    const onKey = `send${i}On`;
    const msSendIndex = C1_SEND_TO_MS_SEND_INDEX[i - 1] ?? i - 1;

    if (parsed[lvlKey] !== undefined) {
      const nextSendLevelForConsole1 = coerceConsole1NumericString(parsed[lvlKey]);
      track[`send${i}`] = nextSendLevelForConsole1;
      queueConsole1TrackUpdate(track.trackId, { [lvlKey]: nextSendLevelForConsole1 });

      const v = normalizeConsole1LevelForMs(parsed[lvlKey]);
      if (v !== null) {
        // For stereo-linked pairs we must update BOTH underlying channels.
        for (const ch of slot.msChannels) {
          writes.push({ msPath: `ch.${ch}.mix.sends.${msSendIndex}.lvl`, value: v });
        }
      }
    }

    if (parsed[onKey] !== undefined) {
      // For stereo-linked pairs we must update BOTH underlying channels.
      for (const ch of slot.msChannels) {
        writes.push({
          msPath: `ch.${ch}.mix.sends.${msSendIndex}.on`,
          value: parsed[onKey] ? 1 : 0,
        });
      }
    }
  }
}

/**
 * Resolve all per-track context needed to handle a Console 1 track SysEx update.
 *
 * Returns null if the message can't be mapped to a valid layout slot.
 *
 * @param {any} parsed - Parsed SysEx JSON
 * @returns {{objectId:number,slot:TrackLayoutSlot,primaryChannel:number,track:TrackInfo}|null}
 */
function resolveMidiTrackContext(parsed) {
  if (!parsed || !parsed.trackId) return null;

  const objectId = getObjectIdForTrackId(parsed.trackId);
  if (objectId === undefined) return null;
  if (objectId < 0 || objectId >= trackLayout.length) return null;

  const slot = trackLayout[objectId];
  if (!slot || slot.kind === "empty") return null;

  const primaryChannel = slot.msPrimary;
  if (primaryChannel === null) return null;

  // Ensure we have a cached track for optimistic updates.
  const track = getOrCreateTrackInfo(objectId);

  return { objectId, slot, primaryChannel, track };
}

/**
 * Handles incoming MIDI messages, specifically SysEx JSON messages, and processes them
 * according to their type. Delegates control messages to `handleConsole1ControlJson`,
 * and for track-related messages, updates volume, mute, solo, pan, selection, and send slots.
 * Writes resulting changes to the MS Bridge via `queueMsWrite`.
 *
 * @param {number} deltaTime - The time elapsed since the last MIDI message, in milliseconds.
 * @param {Uint8Array} message - The raw MIDI message data.
 */
function handleMidiMessage(deltaTime, message) {
  const parsed = parseSysexJson(message);
  if (!parsed) return;

  if (LOG_JSON) console.log("Received SysEx JSON from MIDI input:", parsed);

  // Reuse the existing control handling.
  if (parsed.cmd || parsed.handshake || parsed.activeMeters) {
    handleConsole1ControlJson(parsed);
    return;
  }

  if (!parsed.trackId) return;

  const ctx = resolveMidiTrackContext(parsed);
  if (!ctx) return;
  const { slot, primaryChannel, track } = ctx;

  /** @type {Array<{msPath:string,value:any,format?:string}>} */
  const writes = [];
  handleMidiBusMainSendsModeFaderProxy(parsed, slot, track, writes);
  handleMidiVolumeUpdate(parsed, slot, track, writes);
  handleMidiMuteUpdate(parsed, slot, track, writes);
  handleMidiSoloUpdate(parsed, slot, track, writes);
  handleMidiPanUpdate(parsed, slot, track, primaryChannel, writes);
  handleMidiSelectedUpdate(parsed, slot, primaryChannel, writes);
  handleMidiSendSlotsUpdate(parsed, slot, track, writes);

  for (const w of writes) {
    queueMsWrite(w.msPath, w.value, w.format || "val");
  }
}

/**
 * Main MIDI input event handler. Only processes SysEx messages.
 */
function onConsole1MidiInputMessage(deltaTime, message) {
  if (LOG_JSON) console.log("Received MIDI message:", message);
  // Only handle SysEx messages (start with 0xF0)
  if (Array.isArray(message) && message[0] === SYSEX_START) {
    handleMidiMessage(deltaTime, message);
  }
}

// ####################################
// ############# SHUTDOWN #############
// ####################################
/**
 * Sets up handlers for graceful application shutdown on SIGINT (CTRL+C) and SIGTERM signals.
 *
 * When triggered, this function:
 * - Sends a RESET command to Console 1 via SysEx.
 * - Deactivates all known tracks on Console 1.
 * - Clears the cached track state.
 * - Unsubscribes from all Mixing Station data subscriptions.
 * - Disables the On-Screen Display (OSD).
 * - Closes MIDI input and output ports if open.
 * - Closes the Mixing Station WebSocket connection if open.
 * - Logs each shutdown step and exits the process.
 *
 * @function
 */
function setupShutdownHandler() {
  /**
   * Graceful shutdown handler for CTRL+C (SIGINT).
   * Closes MIDI ports and WebSocket, logs exit message.
   */
  const shutdown = (signal) => {
    console.log(`\nReceived ${signal}. Shutting down Softube-MS-Bridge...`);
    try {
      sendSysexToConsole1({ cmd: "RESET" }, true);
      console.log("Sent RESET command to Console 1.");

      // Deactivate all known tracks on Console 1, like Cubase `removeObject`.
      batchDeactivateAllTracks(true);
      console.log("Deactivated all tracks on Console 1.");

      // Clear cached state so we don't keep stale track data around during teardown.
      tracksByObjectId = {};
      console.log("Cleared track cache.");

      // Unsubscribe everything we subscribed.
      for (const key of Object.keys(wsDataSubscriptions)) {
        const sub = wsDataSubscriptions[key];
        if (sub && sub.path && sub.format) unsubscribeFromChannelData(sub.path, sub.format);
      }
      console.log("Unsubscribed from all Mixing Station data subscriptions.");
      disableOSD();
      console.log("Disabled OSD.");
      if (midiInput) {
        midiInput.closePort();
        console.log("Closed MIDI input port.");
      }
      if (midiOutput) {
        midiOutput.closePort();
        console.log("Closed MIDI output port.");
      }
      if (msWebSocket && msWebSocket.readyState === WebSocket.OPEN) {
        msWebSocket.close();
        console.log("Closed Mixing Station WebSocket.");
      }
    } catch (e) {
      console.error("Error during shutdown:", e.message);
    }
    process.exit(0);
  };

  process.on("SIGINT", () => shutdown("SIGINT (CTRL+C)"));
  process.on("SIGTERM", () => shutdown("SIGTERM"));
}

// ###################################
// ############ BOOTSTRAP ############
// ###################################
async function startBridgeProcess() {
  setupShutdownHandler();
  await waitForConsole1MidiPorts();
  midiInput.on("message", onConsole1MidiInputMessage);

  // Build initial track layout early so any Console 1 traffic can be mapped, then enter
  // standby — the process starts in standby and stays there until a `lifecycle:start`
  // stdin message (from the GUI's Start button or a CLI flag).
  rebuildTrackLayout();
  enterStandbyState();

  console.log("Softube-MS-Bridge running (standby). Press CTRL+C to exit.");
}

startBridgeProcess();
