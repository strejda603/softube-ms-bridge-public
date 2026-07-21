const { app, BrowserWindow, ipcMain, dialog, shell, nativeImage } = require("electron");
const path = require("path");
const fs = require("fs");
const { spawn } = require("child_process");
const { parseCliArgs, getUserArgv } = require("./cliArgs");
const { startStatusMonitor } = require("./statusMonitor");

// CLI flags
// Usage: `npm run gui -- --verbose`
// - Prints bridge logs to the parent Terminal
// - Forces LOG_JSON=1 for the bridge process
// - Hides bridge log lines from the GUI "Console Output" (renderer still receives lines for status)
const VERBOSE_TERMINAL = process.argv.includes("--verbose");

// Only one GUI instance may run at a time. A second launch (e.g. from a shell
// script running `open -a "Softube Console 1 MS Bridge" --args --stop`) forwards
// its CLI args to this instance via the "second-instance" event below, instead
// of opening a duplicate window.
if (!app.requestSingleInstanceLock()) {
  app.quit();
  return;
}

/** Parsed once at process start; sent to the renderer after the window loads. */
const initialCliArgs = parseCliArgs(getUserArgv(process.argv, app.isPackaged));
for (const warning of initialCliArgs.warnings) {
  console.warn(`[cli] ${warning}`);
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
 * @typedef {{type:"config:apply", config: BridgeConfig}} BridgeControlMessage
 */

// Ensure presets/config are stored under a clearly named app folder (not "Electron" in dev).
const USER_DATA_DIR_NAME = "Softube Console 1 MS Bridge";
const legacyUserDataPath = app.getPath("userData");
try {
  app.setPath("userData", path.join(app.getPath("appData"), USER_DATA_DIR_NAME));
} catch {
  // ignore
}
const userDataPath = app.getPath("userData");

/** @type {import('child_process').ChildProcessWithoutNullStreams | null} */
let bridgeProcess = null;
let isQuitting = false;
/** @type {import('electron').BrowserWindow | null} */
let mainWindow = null;
/** @type {(() => void) | null} */
let stopStatusMonitor = null;
/**
 * The bridge process is spawned once and stays alive until the app quits; this tracks
 * which of its two lifecycle states (see `index.js`) it's currently in.
 * @type {"standby"|"running"}
 */
let bridgeLifecycleState = "standby";

/**
 * Prefix marking a structured event line on the bridge process's stdout, as opposed to a
 * plain human-readable log line. See `index.js`'s `@@BRIDGE_EVENT@@` convention.
 */
const BRIDGE_EVENT_PREFIX = "@@BRIDGE_EVENT@@";

/**
 * Send parsed CLI args to the renderer over the `cli:apply` channel.
 * @param {import('electron').BrowserWindow|null} win
 * @param {ReturnType<typeof parseCliArgs>} args
 */
function sendCliArgsToRenderer(win, args) {
  if (!win || win.isDestroyed()) return;
  win.webContents.send("cli:apply", args);
}

/** Forward a bridge log line to the renderer's Console Output panel. */
function sendBridgeLogLine(line) {
  if (!mainWindow || mainWindow.isDestroyed()) return;
  mainWindow.webContents.send("bridge:log", line);
}

/**
 * Parse and dispatch a structured `@@BRIDGE_EVENT@@`-prefixed stdout line from the bridge
 * process (see `index.js`). Currently only `hardware:start`/`hardware:stop` (the physical
 * Start/Stop slot on the Console 1 Fader's status bank) are recognized.
 * @param {string} jsonText - everything after the `@@BRIDGE_EVENT@@` prefix
 */
function handleBridgeEventLine(jsonText) {
  let event;
  try {
    event = JSON.parse(jsonText);
  } catch {
    return;
  }
  if (!event || typeof event.type !== "string") return;

  if (event.type === "hardware:start" || event.type === "hardware:stop") {
    if (!mainWindow || mainWindow.isDestroyed()) return;
    const triggerType = event.type === "hardware:start" ? "start" : "stop";
    mainWindow.webContents.send("bridge:hardwareTrigger", { type: triggerType });
  }
}

/**
 * Send a newline-delimited JSON control message to the bridge process.
 *
 * The bridge listens on stdin (see `installRuntimeControlChannel()` in the bridge entry).
 * @param {BridgeControlMessage} obj
 */
function sendBridgeControlMessage(obj) {
  if (!bridgeProcess) throw new Error("Bridge is not running");
  const stdin = bridgeProcess.stdin;
  if (!stdin || stdin.destroyed) throw new Error("Bridge stdin is not available");
  stdin.write(JSON.stringify(obj) + "\n");
}

function getUserDataFile(relPath) {
  return path.join(app.getPath("userData"), relPath);
}

function ensureDir(dirPath) {
  fs.mkdirSync(dirPath, { recursive: true });
}

function migrateLegacyPresetsOnce() {
  // Only migrate if legacy path differs and legacy presets exist.
  if (!legacyUserDataPath || legacyUserDataPath === userDataPath) return;

  const legacyPresetsDir = path.join(legacyUserDataPath, "presets");
  if (!fs.existsSync(legacyPresetsDir)) return;

  const newPresetsDir = getUserDataFile("presets");
  ensureDir(newPresetsDir);

  const legacyFiles = fs
    .readdirSync(legacyPresetsDir, { withFileTypes: true })
    .filter((e) => e.isFile() && e.name.endsWith(".json"))
    .map((e) => e.name);

  if (legacyFiles.length === 0) return;

  // If the new folder is empty, copy everything. If not empty, only copy missing names.
  const existing = new Set(
    fs
      .readdirSync(newPresetsDir, { withFileTypes: true })
      .filter((e) => e.isFile() && e.name.endsWith(".json"))
      .map((e) => e.name)
  );

  for (const name of legacyFiles) {
    if (existing.has(name)) continue;
    try {
      fs.copyFileSync(path.join(legacyPresetsDir, name), path.join(newPresetsDir, name));
    } catch {
      // ignore copy errors
    }
  }
}

function writeConfigFile(config) {
  const configPath = getUserDataFile("bridge-config.json");
  ensureDir(path.dirname(configPath));
  fs.writeFileSync(configPath, JSON.stringify(config, null, 2), "utf8");
  return configPath;
}

function listPresets() {
  const presetsDir = getUserDataFile("presets");
  ensureDir(presetsDir);
  const entries = fs
    .readdirSync(presetsDir, { withFileTypes: true })
    .filter((e) => e.isFile() && e.name.endsWith(".json"))
    .map((e) => e.name)
    .sort((a, b) => a.localeCompare(b));

  return entries.map((fileName) => {
    const fullPath = path.join(presetsDir, fileName);
    try {
      const raw = fs.readFileSync(fullPath, "utf8");
      const data = JSON.parse(raw);
      return {
        id: fileName,
        name: data?.meta?.name || fileName.replace(/\.json$/i, ""),
        updatedAt: fs.statSync(fullPath).mtimeMs,
      };
    } catch {
      return { id: fileName, name: fileName.replace(/\.json$/i, ""), updatedAt: 0 };
    }
  });
}

function savePreset(preset) {
  const presetsDir = getUserDataFile("presets");
  ensureDir(presetsDir);

  const safeName = String(preset?.meta?.name || "Preset")
    .trim()
    .replace(/[^a-z0-9._ -]/gi, "_")
    .replace(/\s+/g, " ");

  const fileName = `${safeName || "Preset"}.json`;
  const fullPath = path.join(presetsDir, fileName);
  fs.writeFileSync(fullPath, JSON.stringify(preset, null, 2), "utf8");
  return fileName;
}

function loadPreset(presetId) {
  const presetsDir = getUserDataFile("presets");
  const fullPath = path.join(presetsDir, presetId);
  const raw = fs.readFileSync(fullPath, "utf8");
  return JSON.parse(raw);
}

function deletePreset(presetId) {
  const presetsDir = getUserDataFile("presets");
  const fullPath = path.join(presetsDir, presetId);
  fs.unlinkSync(fullPath);
}

function getPresetsDir() {
  const presetsDir = getUserDataFile("presets");
  ensureDir(presetsDir);
  return presetsDir;
}

function getAppIconImage() {
  const iconPath = path.join(__dirname, "renderer", "assets", "icon.png");
  try {
    if (!fs.existsSync(iconPath)) return null;
    const buf = fs.readFileSync(iconPath);
    const img = nativeImage.createFromBuffer(buf);
    return img.isEmpty() ? null : img;
  } catch {
    return null;
  }
}

function createWindow() {
  const iconImage = getAppIconImage();
  // Best-effort: on macOS set dock icon too (dev `npm run gui` won't otherwise show it).
  try {
    if (process.platform === "darwin" && iconImage && app.dock) {
      app.dock.setIcon(iconImage);
    }
  } catch {
    // ignore
  }

  const win = new BrowserWindow({
    width: 1200,
    height: 850,
    backgroundColor: "#0b0f14",
    icon: iconImage || undefined,
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      preload: path.join(__dirname, "preload.js"),
    },
  });

  mainWindow = win;

  win.loadFile(path.join(__dirname, "renderer", "index.html"),
    VERBOSE_TERMINAL ? { query: { verbose: "1" } } : undefined
  );

  win.webContents.once("did-finish-load", () => {
    sendCliArgsToRenderer(win, initialCliArgs);
    // Starting the monitor here (rather than immediately after createWindow())
    // guarantees the renderer/preload has registered its `status:update`
    // listener before the first snapshot is sent — otherwise a device/app
    // already present at launch could send its first (only, since the
    // poller only re-sends on change) update before anyone is listening.
    stopStatusMonitor = startStatusMonitor(win);
    // Same reasoning applies to the bridge process: spawn it once the renderer
    // has loaded, so its first log lines / hardware-trigger events have a
    // listener already registered.
    spawnBridgeProcess();
  });

  win.on("closed", () => {
    mainWindow = null;
  });

  // Ensure the bridge is stopped before the window closes.
  win.on("close", (e) => {
    if (isQuitting) return;
    if (!bridgeProcess) return;

    e.preventDefault();
    stopBridgeGraceful({ reason: "window-close" })
      .catch(() => {
        // ignore
      })
      .finally(() => {
        try {
          win.destroy();
        } catch {
          // ignore
        }
      });
  });
}

function stopBridgeGraceful({ reason } = {}) {
  if (!bridgeProcess) return Promise.resolve();

  const proc = bridgeProcess;
  bridgeProcess = null;

  return new Promise((resolve) => {
    let settled = false;

    const done = () => {
      if (settled) return;
      settled = true;
      resolve();
    };

    proc.once("exit", done);

    // Let the bridge run its SIGINT handler (it sends Console 1 RESET etc.).
    try {
      proc.kill("SIGINT");
    } catch {
      done();
      return;
    }

    // Fallback: force kill after a short timeout.
    setTimeout(() => {
      if (settled) return;
      try {
        proc.kill("SIGKILL");
      } catch {
        // ignore
      }
    }, 1200);
  });
}

/**
 * Spawn the bridge as a child Node process, into its `standby` lifecycle state (Console 1
 * Fader MIDI held, no Mixing Station connection yet). No-ops if already spawned — the
 * process is spawned once and stays alive until the app quits; `bridge:start`/`bridge:stop`
 * transition its lifecycle state over the existing stdin control channel rather than
 * spawning/killing it.
 *
 * In dev: runs the project-root `index.js`.
 * In packaged: runs `index.js` from inside the app bundle (so prod deps resolve correctly).
 */
function spawnBridgeProcess() {
  if (bridgeProcess) return;

  const spawnCommand = process.execPath;
  const spawnEnv = {
    ...process.env,
    ELECTRON_RUN_AS_NODE: "1",
  };

  // Verbose mode: force bridge JSON logging regardless of GUI checkbox.
  if (VERBOSE_TERMINAL) spawnEnv.LOG_JSON = "1";

  // Packaged: run the bridge script from inside app.asar so Node's module resolution
  // finds production dependencies (including native modules unpacked by electron-builder).
  const bridgeEntry = app.isPackaged
    ? path.join(app.getAppPath(), "index.js")
    : path.join(process.cwd(), "index.js");

  // Use project root as cwd in dev, Resources in packaged.
  const spawnCwd = app.isPackaged ? process.resourcesPath : process.cwd();

  bridgeProcess = spawn(spawnCommand, [bridgeEntry], {
    cwd: spawnCwd,
    env: spawnEnv,
  });
  bridgeLifecycleState = "standby";

  const onData = (buf) => {
    const text = buf.toString("utf8");
    for (const line of text.split(/\r?\n/)) {
      if (!line) continue;

      if (line.startsWith(BRIDGE_EVENT_PREFIX)) {
        handleBridgeEventLine(line.slice(BRIDGE_EVENT_PREFIX.length));
        continue;
      }

      if (VERBOSE_TERMINAL) {
        try {
          process.stdout.write(line + "\n");
        } catch {
          // ignore
        }
      }

      sendBridgeLogLine(line);
    }
  };

  bridgeProcess.stdout.on("data", onData);
  bridgeProcess.stderr.on("data", onData);

  bridgeProcess.on("exit", (code, signal) => {
    sendBridgeLogLine(`Bridge exited (code=${code ?? "?"}, signal=${signal ?? "?"})`);
    bridgeProcess = null;
    bridgeLifecycleState = "standby";
  });
}

app.on("second-instance", (_event, argv, _workingDirectory) => {
  // Note: unlike the initial-launch argv, Electron's forwarded second-instance
  // argv can include injected Chromium/dev-mode switches we don't control
  // (e.g. --allow-file-access-from-files), so we don't warn on unrecognized
  // tokens here — recognized flags (--start/--stop/etc.) still work correctly
  // regardless of that noise. Clear `warnings` too, not just the console.warn
  // above: the renderer replays `args.warnings` into the GUI's own log panel,
  // which would otherwise leak the same noise back in through that path.
  const args = parseCliArgs(getUserArgv(argv, app.isPackaged));
  args.warnings = [];

  if (!mainWindow) return;
  if (mainWindow.isMinimized()) mainWindow.restore();
  mainWindow.show();
  mainWindow.focus();
  sendCliArgsToRenderer(mainWindow, args);
});

app.whenReady().then(() => {
  migrateLegacyPresetsOnce();
  createWindow();

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on("window-all-closed", () => {
  // Quit even on macOS; but ensure bridge has been stopped.
  app.quit();
});

app.on("before-quit", () => {
  if (stopStatusMonitor) {
    stopStatusMonitor();
    stopStatusMonitor = null;
  }
});

app.on("before-quit", (e) => {
  if (isQuitting) return;
  if (!bridgeProcess) return;

  // Stop the bridge before exiting the app.
  e.preventDefault();
  isQuitting = true;
  stopBridgeGraceful({ reason: "app-quit" })
    .catch(() => {
      // ignore
    })
    .finally(() => {
      app.quit();
    });
});

ipcMain.handle("bridge:start", async (_evt, config) => {
  const effectiveConfig = VERBOSE_TERMINAL ? { ...(config || {}), logJson: true } : config;

  // Defensive fallback: the process is normally already spawned (into standby) by the time
  // Start can be clicked, but re-spawn if it somehow isn't running (e.g. it crashed).
  spawnBridgeProcess();

  writeConfigFile(effectiveConfig);
  sendBridgeControlMessage({ type: "lifecycle:start", config: effectiveConfig });
  bridgeLifecycleState = "running";

  return { ok: true };
});

ipcMain.handle("bridge:stop", async () => {
  if (!bridgeProcess) return { ok: true };

  sendBridgeControlMessage({ type: "lifecycle:stop" });
  bridgeLifecycleState = "standby";

  return { ok: true };
});

ipcMain.handle("bridge:applyConfig", async (_evt, config) => {
  const effectiveConfig = VERBOSE_TERMINAL ? { ...(config || {}), logJson: true } : config;

  // Always persist config so a re-spawned bridge (e.g. after a crash) will come back with
  // the latest settings.
  writeConfigFile(effectiveConfig);

  spawnBridgeProcess();

  if (bridgeLifecycleState !== "running") {
    // Standby doesn't use the Mixing Station config at all — nothing more to do until Start.
    return { ok: true, started: false };
  }

  sendBridgeControlMessage({
    type: "config:apply",
    config: effectiveConfig,
  });

  return { ok: true, started: false };
});

ipcMain.handle("bridge:status", async () => {
  return { running: bridgeLifecycleState === "running" };
});

ipcMain.handle("presets:list", async () => {
  return listPresets();
});

ipcMain.handle("presets:save", async (_evt, preset) => {
  const id = savePreset(preset);
  return { ok: true, id };
});

ipcMain.handle("presets:load", async (_evt, presetId) => {
  return loadPreset(presetId);
});

ipcMain.handle("presets:delete", async (_evt, presetId) => {
  deletePreset(presetId);
  return { ok: true };
});

ipcMain.handle("presets:export", async (evt, presetId) => {
  const win = BrowserWindow.fromWebContents(evt.sender);
  if (!win) throw new Error("No window");
  const preset = loadPreset(presetId);

  const res = await dialog.showSaveDialog(win, {
    title: "Export Preset",
    defaultPath: `${preset?.meta?.name || "preset"}.json`,
    filters: [{ name: "JSON", extensions: ["json"] }],
  });
  if (res.canceled || !res.filePath) return { ok: false, canceled: true };
  fs.writeFileSync(res.filePath, JSON.stringify(preset, null, 2), "utf8");
  return { ok: true };
});

ipcMain.handle("presets:import", async (evt) => {
  const win = BrowserWindow.fromWebContents(evt.sender);
  if (!win) throw new Error("No window");
  const res = await dialog.showOpenDialog(win, {
    title: "Import Preset",
    properties: ["openFile"],
    filters: [{ name: "JSON", extensions: ["json"] }],
  });
  if (res.canceled || !res.filePaths?.[0]) return { ok: false, canceled: true };

  const raw = fs.readFileSync(res.filePaths[0], "utf8");
  const preset = JSON.parse(raw);
  const id = savePreset(preset);
  return { ok: true, id };
});

ipcMain.handle("presets:openFolder", async () => {
  const presetsDir = getPresetsDir();
  await shell.openPath(presetsDir);
  return { ok: true };
});
