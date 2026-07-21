const { contextBridge, ipcRenderer } = require("electron");

/**
 * @typedef {object} BridgeConfig
 * @property {string} mixingStationWsUrl
 * @property {boolean} logJson
 * @property {number} inputCount
 * @property {number} busCount
 * @property {(number|number[])[]} inputTrackOrder
 * @property {(number|number[])[]} busTrackOrder
 * @property {number[]} c1SendToMsBusNumber
 * @property {number|undefined} [metering2IntervalMs]
 * @property {number|undefined} [console1MainColor]
 * @property {number|undefined} [console1BusColor]
 */

/**
 * @typedef {object} BridgeStatus
 * @property {boolean} running
 */

/**
 * @typedef {object} PresetMeta
 * @property {string} name
 * @property {string} savedAt
 */

/**
 * @typedef {object} PresetPayload
 * @property {PresetMeta} meta
 * @property {BridgeConfig} config
 */

/**
 * Renderer-safe API exposed to the UI.
 *
 * This is the only supported way for renderer code to talk to Electron.
 */

/**
 * @typedef {object} ParsedCliArgs
 * @property {boolean} start
 * @property {boolean} stop
 * @property {string|null} preset
 * @property {string|null} ws
 * @property {number|null} interval
 * @property {boolean} log
 * @property {boolean} verbose
 * @property {string[]} warnings
 */

/**
 * Buffer for a `cli:apply` message that arrives before the renderer has
 * called `bridge.onCliArgs(...)` (e.g. a fast `second-instance` forward
 * racing the page's own startup). Only the most recent message is kept.
 * @type {ParsedCliArgs|null}
 */
let pendingCliArgs = null;
/** @type {((args: ParsedCliArgs) => void)|null} */
let cliArgsCallback = null;

ipcRenderer.on("cli:apply", (_evt, args) => {
  if (cliArgsCallback) {
    cliArgsCallback(args);
  } else {
    pendingCliArgs = args;
  }
});

/**
 * Buffer for a `status:update` message that arrives before the renderer has
 * called `bridge.onStatusUpdate(...)`. Same one-slot, last-message-wins
 * approach as `pendingCliArgs` above.
 * @type {object|null}
 */
let pendingStatusUpdate = null;
/** @type {((status: object) => void)|null} */
let statusUpdateCallback = null;

ipcRenderer.on("status:update", (_evt, status) => {
  if (statusUpdateCallback) {
    statusUpdateCallback(status);
  } else {
    pendingStatusUpdate = status;
  }
});

contextBridge.exposeInMainWorld("bridge", {
  /** @param {BridgeConfig} config */
  start: (config) => ipcRenderer.invoke("bridge:start", config),
  stop: () => ipcRenderer.invoke("bridge:stop"),
  /** @returns {Promise<BridgeStatus>} */
  status: () => ipcRenderer.invoke("bridge:status"),
  /** @param {BridgeConfig} config */
  applyConfig: (config) => ipcRenderer.invoke("bridge:applyConfig", config),

  /**
   * Subscribe to bridge log lines.
   *
   * @param {(line:string)=>void} cb
   * @returns {()=>void} unsubscribe
   * @example
   * const off = bridge.onLog((line) => console.log(line));
   * // later: off();
   */
  onLog: (cb) => {
    const listener = (_evt, line) => cb(line);
    ipcRenderer.on("bridge:log", listener);
    return () => ipcRenderer.removeListener("bridge:log", listener);
  },

  /**
   * Subscribe to hardware Start/Stop triggers from the Console 1 Fader's status bank.
   *
   * Unlike `onCliArgs`/`onStatusUpdate`, this is deliberately NOT the one-slot-buffered
   * pattern — a hardware press can happen repeatedly throughout a session, not just once at
   * launch, so it follows `onLog`'s always-on multi-listener shape instead.
   *
   * @param {(trigger: {type: "start"|"stop"}) => void} cb
   * @returns {()=>void} unsubscribe
   * @example
   * const off = bridge.onHardwareTrigger((t) => { if (t.type === "start") startBridgeFromForm(); });
   */
  onHardwareTrigger: (cb) => {
    const listener = (_evt, trigger) => cb(trigger);
    ipcRenderer.on("bridge:hardwareTrigger", listener);
    return () => ipcRenderer.removeListener("bridge:hardwareTrigger", listener);
  },

  /**
   * Subscribe to parsed launch/forwarded CLI args. Delivers immediately if
   * one arrived before this was called (see `pendingCliArgs` above).
   *
   * @param {(args: ParsedCliArgs) => void} cb
   * @example
   * bridge.onCliArgs((args) => { if (args.start) startBridgeFromForm(); });
   */
  onCliArgs: (cb) => {
    cliArgsCallback = cb;
    if (pendingCliArgs) {
      const args = pendingCliArgs;
      pendingCliArgs = null;
      cb(args);
    }
  },

  /**
   * Subscribe to topbar status indicator updates. Delivers immediately if one
   * arrived before this was called (same buffering pattern as `onCliArgs`).
   *
   * @param {(status: {ipad:boolean, spdSxPro:boolean, midiMaestro:boolean, bomeMtp:boolean, mixingStation:boolean, console1Osd:boolean, abletonLive:boolean}) => void} cb
   * @example
   * bridge.onStatusUpdate((status) => { console.log(status.ipad); });
   */
  onStatusUpdate: (cb) => {
    statusUpdateCallback = cb;
    if (pendingStatusUpdate) {
      const status = pendingStatusUpdate;
      pendingStatusUpdate = null;
      cb(status);
    }
  },
});

contextBridge.exposeInMainWorld("presets", {
  /** @returns {Promise<Array<{id:string,name:string,updatedAt:number}>>} */
  list: () => ipcRenderer.invoke("presets:list"),
  /** @param {PresetPayload} preset */
  save: (preset) => ipcRenderer.invoke("presets:save", preset),
  /** @param {string} id */
  load: (id) => ipcRenderer.invoke("presets:load", id),
  /** @param {string} id */
  delete: (id) => ipcRenderer.invoke("presets:delete", id),
  /** @param {string} id */
  export: (id) => ipcRenderer.invoke("presets:export", id),
  import: () => ipcRenderer.invoke("presets:import"),
  openFolder: () => ipcRenderer.invoke("presets:openFolder"),
});
