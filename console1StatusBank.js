/**
 * Pure helpers for the Console 1 Fader's status/Start bank (bank 0).
 *
 * Kept separate from `index.js` (which self-executes the bridge process on load) so this
 * logic can be unit-tested with the plain Node test runner without spinning up real MIDI/WS
 * connections — same rationale as `app/cliArgs.js` and `app/statusMonitor.js`.
 */

/**
 * @typedef {object} StatusBankSlot
 * @property {number} objectId
 * @property {"status"|"empty"|"start"} kind
 * @property {number[]} msChannels - Always empty; these slots have no Mixing Station channel.
 * @property {number|null} msPrimary - Always null, for the same reason.
 * @property {string} [statusKey] - Only present on `"status"` slots; matches
 *   `app/statusMonitor.js`'s `computeStatus()` field names (e.g. "ipad").
 * @property {string} [statusLabel] - Only present on `"status"` slots; the display name.
 */

/**
 * The 7 status indicators, in the same order as the GUI topbar (Feature A) and using the
 * same field names as `app/statusMonitor.js`'s `computeStatus()` return value.
 * @type {{key:string, label:string}[]}
 */
const STATUS_BANK_INDICATORS = [
  { key: "ipad", label: "iPad" },
  { key: "spdSxPro", label: "SPD-SX PRO" },
  { key: "midiMaestro", label: "MIDI Maestro" },
  { key: "bomeMtp", label: "Bome MIDI Translator Pro" },
  { key: "mixingStation", label: "Mixing Station" },
  { key: "console1Osd", label: "Console 1 On-Screen Display" },
  { key: "abletonLive", label: "Ableton Live 12 Suite" },
];

/** Total slot count of the fixed status/Start bank (bank 0). */
const STATUS_BANK_SIZE = 10;

/** 0-based objectId of the Start slot — the last slot of bank 0. */
const START_SLOT_OBJECT_ID = STATUS_BANK_SIZE - 1;

/**
 * Build the fixed status/Start bank: 7 status slots (one per indicator, in order), 2 empty
 * spacer slots, then the Start slot as the 10th/last slot.
 *
 * @returns {StatusBankSlot[]}
 * @example
 * buildStatusBankSlots()[9].kind; // "start"
 */
function buildStatusBankSlots() {
  /** @type {StatusBankSlot[]} */
  const slots = [];

  for (const indicator of STATUS_BANK_INDICATORS) {
    slots.push({
      objectId: slots.length,
      kind: "status",
      msChannels: [],
      msPrimary: null,
      statusKey: indicator.key,
      statusLabel: indicator.label,
    });
  }

  while (slots.length < STATUS_BANK_SIZE - 1) {
    slots.push({ objectId: slots.length, kind: "empty", msChannels: [], msPrimary: null });
  }

  slots.push({ objectId: slots.length, kind: "start", msChannels: [], msPrimary: null });

  return slots;
}

/**
 * Start/Stop toggle display for the Start slot, driven by the bridge's current lifecycle
 * state. Any value other than `"running"` is treated as not-running (defensive default).
 *
 * @param {"standby"|"running"} lifecycle
 * @param {number} mainColor - color to use for the "Start" (not running) state
 * @param {number} stopColor - color to use for the "Stop" (running) state
 * @returns {{name:string, color:number}}
 * @example
 * startSlotDisplayFor("running", 0x00a5ff, 0x0000ff); // { name: "Stop", color: 0x0000ff }
 */
function startSlotDisplayFor(lifecycle, mainColor, stopColor) {
  if (lifecycle === "running") return { name: "Stop", color: stopColor };
  return { name: "Start", color: mainColor };
}

/**
 * Decide what hardware trigger event (if any) a Start-slot `selected` field implies. Only
 * a `true` selection is meaningful (deselection is not a trigger) — same convention as the
 * existing bus/main sends-mode selection handling in `index.js`.
 *
 * @param {"standby"|"running"} lifecycle
 * @param {any} selectedValue - `parsed.selected` from the incoming SysEx JSON
 * @returns {"start"|"stop"|null}
 * @example
 * hardwareTriggerTypeFor("standby", true); // "start"
 * hardwareTriggerTypeFor("running", true); // "stop"
 */
function hardwareTriggerTypeFor(lifecycle, selectedValue) {
  if (selectedValue !== true) return null;
  return lifecycle === "running" ? "stop" : "start";
}

/**
 * On/off color for a status indicator slot, given a boolean (or boolean-ish) value from a
 * live status snapshot. Only literal `true` counts as "on" — same strict-equality convention
 * as `hardwareTriggerTypeFor`, so a missing/malformed status field defaults to "off" rather
 * than throwing or needing special-casing by the caller.
 *
 * @param {any} isOn - e.g. `status.ipad` from a live status snapshot
 * @param {number} onColor
 * @param {number} offColor
 * @returns {number}
 * @example
 * statusSlotColorFor(true, 0x00ff00, 0x0000ff); // 0x00ff00
 * statusSlotColorFor(undefined, 0x00ff00, 0x0000ff); // 0x0000ff
 */
function statusSlotColorFor(isOn, onColor, offColor) {
  return isOn === true ? onColor : offColor;
}

module.exports = {
  STATUS_BANK_INDICATORS,
  STATUS_BANK_SIZE,
  START_SLOT_OBJECT_ID,
  buildStatusBankSlots,
  startSlotDisplayFor,
  hardwareTriggerTypeFor,
  statusSlotColorFor,
};
