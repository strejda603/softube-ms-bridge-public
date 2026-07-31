const { contextBridge, ipcRenderer } = require("electron");

// Preload scripts run sandboxed (Electron's default since v20): `require()` only resolves a
// small built-in whitelist, NOT local project files like `./i18n` — attempting that silently
// aborts the whole preload script (nothing gets exposed at all, not just i18n). So the locale
// is loaded in the main process (see app/main.js's `i18n:get` handler, which has full Node
// access and the real `app.getLocale()`) and fetched here over synchronous IPC instead.
// `let`, not `const`: reassigned by the exposed `i18n.setLocale()` below when the user
// switches language at runtime, so `t()`/`getLocale()` reflect the new locale immediately
// without needing a full app restart.
let { locale: activeLocale, strings: i18nStrings, version: appVersion } = ipcRenderer.sendSync("i18n:get");

/**
 * Inlined rather than imported from app/i18n.js, for the same sandboxed-`require()` reason
 * above — this mirrors `createTranslator()` there exactly; keep the two in sync if either
 * changes.
 * @param {string} key
 * @param {Record<string, string|number>} [vars]
 * @returns {string}
 */
function t(key, vars) {
  const template = Object.prototype.hasOwnProperty.call(i18nStrings, key) ? i18nStrings[key] : key;
  if (!vars) return template;
  return template.replace(/\{(\w+)\}/g, (match, varName) =>
    Object.prototype.hasOwnProperty.call(vars, varName) ? String(vars[varName]) : match,
  );
}

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
   * @param {(status: {mixingStation:boolean, console1Osd:boolean}) => void} cb
   * @example
   * bridge.onStatusUpdate((status) => { console.log(status.mixingStation); });
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

contextBridge.exposeInMainWorld("i18n", {
  /**
   * Active locale code (e.g. "en"). A function, not a plain property — contextBridge
   * deep-copies plain values at expose time, so a static property would go stale after
   * `setLocale()` reassigns `activeLocale`; a function reads it fresh every call.
   * @returns {string}
   */
  getLocale: () => activeLocale,
  /**
   * Translate a key from app/locales/<locale>.json, with optional `{placeholder}` substitution.
   * Unknown keys resolve to the key itself rather than throwing.
   * @param {string} key
   * @param {Record<string, string|number>} [vars]
   * @returns {string}
   * @example
   * i18n.t("status.ariaLabel", { name: "Mixing Station", state: "running" });
   */
  t: (key, vars) => t(key, vars),
  /** App version (from package.json, via Electron's app.getVersion()). Static, never changes. */
  version: appVersion,
  /**
   * List installed locales as `{code, name}` pairs for populating a language selector.
   * @returns {Promise<Array<{code: string, name: string}>>}
   */
  listLocales: () => ipcRenderer.invoke("i18n:listLocales"),
  /**
   * Switch the active UI language at runtime and persist the choice. Updates this
   * preload's own `t()`/`getLocale()` immediately; the caller still needs to re-apply
   * translations to the already-rendered DOM (see renderer.js's `applyI18n()`).
   * @param {string} code
   * @returns {Promise<{locale: string}>}
   * @example
   * await i18n.setLocale("cs");
   * applyI18n();
   */
  setLocale: async (code) => {
    const result = await ipcRenderer.invoke("i18n:setLocale", code);
    activeLocale = result.locale;
    i18nStrings = result.strings;
    return { locale: activeLocale };
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

contextBridge.exposeInMainWorld("updates", {
  /** @returns {Promise<{available: boolean, latestVersion?: string, downloadUrl?: string, releaseUrl?: string, error?: boolean}>} */
  check: () => ipcRenderer.invoke("update:check"),
  /** @param {string} url */
  openDownload: (url) => ipcRenderer.invoke("update:openDownload", url),
});
