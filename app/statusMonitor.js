/**
 * GUI topbar status indicators: running-app detection for Mixing Station and the
 * Softube Console 1 On-Screen Display.
 *
 * Runs entirely in the Electron main process, independent of the bridge child
 * process (`index.js`).
 */

const { exec } = require("child_process");

/**
 * @typedef {object} StatusSnapshot
 * @property {boolean} mixingStation
 * @property {boolean} console1Osd
 */

/**
 * Pure: compute the 2 indicator booleans from already-gathered input.
 *
 * Match rules (substring, case-sensitive): the `ps` output text contains the app's name.
 *
 * @param {{psOutput?: string}} args
 * @returns {StatusSnapshot}
 * @example
 * computeStatus({ psOutput: "Mixing Station" }).mixingStation; // true
 */
function computeStatus({ psOutput = "" } = {}) {
  const psHas = (needle) => psOutput.includes(needle);

  return {
    mixingStation: psHas("Mixing Station"),
    console1Osd: psHas("Softube On-Screen Display"),
  };
}

/**
 * Impure: run `ps -Ao args=` and resolve with its stdout text (empty string on failure).
 * Never rejects.
 *
 * @returns {Promise<string>}
 */
function gatherPsOutput() {
  return new Promise((resolve) => {
    exec("ps -Ao args=", { maxBuffer: 10 * 1024 * 1024 }, (err, stdout) => {
      if (err) {
        resolve("");
        return;
      }
      resolve(stdout || "");
    });
  });
}

/**
 * Start polling running processes every `intervalMs`, sending a `status:update` IPC
 * message to `win` only when the computed status changes.
 *
 * @param {import('electron').BrowserWindow|null} win
 * @param {number} [intervalMs=2000]
 * @param {(status: StatusSnapshot) => void} [onChange] - optional, called with the same
 *   status object whenever it changes (in addition to the `win` IPC send).
 * @returns {() => void} stop function; call to clear the interval
 */
function startStatusMonitor(win, intervalMs = 2000, onChange) {
  let lastSentKey = null;
  let psWarned = false;

  async function tick() {
    let psOutput = "";
    try {
      psOutput = await gatherPsOutput();
    } catch (e) {
      if (!psWarned) {
        console.warn("[status] ps failed:", e?.message || e);
        psWarned = true;
      }
    }

    const status = computeStatus({ psOutput });
    const key = JSON.stringify(status);
    if (key === lastSentKey) return;
    lastSentKey = key;

    if (win && !win.isDestroyed()) win.webContents.send("status:update", status);
    if (typeof onChange === "function") onChange(status);
  }

  tick();
  const timer = setInterval(tick, intervalMs);
  return () => clearInterval(timer);
}

module.exports = { computeStatus, gatherPsOutput, startStatusMonitor };
