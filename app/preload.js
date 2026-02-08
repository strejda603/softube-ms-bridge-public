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
