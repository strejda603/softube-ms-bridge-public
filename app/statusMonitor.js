/**
 * GUI topbar status indicators: MIDI-device presence and running-app detection.
 *
 * Runs entirely in the Electron main process, independent of the bridge child
 * process (`index.js`). MIDI port enumeration only *lists* ports (never opens
 * one), so it can't conflict with the bridge process or any other app holding
 * those ports open.
 */

const midi = require("@julusian/midi");
const { exec } = require("child_process");

/**
 * @typedef {object} StatusSnapshot
 * @property {boolean} ipad
 * @property {boolean} spdSxPro
 * @property {boolean} midiMaestro
 * @property {boolean} bomeMtp
 * @property {boolean} mixingStation
 * @property {boolean} console1Osd
 * @property {boolean} abletonLive
 */

/**
 * Pure: compute the 7 indicator booleans from already-gathered inputs.
 *
 * Match rules (all substring, case-sensitive, mirroring the existing
 * `preferredNames.some((pn) => name.includes(pn))` convention in `index.js`):
 * - ipad: a MIDI input name AND a MIDI output name both contain "iPad"
 * - spdSxPro: a MIDI output name contains "SPD-SX PRO"
 * - midiMaestro: a MIDI input name contains "MIDI Maestro" (this substring
 *   also matches the "MIDI Maestro Bluetooth" variant, so no separate check
 *   is needed for it)
 * - bomeMtp / mixingStation / console1Osd / abletonLive: the `ps` output text
 *   contains the app's name
 *
 * @param {{midiInputNames?: string[], midiOutputNames?: string[], psOutput?: string}} args
 * @returns {StatusSnapshot}
 * @example
 * computeStatus({ midiInputNames: ["iPad"], midiOutputNames: ["iPad"], psOutput: "" }).ipad; // true
 */
function computeStatus({ midiInputNames = [], midiOutputNames = [], psOutput = "" } = {}) {
  const inputHas = (needle) => midiInputNames.some((name) => name.includes(needle));
  const outputHas = (needle) => midiOutputNames.some((name) => name.includes(needle));
  const psHas = (needle) => psOutput.includes(needle);

  return {
    ipad: inputHas("iPad") && outputHas("iPad"),
    spdSxPro: outputHas("SPD-SX PRO"),
    midiMaestro: inputHas("MIDI Maestro"),
    bomeMtp: psHas("Bome MIDI Translator Pro"),
    mixingStation: psHas("Mixing Station"),
    console1Osd: psHas("Softube On-Screen Display"),
    abletonLive: psHas("Ableton Live 12 Suite"),
  };
}

/**
 * Impure: enumerate current MIDI input/output port names.
 * Never opens a port — only lists them. May throw if the MIDI subsystem is
 * unavailable; callers should catch.
 *
 * @returns {{inputNames: string[], outputNames: string[]}}
 */
function gatherMidiPortNames() {
  const input = new midi.Input();
  const output = new midi.Output();

  const inputNames = [];
  const inCount = input.getPortCount();
  for (let i = 0; i < inCount; i++) inputNames.push(input.getPortName(i));

  const outputNames = [];
  const outCount = output.getPortCount();
  for (let i = 0; i < outCount; i++) outputNames.push(output.getPortName(i));

  return { inputNames, outputNames };
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
 * Start polling MIDI ports + running processes every `intervalMs`, sending a
 * `status:update` IPC message to `win` only when the computed status changes.
 *
 * @param {import('electron').BrowserWindow|null} win
 * @param {number} [intervalMs=2000]
 * @param {(status: StatusSnapshot) => void} [onChange] - optional, called with the same
 *   status object whenever it changes (in addition to the `win` IPC send) — e.g. to also
 *   forward it to the bridge child process.
 * @returns {() => void} stop function; call to clear the interval
 */
function startStatusMonitor(win, intervalMs = 2000, onChange) {
  let lastSentKey = null;
  let midiWarned = false;
  let psWarned = false;

  async function tick() {
    let midiInputNames = [];
    let midiOutputNames = [];
    try {
      ({ inputNames: midiInputNames, outputNames: midiOutputNames } = gatherMidiPortNames());
    } catch (e) {
      if (!midiWarned) {
        console.warn("[status] MIDI port enumeration failed:", e?.message || e);
        midiWarned = true;
      }
    }

    let psOutput = "";
    try {
      psOutput = await gatherPsOutput();
    } catch (e) {
      if (!psWarned) {
        console.warn("[status] ps failed:", e?.message || e);
        psWarned = true;
      }
    }

    const status = computeStatus({ midiInputNames, midiOutputNames, psOutput });
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

module.exports = { computeStatus, gatherMidiPortNames, gatherPsOutput, startStatusMonitor };
