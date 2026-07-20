/* global bridge, presets */

// When Electron is launched with `--verbose`, main passes `?verbose=1`.
// In that mode we keep processing bridge log lines (for mode/status), but do not
// render them into the GUI "Console Output" panel.
const HIDE_BRIDGE_LOG_IN_GUI = (() => {
  try {
    return new URLSearchParams(window.location.search).get("verbose") === "1";
  } catch {
    return false;
  }
})();

/**
 * @typedef {"input"|"bus"} LayoutKind
 */

/**
 * Bridge config persisted by the GUI and consumed by the bridge process.
 *
 * Notes:
 * - `inputTrackOrder` is 0-based channels and supports stereo pairs: `[left,right]`.
 * - `busTrackOrder` is 1-based bus numbers and supports stereo pairs: `[leftBus,rightBus]`.
 * - `c1SendToMsBusNumber` is 1-based bus numbers for Send1..Send6.
 *
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
 * Mutable renderer state derived from the current form and used by the editors.
 *
 * @typedef {object} UiState
 * @property {number} inputTotalCount
 * @property {number} busTotalCount
 * @property {number} inputCustomCount
 * @property {number} busCustomCount
 * @property {number[][]} inputRows
 * @property {number[][]} busRows
 * @property {number[]} c1SendToMsBusNumber
 */

/** @type {UiState} */
const state = {
  // Total available tracks (used for clamping and Sends mapping options).
  inputTotalCount: 32,
  busTotalCount: 16,
  // Custom layout length (the "Count" fields in the UI).
  inputCustomCount: 32,
  busCustomCount: 16,
  /** @type {number[][]} 1-based channel numbers; rows can be [n] or [n,n+1] */
  inputRows: [],
  /** @type {number[][]} 1-based bus numbers; rows can be [n] or [n,n+1] */
  busRows: [],
  /** @type {number[]} length=6, values are 1-based bus numbers */
  c1SendToMsBusNumber: [1, 2, 3, 7, 9, 13],
};

let currentMode = "—";
let stopRequestedByUser = false;

const DEFAULT_PRESET_ID = "__default__";
const DEFAULT_PRESET_NAME = "Default";
const LAST_PRESET_STORAGE_KEY = "softubeMsBridge.lastPresetId";
const LAYOUT_TAB_STORAGE_KEY = "softubeMsBridge.layoutTab";

let uiRunning = false;
let uiError = false;

let lastStartedConfigKey = "";
let applyInProgress = false;

/**
 * Create a complete config object with safe defaults.
 *
 * @param {{ inputCount?: number, busCount?: number }} [opts]
 * @returns {BridgeConfig}
 * @example
 * const cfg = makeDefaultConfig({ inputCount: 32, busCount: 16 });
 * console.log(cfg.mixingStationWsUrl);
 */
function makeDefaultConfig({ inputCount = 32, busCount = 16 } = {}) {
  const inCount = clampInt(Number(inputCount), 1, 512);
  const bCount = clampInt(Number(busCount), 1, 512);

  return {
    mixingStationWsUrl: "ws://localhost:8080",
    logJson: false,
    metering2IntervalMs: 100,
    // Total counts (independent of custom order length).
    inputCount: inCount,
    busCount: bCount,
    // Default preset: start with an empty custom prefix so the user can drag channels
    // from the Unorganized lists into the custom layout.
    inputTrackOrder: [],
    busTrackOrder: [],
    c1SendToMsBusNumber: [1, 2, 3, 4, 5, 6],
    console1MainColor: 0x00a5ff,
    console1BusColor: 0x800080,
  };
}

/**
 * Read a localStorage key safely (e.g. when storage is disabled).
 * @param {string} key
 * @returns {string|null}
 */
function safeLocalStorageGet(key) {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

/**
 * Write a localStorage key safely (e.g. when storage is disabled).
 * @param {string} key
 * @param {string} value
 */
function safeLocalStorageSet(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // ignore
  }
}

/**
 * Wire up the Inputs/Buses tab UI.
 *
 * - Click switches tabs.
 * - Keyboard navigation uses ArrowLeft/ArrowRight/Home/End.
 * - Selected tab is persisted in localStorage.
 */
function setupTrackLayoutTabs() {
  const btnInputs = document.getElementById("layoutTabInputs");
  const btnBuses = document.getElementById("layoutTabBuses");
  const panelInputs = document.getElementById("layoutPanelInputs");
  const panelBuses = document.getElementById("layoutPanelBuses");
  if (!btnInputs || !btnBuses || !panelInputs || !panelBuses) return;

  const tabs = [btnInputs, btnBuses];

  const getTabKeyForButton = (btn) => (btn === btnBuses ? "buses" : "inputs");

  const focusTab = (tab) => {
    const btn = tab === "buses" ? btnBuses : btnInputs;
    try {
      btn.focus();
    } catch {
      // ignore
    }
  };

  const setActive = (tab, { moveFocus = false } = {}) => {
    const isInputs = tab === "inputs";

    btnInputs.classList.toggle("active", isInputs);
    btnBuses.classList.toggle("active", !isInputs);
    btnInputs.setAttribute("aria-selected", isInputs ? "true" : "false");
    btnBuses.setAttribute("aria-selected", !isInputs ? "true" : "false");

    btnInputs.tabIndex = isInputs ? 0 : -1;
    btnBuses.tabIndex = !isInputs ? 0 : -1;

    panelInputs.classList.toggle("hidden", !isInputs);
    panelBuses.classList.toggle("hidden", isInputs);
    panelInputs.setAttribute("aria-hidden", isInputs ? "false" : "true");
    panelBuses.setAttribute("aria-hidden", !isInputs ? "false" : "true");

    safeLocalStorageSet(LAYOUT_TAB_STORAGE_KEY, isInputs ? "inputs" : "buses");

    if (moveFocus) focusTab(isInputs ? "inputs" : "buses");
  };

  btnInputs.addEventListener("click", () => setActive("inputs"));
  btnBuses.addEventListener("click", () => setActive("buses"));

  const onKeyDown = (e) => {
    const key = e.key;
    if (!key) return;

    const currentBtn = tabs.includes(document.activeElement)
      ? document.activeElement
      : e.currentTarget;
    const currentTab = getTabKeyForButton(currentBtn);

    if (key === "ArrowLeft" || key === "ArrowRight") {
      e.preventDefault();
      setActive(currentTab === "inputs" ? "buses" : "inputs", { moveFocus: true });
      return;
    }

    if (key === "Home") {
      e.preventDefault();
      setActive("inputs", { moveFocus: true });
      return;
    }

    if (key === "End") {
      e.preventDefault();
      setActive("buses", { moveFocus: true });
      return;
    }
  };

  for (const t of tabs) t.addEventListener("keydown", onKeyDown);

  const saved = safeLocalStorageGet(LAYOUT_TAB_STORAGE_KEY);
  setActive(saved === "buses" ? "buses" : "inputs");
  updateLayoutTabBadges();
}

function rgbToSoftubeInt(hex) {
  // hex: #RRGGBB
  const m = /^#([0-9a-f]{6})$/i.exec(hex);
  if (!m) return null;
  const rr = parseInt(m[1].slice(0, 2), 16);
  const gg = parseInt(m[1].slice(2, 4), 16);
  const bb = parseInt(m[1].slice(4, 6), 16);
  return (rr & 0xff) | ((gg & 0xff) << 8) | ((bb & 0xff) << 16);
}

function softubeIntToRgb(intVal) {
  if (typeof intVal !== "number" || !Number.isFinite(intVal)) return "#000000";
  const rr = intVal & 0xff;
  const gg = (intVal >> 8) & 0xff;
  const bb = (intVal >> 16) & 0xff;
  return `#${rr.toString(16).padStart(2, "0")}${gg.toString(16).padStart(2, "0")}${bb
    .toString(16)
    .padStart(2, "0")}`;
}

function parseHexInt(str) {
  const s = String(str || "").trim();
  if (!s) return null;
  const m = /^(0x)?([0-9a-f]+)$/i.exec(s);
  if (!m) return null;
  return parseInt(m[2], 16);
}

function clampInt(n, min, max) {
  if (!Number.isFinite(n)) return min;
  return Math.max(min, Math.min(max, Math.trunc(n)));
}

function clampIntWithFallback(n, min, max, fallback) {
  const x = Number(n);
  if (!Number.isFinite(x)) return fallback;
  return Math.max(min, Math.min(max, Math.trunc(x)));
}

function setMeteringIntervalUi(ms) {
  const slider = document.getElementById("metering2IntervalMs");
  const valueInput = document.getElementById("metering2IntervalMsValue");
  if (!slider || !valueInput) return;

  const clamped = clampIntWithFallback(ms, 30, 1000, 100);
  slider.value = String(clamped);
  valueInput.value = String(clamped);
}

function getMeteringIntervalFromUi() {
  const valueInput = document.getElementById("metering2IntervalMsValue");
  const slider = document.getElementById("metering2IntervalMs");
  const raw = valueInput?.value ?? slider?.value;
  return clampIntWithFallback(raw, 30, 1000, 100);
}

function flattenRowsToMax(rows) {
  let maxVal = 0;
  for (const r of rows) {
    if (!Array.isArray(r)) continue;
    for (const v of r) {
      if (Number.isFinite(v)) maxVal = Math.max(maxVal, v);
    }
  }
  return maxVal;
}

function dedupeAndClampRows(rows, maxValue) {
  const used = new Set();
  /** @type {number[][]} */
  const out = [];
  for (const r of rows) {
    if (!Array.isArray(r) || r.length === 0) continue;
    const row = r.map((v) => clampInt(v, 1, maxValue));
    // Ensure sorted within stereo row and exactly 2 for stereo.
    if (row.length >= 2) {
      const a = row[0];
      const b = row[1];
      if (a === b) continue;
      const lo = Math.min(a, b);
      const hi = Math.max(a, b);
      if (used.has(lo) || used.has(hi)) continue;
      used.add(lo);
      used.add(hi);
      out.push([lo, hi]);
      continue;
    }
    const v = row[0];
    if (used.has(v)) continue;
    used.add(v);
    out.push([v]);
  }
  return out;
}

function rowsUsedNumbers(rows) {
  const used = new Set();
  for (const r of rows) {
    if (!Array.isArray(r)) continue;
    for (const v of r) {
      if (Number.isFinite(v)) used.add(v);
    }
  }
  return used;
}

function ensureRowsLength(rows, desiredLen, totalMax) {
  const maxValue = clampInt(Number(totalMax), 1, 512);
  const desired = clampInt(Number(desiredLen), 0, maxValue);

  if (desired === 0) return [];

  let out = dedupeAndClampRows(rows, maxValue);
  if (out.length > desired) out = out.slice(0, desired);

  const used = rowsUsedNumbers(out);
  for (let i = 1; i <= maxValue && out.length < desired; i++) {
    if (used.has(i)) continue;
    used.add(i);
    out.push([i]);
  }

  return out;
}

function normalizeC1SendMapping(raw, busCount) {
  const out = [];
  const maxBus = clampInt(Number(busCount), 1, 512);
  for (let i = 0; i < 6; i++) {
    const v = Number(Array.isArray(raw) ? raw[i] : undefined);
    out[i] = Number.isFinite(v) ? clampInt(v, 1, maxBus) : Math.min(i + 1, maxBus);
  }
  return out;
}

function setSendMappingUiFromState() {
  for (let i = 1; i <= 6; i++) {
    const sel = document.getElementById(`sendMap${i}`);
    if (!sel) continue;
    const v = Number(state.c1SendToMsBusNumber[i - 1]);
    sel.value = String(Number.isFinite(v) ? v : i);
  }
}

function renderSendMappingOptions() {
  const maxBus = clampInt(state.busTotalCount, 1, 512);
  for (let i = 1; i <= 6; i++) {
    const sel = document.getElementById(`sendMap${i}`);
    if (!sel) continue;

    const prev = sel.value;
    sel.innerHTML = "";
    for (let b = 1; b <= maxBus; b++) {
      const opt = document.createElement("option");
      opt.value = String(b);
      opt.textContent = `Bus ${b}`;
      sel.appendChild(opt);
    }

    // Restore selection if still valid.
    if (prev && sel.querySelector(`option[value="${CSS.escape(prev)}"]`)) {
      sel.value = prev;
    }
  }
  setSendMappingUiFromState();
}

/**
 * Convert a config object to a stable string suitable for change detection.
 *
 * This intentionally normalizes defaults and types so the Apply button only
 * appears when the config *meaningfully* changed.
 *
 * @param {Partial<BridgeConfig>} cfg
 * @returns {string}
 * @example
 * const a = canonicalizeConfigForCompare(getConfigFromForm());
 * const b = canonicalizeConfigForCompare(getConfigFromForm());
 * console.log(a === b); // true
 */
function canonicalizeConfigForCompare(cfg) {
  // Stable order, avoid noisy differences.
  const obj = {
    mixingStationWsUrl: String(cfg?.mixingStationWsUrl || "ws://localhost:8080"),
    logJson: !!cfg?.logJson,
    metering2IntervalMs: clampIntWithFallback(cfg?.metering2IntervalMs, 30, 1000, 100),
    inputCount: clampInt(Number(cfg?.inputCount ?? state.inputTotalCount), 1, 512),
    busCount: clampInt(Number(cfg?.busCount ?? state.busTotalCount), 1, 512),
    inputTrackOrder: Array.isArray(cfg?.inputTrackOrder) ? cfg.inputTrackOrder : [],
    busTrackOrder: Array.isArray(cfg?.busTrackOrder) ? cfg.busTrackOrder : [],
    c1SendToMsBusNumber: Array.isArray(cfg?.c1SendToMsBusNumber)
      ? cfg.c1SendToMsBusNumber.map((v) => clampInt(Number(v), 1, 512))
      : [],
    console1MainColor:
      typeof cfg?.console1MainColor === "number" ? cfg.console1MainColor : undefined,
    console1BusColor: typeof cfg?.console1BusColor === "number" ? cfg.console1BusColor : undefined,
  };
  return JSON.stringify(obj);
}

/**
 * Update the visibility/enabled-state of the Apply button based on:
 * - bridge running state
 * - whether the current form differs from the last started/applied config
 */
function updateApplyButton() {
  const btn = document.getElementById("btnApply");
  if (!btn) return;
  if (!uiRunning) {
    btn.classList.add("hidden");
    btn.disabled = true;
    return;
  }
  const key = canonicalizeConfigForCompare(getConfigFromForm());
  const dirty = !!lastStartedConfigKey && key !== lastStartedConfigKey;
  btn.classList.toggle("hidden", !dirty);
  btn.disabled = !dirty || applyInProgress;
}

function configInputOrderToRows(inputTrackOrder) {
  if (!Array.isArray(inputTrackOrder)) return [];
  /** @type {number[][]} */
  const rows = [];
  for (const entry of inputTrackOrder) {
    if (Array.isArray(entry) && entry.length === 2) {
      rows.push([Number(entry[0]) + 1, Number(entry[1]) + 1]);
    } else if (typeof entry === "number") {
      rows.push([entry + 1]);
    }
  }
  return rows;
}

function configBusOrderToRows(busTrackOrder) {
  if (!Array.isArray(busTrackOrder)) return [];
  /** @type {number[][]} */
  const rows = [];
  for (const entry of busTrackOrder) {
    if (Array.isArray(entry) && entry.length === 2) {
      rows.push([Number(entry[0]), Number(entry[1])]);
    } else if (typeof entry === "number") {
      rows.push([entry]);
    }
  }
  return rows;
}

function rowsToConfigInputOrder(rows) {
  /** @type {(number|number[])[]} */
  const out = [];
  for (const r of rows) {
    if (!Array.isArray(r) || r.length === 0) continue;
    if (r.length === 1) out.push(r[0] - 1);
    else out.push([r[0] - 1, r[1] - 1]);
  }
  return out;
}

function rowsToConfigBusOrder(rows) {
  /** @type {(number|number[])[]} */
  const out = [];
  for (const r of rows) {
    if (!Array.isArray(r) || r.length === 0) continue;
    if (r.length === 1) out.push(r[0]);
    else out.push([r[0], r[1]]);
  }
  return out;
}

function generateSequentialRows(count) {
  /** @type {number[][]} */
  const rows = [];
  for (let i = 1; i <= count; i++) rows.push([i]);
  return rows;
}

function labelFor(kind, row) {
  const a = row[0];
  const b = row.length === 2 ? row[1] : null;
  const prefix = kind === "input" ? "Ch" : "Bus";
  return b ? `${prefix} ${a}+${b}` : `${prefix} ${a}`;
}

function labelForRest(kind, n) {
  const prefix = kind === "input" ? "Ch" : "Bus";
  return `${prefix} ${n}`;
}

function canLinkAdjacent(rows, index) {
  const row = rows[index];
  const next = rows[index + 1];
  if (!row || !next) return false;
  if (row.length !== 1 || next.length !== 1) return false;
  return next[0] === row[0] + 1;
}

function moveArrayItem(arr, fromIndex, toIndex) {
  if (!Array.isArray(arr)) return;
  if (fromIndex === toIndex) return;
  if (fromIndex < 0 || fromIndex >= arr.length) return;
  if (toIndex < 0 || toIndex >= arr.length) return;

  const [item] = arr.splice(fromIndex, 1);
  arr.splice(toIndex, 0, item);
}

function applyDefaultPresetToForm() {
  setFormFromConfig(
    makeDefaultConfig({
      inputCount: state.inputTotalCount,
      busCount: state.busTotalCount,
    })
  );
}

function computeRestNumbers(kind) {
  const isInput = kind === "input";
  const total = isInput ? state.inputTotalCount : state.busTotalCount;
  const rows = isInput ? state.inputRows : state.busRows;
  const used = rowsUsedNumbers(rows);
  const out = [];
  for (let i = 1; i <= total; i++) {
    if (!used.has(i)) out.push(i);
  }
  return out;
}

/**
 * Refresh the little numeric badges on the Inputs/Buses tabs.
 *
 * Badge = number of tracks currently in the "Unorganized" list.
 * Hidden when the count is 0.
 */
function updateLayoutTabBadges() {
  const inputsBadge = document.getElementById("layoutTabInputsBadge");
  const busesBadge = document.getElementById("layoutTabBusesBadge");

  const setBadge = (el, count) => {
    if (!el) return;
    const n = Number(count);
    if (Number.isFinite(n) && n > 0) {
      el.textContent = String(n);
      el.classList.remove("hidden");
    } else {
      el.textContent = "";
      el.classList.add("hidden");
    }
  };

  setBadge(inputsBadge, computeRestNumbers("input").length);
  setBadge(busesBadge, computeRestNumbers("bus").length);
}

function renderRestList(kind) {
  const isInput = kind === "input";
  const container = document.getElementById(isInput ? "inputRest" : "busRest");
  if (!container) return;

  container.innerHTML = "";
  const rest = computeRestNumbers(kind);

  if (rest.length === 0) {
    const empty = document.createElement("div");
    empty.className = "muted small";
    empty.style.padding = "10px";
    empty.textContent = "Nothing left";
    container.appendChild(empty);
    return;
  }

  for (const n of rest) {
    const item = document.createElement("div");
    item.className = "list-item";
    item.draggable = true;

    item.addEventListener("dragstart", (e) => {
      item.classList.add("dragging");
      try {
        // Allow both copy and move drop effects; some Chromium builds reject a drop if
        // dropEffect isn't included in effectAllowed.
        e.dataTransfer.effectAllowed = "copyMove";
        e.dataTransfer.setData(
          "application/json",
          JSON.stringify({ kind, source: "rest", value: n })
        );
        e.dataTransfer.setData("text/plain", `${kind}:rest:${n}`);
      } catch {
        // ignore
      }
    });

    item.addEventListener("dragend", () => {
      item.classList.remove("dragging");
    });

    const left = document.createElement("div");
    left.className = "chip";
    left.innerHTML = `<strong>${labelForRest(
      kind,
      n
    )}</strong><span class="muted">unassigned</span>`;

    const right = document.createElement("div");
    right.className = "btn-group";
    const hint = document.createElement("div");
    hint.className = "muted small";
    hint.textContent = "Drag";
    right.appendChild(hint);

    item.appendChild(left);
    item.appendChild(right);
    container.appendChild(item);
  }
}

function syncRestLists() {
  renderRestList("input");
  renderRestList("bus");
  updateLayoutTabBadges();
}

function insertFromRestIntoRows(kind, value, insertIndex) {
  const isInput = kind === "input";
  const rows = isInput ? state.inputRows : state.busRows;
  const total = isInput ? state.inputTotalCount : state.busTotalCount;

  const v = clampInt(Number(value), 1, total);
  const used = rowsUsedNumbers(rows);
  if (used.has(v)) return;

  const idx = clampInt(Number(insertIndex), 0, rows.length);
  rows.splice(idx, 0, [v]);

  if (isInput) {
    state.inputCustomCount = Math.min(total, rows.length);
    document.getElementById("inputCount").value = String(state.inputCustomCount);
  } else {
    state.busCustomCount = Math.min(total, rows.length);
    document.getElementById("busCount").value = String(state.busCustomCount);
  }
}

function removeRowToRest(kind, index) {
  const isInput = kind === "input";
  const rows = isInput ? state.inputRows : state.busRows;
  const total = isInput ? state.inputTotalCount : state.busTotalCount;

  const idx = Number(index);
  if (!Number.isFinite(idx) || idx < 0 || idx >= rows.length) return;

  rows.splice(idx, 1);

  if (isInput) {
    state.inputCustomCount = clampInt(rows.length, 0, total);
    document.getElementById("inputCount").value = String(state.inputCustomCount);
  } else {
    state.busCustomCount = clampInt(rows.length, 0, total);
    document.getElementById("busCount").value = String(state.busCustomCount);
  }
}

function setupRestDropTargets() {
  /** @param {"input"|"bus"} kind */
  const wire = (kind) => {
    const isInput = kind === "input";
    const container = document.getElementById(isInput ? "inputRest" : "busRest");
    if (!container) return;

    container.ondragover = (e) => {
      e.preventDefault();
      try {
        e.dataTransfer.dropEffect = "move";
      } catch {
        // ignore
      }
      container.classList.add("drag-over");
    };

    container.ondragleave = () => {
      container.classList.remove("drag-over");
    };

    container.ondrop = (e) => {
      e.preventDefault();
      container.classList.remove("drag-over");

      let payload = null;
      try {
        const raw = e.dataTransfer.getData("application/json");
        if (raw) payload = JSON.parse(raw);
      } catch {
        // ignore
      }

      if (!payload || payload.kind !== kind) return;
      // Only accept drops from the main list.
      if (payload.source === "rest") return;

      removeRowToRest(kind, payload.index);
      renderList(kind);
      syncRestLists();
      updateApplyButton();
    };
  };

  wire("input");
  wire("bus");
}

function renderList(kind) {
  const isInput = kind === "input";
  const container = document.getElementById(isInput ? "inputEditor" : "busEditor");
  const rows = isInput ? state.inputRows : state.busRows;

  if (!container) return;

  container.innerHTML = "";

  const handleDropPayload = (payload, dropIndex) => {
    if (!payload || payload.kind !== kind) return false;

    // Dragging from the "rest" list inserts a new row and increases Count.
    if (payload.source === "rest") {
      insertFromRestIntoRows(kind, payload.value, dropIndex);
      renderList(kind);
      syncRestLists();
      updateApplyButton();
      return true;
    }

    // Moving within the main list.
    const fromIndex = Number(payload.index);
    const toIndex = Number(dropIndex);
    if (!Number.isFinite(fromIndex) || !Number.isFinite(toIndex)) return false;
    moveArrayItem(rows, fromIndex, toIndex);
    renderList(kind);
    syncRestLists();
    updateApplyButton();
    return true;
  };

  // Allow dropping to the end of the list.
  container.ondragover = (e) => {
    e.preventDefault();
    try {
      e.dataTransfer.dropEffect = "move";
    } catch {
      // ignore
    }
  };

  container.ondrop = (e) => {
    e.preventDefault();
    let payload = null;
    try {
      const raw = e.dataTransfer.getData("application/json");
      if (raw) payload = JSON.parse(raw);
    } catch {
      // ignore
    }

    // Drop to end.
    handleDropPayload(payload, rows.length);
  };

  rows.forEach((row, idx) => {
    const item = document.createElement("div");
    item.className = "list-item";
    item.draggable = true;

    item.addEventListener("dragstart", (e) => {
      item.classList.add("dragging");
      try {
        e.dataTransfer.effectAllowed = "move";
        e.dataTransfer.setData("application/json", JSON.stringify({ kind, index: idx }));
        e.dataTransfer.setData("text/plain", `${kind}:${idx}`);
      } catch {
        // ignore
      }
    });

    item.addEventListener("dragend", () => {
      item.classList.remove("dragging");
      for (const el of container.querySelectorAll(".list-item.drag-over")) {
        el.classList.remove("drag-over");
      }
    });

    item.addEventListener("dragover", (e) => {
      e.preventDefault();
      try {
        e.dataTransfer.dropEffect = "move";
      } catch {
        // ignore
      }
      item.classList.add("drag-over");
    });

    item.addEventListener("dragleave", () => {
      item.classList.remove("drag-over");
    });

    item.addEventListener("drop", (e) => {
      e.preventDefault();
      item.classList.remove("drag-over");

      let payload = null;
      try {
        const raw = e.dataTransfer.getData("application/json");
        if (raw) payload = JSON.parse(raw);
      } catch {
        // ignore
      }

      if (!payload || payload.kind !== kind) return;
      handleDropPayload(payload, idx);
    });

    const left = document.createElement("div");
    left.className = "chip";
    left.innerHTML = `<strong>${labelFor(kind, row)}</strong><span class="muted">${
      row.length === 2 ? "stereo" : "mono"
    }</span>`;

    const right = document.createElement("div");
    right.className = "btn-group";

    const btnUp = document.createElement("button");
    btnUp.className = "btn small";
    btnUp.textContent = "↑";
    btnUp.disabled = idx === 0;
    btnUp.addEventListener("click", () => {
      const tmp = rows[idx - 1];
      rows[idx - 1] = rows[idx];
      rows[idx] = tmp;
      renderList(kind);
      updateApplyButton();
    });

    const btnDown = document.createElement("button");
    btnDown.className = "btn small";
    btnDown.textContent = "↓";
    btnDown.disabled = idx === rows.length - 1;
    btnDown.addEventListener("click", () => {
      const tmp = rows[idx + 1];
      rows[idx + 1] = rows[idx];
      rows[idx] = tmp;
      renderList(kind);
      updateApplyButton();
    });

    const btnLink = document.createElement("button");
    btnLink.className = "btn small";

    if (row.length === 2) {
      btnLink.textContent = "Unlink";
      btnLink.addEventListener("click", () => {
        const a = row[0];
        const b = row[1];
        rows.splice(idx, 1, [a], [b]);
        renderList(kind);
        updateApplyButton();
      });
    } else {
      btnLink.textContent = "Link";
      btnLink.disabled = !canLinkAdjacent(rows, idx);
      btnLink.addEventListener("click", () => {
        if (!canLinkAdjacent(rows, idx)) return;
        const a = rows[idx][0];
        const b = rows[idx + 1][0];
        rows.splice(idx, 2, [a, b]);
        renderList(kind);
        updateApplyButton();
      });
    }

    right.appendChild(btnUp);
    right.appendChild(btnDown);
    right.appendChild(btnLink);

    const btnRemove = document.createElement("button");
    btnRemove.className = "btn small danger";
    btnRemove.textContent = "✕";
    btnRemove.title = "Move to Unorganized";
    btnRemove.addEventListener("click", () => {
      removeRowToRest(kind, idx);
      renderList(kind);
      syncRestLists();
      updateApplyButton();
    });

    right.appendChild(btnRemove);

    item.appendChild(left);
    item.appendChild(right);
    container.appendChild(item);
  });

  // Keep rest list in sync.
  syncRestLists();
}

function syncCountsToState() {
  document.getElementById("inputCount").value = String(state.inputCustomCount);
  document.getElementById("busCount").value = String(state.busCustomCount);
}

/**
 * Read the current UI form state and build a config payload for the bridge.
 *
 * @returns {BridgeConfig}
 * @example
 * const cfg = getConfigFromForm();
 * await bridge.applyConfig(cfg);
 */
function getConfigFromForm() {
  const wsUrl = document.getElementById("wsUrl").value.trim();
  const logJson = document.getElementById("logJson").checked;
  const metering2IntervalMs = getMeteringIntervalFromUi();

  const inputTrackOrder = rowsToConfigInputOrder(state.inputRows);
  const busTrackOrder = rowsToConfigBusOrder(state.busRows);

  const mainColorInt = parseHexInt(document.getElementById("mainColorInt").value);
  const busColorInt = parseHexInt(document.getElementById("busColorInt").value);

  const c1SendToMsBusNumber = [];
  for (let i = 1; i <= 6; i++) {
    const sel = document.getElementById(`sendMap${i}`);
    const v = Number(sel?.value);
    c1SendToMsBusNumber.push(Number.isFinite(v) ? clampInt(v, 1, state.busTotalCount) : i);
  }

  return {
    mixingStationWsUrl: wsUrl || "ws://localhost:8080",
    logJson,
    metering2IntervalMs,
    // Total counts.
    inputCount: state.inputTotalCount,
    busCount: state.busTotalCount,
    inputTrackOrder,
    busTrackOrder,
    c1SendToMsBusNumber,
    console1MainColor: mainColorInt ?? undefined,
    console1BusColor: busColorInt ?? undefined,
  };
}

/**
 * Populate the UI from a config object.
 *
 * This updates form controls and rebuilds the drag/drop editors.
 *
 * @param {Partial<BridgeConfig>} cfg
 * @returns {void}
 * @example
 * setFormFromConfig(makeDefaultConfig());
 */
function setFormFromConfig(cfg) {
  document.getElementById("wsUrl").value = cfg?.mixingStationWsUrl || "ws://localhost:8080";
  document.getElementById("logJson").checked = !!cfg?.logJson;

  setMeteringIntervalUi(cfg?.metering2IntervalMs);

  // Build editor rows from config (or neutral sequential defaults).
  const inputRowsRaw = Array.isArray(cfg?.inputTrackOrder)
    ? configInputOrderToRows(cfg.inputTrackOrder)
    : generateSequentialRows(32);
  const busRowsRaw = Array.isArray(cfg?.busTrackOrder)
    ? configBusOrderToRows(cfg.busTrackOrder)
    : generateSequentialRows(16);

  state.inputTotalCount = clampInt(
    Number(cfg?.inputCount ?? (flattenRowsToMax(inputRowsRaw) || 32)),
    1,
    512
  );
  state.busTotalCount = clampInt(
    Number(cfg?.busCount ?? (flattenRowsToMax(busRowsRaw) || 16)),
    1,
    512
  );

  // Custom count is the number of entries explicitly provided.
  state.inputCustomCount = clampInt(
    Number(Array.isArray(cfg?.inputTrackOrder) ? inputRowsRaw.length : state.inputTotalCount),
    0,
    state.inputTotalCount
  );
  state.busCustomCount = clampInt(
    Number(Array.isArray(cfg?.busTrackOrder) ? busRowsRaw.length : state.busTotalCount),
    0,
    state.busTotalCount
  );

  state.inputRows = ensureRowsLength(inputRowsRaw, state.inputCustomCount, state.inputTotalCount);
  state.busRows = ensureRowsLength(busRowsRaw, state.busCustomCount, state.busTotalCount);

  state.c1SendToMsBusNumber = normalizeC1SendMapping(cfg?.c1SendToMsBusNumber, state.busTotalCount);

  syncCountsToState();
  renderList("input");
  renderList("bus");

  renderSendMappingOptions();

  const mainInt = typeof cfg?.console1MainColor === "number" ? cfg.console1MainColor : 0x00a5ff;
  const busInt = typeof cfg?.console1BusColor === "number" ? cfg.console1BusColor : 0x800080;

  document.getElementById("mainColorInt").value = `0x${mainInt
    .toString(16)
    .padStart(6, "0")
    .toUpperCase()}`;
  document.getElementById("busColorInt").value = `0x${busInt
    .toString(16)
    .padStart(6, "0")
    .toUpperCase()}`;

  document.getElementById("mainColorPicker").value = softubeIntToRgb(mainInt);
  document.getElementById("busColorPicker").value = softubeIntToRgb(busInt);
}

function appendLog(line) {
  const logEl = document.getElementById("log");
  const auto = document.getElementById("autoScroll").checked;

  const text = String(line ?? "");
  const cls = classifyLogLine(text);

  const span = document.createElement("span");
  span.className = `log-line ${cls}`;
  span.textContent = text;
  logEl.appendChild(span);

  if (auto) logEl.scrollTop = logEl.scrollHeight;
}

function isProbablyJsonLine(line) {
  const s = String(line || "").trim();
  if (!s) return false;
  const first = s[0];
  if (first !== "{" && first !== "[") return false;
  try {
    JSON.parse(s);
    return true;
  } catch {
    return false;
  }
}

function classifyLogLine(line) {
  const s = String(line || "");

  // Errors first (wins).
  if (/\b(error|failed|exception|cannot find module)\b/i.test(s)) return "log-error";
  if (/\bwarn(ing)?\b/i.test(s)) return "log-warn";
  if (/\b(reconnecting|reconnect)\b/i.test(s)) return "log-warn";
  if (/\bFailed to load config\b/i.test(s)) return "log-warn";
  if (/\bWebSocket closed\b/i.test(s)) return "log-warn";

  // Tag-based.
  if (/^\[GUI\]/i.test(s)) return "log-gui";
  if (s.includes("[Mode]")) return "log-mode";

  // Common bridge lifecycle / transport lines (muted, but distinct from normal logs).
  if (/\bConnected to Mixing Station WebSocket\b/i.test(s)) return "log-warn";
  if (/\bConsole 1 handshake sent\b/i.test(s)) return "log-warn";
  if (/\bListening to MIDI port\b/i.test(s)) return "log-warn";
  if (/\bOpened MIDI output port\b/i.test(s)) return "log-warn";
  if (/\bReceived RESET command\b/i.test(s)) return "log-warn";
  if (/\bSoftube-MS-Bridge running\b/i.test(s)) return "log-warn";
  if (/\bShutting down\b/i.test(s)) return "log-warn";

  // Debug-style logs.
  if (/\b(SysEx JSON|SysEx data|Received MIDI message)\b/i.test(s)) return "log-json";
  if (/\bFinalizing initialization\b/i.test(s)) return "log-warn";

  if (isProbablyJsonLine(s)) return "log-json";

  return "";
}

function setModeUi(modeText) {
  currentMode = modeText || "—";
  const el = document.getElementById("mode");
  if (!el) return;
  el.textContent = `Mode: ${currentMode}`;

  // Visual hint: sends mode = highlighted, standard/unknown = neutral.
  const isSends = /^Sends\b/i.test(currentMode);
  el.style.borderColor = isSends ? "rgba(59,130,246,0.6)" : "rgba(255,255,255,0.08)";
  el.style.background = isSends ? "rgba(59,130,246,0.12)" : "rgba(255,255,255,0.02)";
}

function updateModeFromLogLine(line) {
  const s = String(line || "");
  if (!s.includes("[Mode]")) return;

  // Bridge emits:
  // [Mode] SENDS active (msSendIndex=0, bus=1)
  // [Mode] STANDARD active
  if (s.includes("[Mode] STANDARD active")) {
    setModeUi("Standard");
    return;
  }

  const m = /\[Mode\]\s+SENDS\s+active\s+\(.*\bbus=(\d+)\b.*\)/i.exec(s);
  if (m) {
    setModeUi(`Sends (Bus ${m[1]})`);
  }
}

function setDetectStatus(text) {
  const el = document.getElementById("detectStatus");
  if (!el) return;
  el.textContent = text || "";
}

async function fetchMixingStationInformation(wsUrl) {
  return await new Promise((resolve, reject) => {
    let settled = false;
    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      try {
        ws.close();
      } catch {
        // ignore
      }
      reject(new Error("Timeout while waiting for /console/information"));
    }, 2500);

    const ws = new WebSocket(wsUrl);

    ws.addEventListener("open", () => {
      ws.send(JSON.stringify({ path: "/console/information", method: "GET" }));
    });

    ws.addEventListener("message", (evt) => {
      if (settled) return;
      try {
        const msg = JSON.parse(typeof evt.data === "string" ? evt.data : String(evt.data));
        if (!msg || msg.path !== "/console/information") return;
        settled = true;
        clearTimeout(timer);
        try {
          ws.close();
        } catch {
          // ignore
        }
        resolve(msg.body ?? msg);
      } catch {
        // ignore non-JSON
      }
    });

    ws.addEventListener("error", () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      reject(new Error("Failed to connect to Mixing Station WebSocket"));
    });
  });
}

function inferCountsFromInformation(info) {
  const types = Array.isArray(info?.channelTypes) ? info.channelTypes : [];
  const normalized = types
    .filter((t) => t && Number.isFinite(t.offset) && Number.isFinite(t.count))
    .map((t) => ({
      offset: t.offset,
      count: t.count,
      name: String(t.name || "").toLowerCase(),
      shortName: String(t.shortName || "").toLowerCase(),
      type: Number.isFinite(t.type) ? t.type : null,
    }))
    .sort((a, b) => a.offset - b.offset);

  const pick = (re) => normalized.find((t) => re.test(t.name) || re.test(t.shortName));

  // Prefer numeric channel type IDs when available (stable across mixers):
  // - Inputs: type 0
  // - Buses:  type 4
  // Fall back to name-based matching if type IDs are missing.
  const inputsByType = normalized.find((t) => t.type === 0);
  const inputsByName = pick(/\binput\b|\bin\b/);
  const inputsByOffset0 = normalized.find((t) => t.offset === 0);
  const inputs = inputsByType || inputsByName || inputsByOffset0 || normalized[0];

  const busesByType = normalized.find((t) => t.type === 4);
  const busesByName = pick(/\bbus\b/);
  const busesFallback = pick(/\bmix\b|\baux\b/);
  const buses =
    busesByType ||
    busesByName ||
    busesFallback ||
    normalized.find((t) => t.offset !== (inputs?.offset ?? -1) && t.count > 0);

  const inputCount = inputs ? clampInt(inputs.count, 1, 512) : 32;
  const busCount = buses ? clampInt(buses.count, 1, 512) : 16;

  return { inputCount, busCount };
}

function setStatusUi({ running, error } = {}) {
  const status = document.getElementById("status");
  const btnStart = document.getElementById("btnStart");
  const btnStop = document.getElementById("btnStop");

  if (typeof running === "boolean") uiRunning = running;
  if (typeof error === "boolean") uiError = error;

  status.textContent = uiError ? "Error" : uiRunning ? "Running" : "Stopped";

  if (uiError) {
    status.style.borderColor = "rgba(239,68,68,0.5)";
    status.style.background = "rgba(239,68,68,0.10)";
    status.style.color = "var(--danger)";
  } else if (uiRunning) {
    status.style.borderColor = "rgba(34,197,94,0.5)";
    status.style.background = "rgba(34,197,94,0.10)";
    status.style.color = "var(--text)";
  } else {
    status.style.borderColor = "rgba(255,255,255,0.08)";
    status.style.background = "rgba(255,255,255,0.02)";
    status.style.color = "var(--text)";
  }

  btnStart.disabled = uiRunning;
  btnStop.disabled = !uiRunning;

  updateApplyButton();
}

async function refreshPresets() {
  const list = await presets.list();
  const sel = document.getElementById("presetSelect");
  const prev = sel.value;
  sel.innerHTML = "";

  const placeholder = document.createElement("option");
  placeholder.value = "";
  placeholder.textContent = "Select preset…";
  sel.appendChild(placeholder);

  const defaultOpt = document.createElement("option");
  defaultOpt.value = DEFAULT_PRESET_ID;
  defaultOpt.textContent = DEFAULT_PRESET_NAME;
  sel.appendChild(defaultOpt);

  for (const p of list) {
    const opt = document.createElement("option");
    opt.value = p.id;
    opt.textContent = p.name;
    sel.appendChild(opt);
  }

  // Keep selection if still present.
  if (prev && sel.querySelector(`option[value="${CSS.escape(prev)}"]`)) {
    sel.value = prev;
  }

  return list;
}

function syncPresetActionButtons() {
  const id = document.getElementById("presetSelect").value;
  const isDefault = id === DEFAULT_PRESET_ID;
  const isEmpty = !id;

  const btnDelete = document.getElementById("btnDeletePreset");
  const btnExport = document.getElementById("btnExportPreset");

  btnDelete.disabled = isEmpty || isDefault;
  btnExport.disabled = isEmpty || isDefault;
}

async function loadPresetById(id, { log = true } = {}) {
  if (!id) return;

  if (id === DEFAULT_PRESET_ID) {
    applyDefaultPresetToForm();
    document.getElementById("presetName").value = DEFAULT_PRESET_NAME;
    safeLocalStorageSet(LAST_PRESET_STORAGE_KEY, DEFAULT_PRESET_ID);
    if (log) appendLog(`[GUI] Preset loaded: ${DEFAULT_PRESET_NAME}`);
    return;
  }

  const payload = await presets.load(id);
  setFormFromConfig(payload.config);
  document.getElementById("presetName").value = payload?.meta?.name || "";
  safeLocalStorageSet(LAST_PRESET_STORAGE_KEY, id);
  if (log) appendLog(`[GUI] Preset loaded: ${id}`);

  updateApplyButton();
}

async function autoLoadLastPresetIfAny() {
  const lastId = safeLocalStorageGet(LAST_PRESET_STORAGE_KEY);
  if (!lastId) return;

  const sel = document.getElementById("presetSelect");
  const exists = !!sel.querySelector(`option[value="${CSS.escape(lastId)}"]`);
  if (!exists) return;

  sel.value = lastId;
  syncPresetActionButtons();
  try {
    await loadPresetById(lastId, { log: false });
    appendLog(
      `[GUI] Auto-loaded preset: ${lastId === DEFAULT_PRESET_ID ? DEFAULT_PRESET_NAME : lastId}`
    );
  } catch (e) {
    appendLog(`[GUI] Failed to auto-load preset: ${e?.message || e}`);
    setStatusUi({ error: true });
  }
}

function currentPresetPayload(nameOverride) {
  const config = getConfigFromForm();
  return {
    meta: {
      name: String(nameOverride || "Preset").trim() || "Preset",
      savedAt: new Date().toISOString(),
    },
    config,
  };
}

/**
 * Start the bridge using the currently loaded form/config. Shared by the Start
 * button and `--start` CLI handling. No-ops if the bridge is already running
 * (the Start button is disabled in that state, but the CLI path has no such
 * guard — without this check a `--start` forwarded to an already-running
 * instance would hit the main process's "already running" throw and flip the
 * status badge to Error even though the bridge is fine).
 */
async function startBridgeFromForm() {
  if (uiRunning) return;
  try {
    const config = getConfigFromForm();
    await bridge.start(config);
    appendLog("[GUI] Bridge started");
    setStatusUi({ running: true, error: false });
    setModeUi("Standard");
    stopRequestedByUser = false;

    lastStartedConfigKey = canonicalizeConfigForCompare(config);
    updateApplyButton();
  } catch (e) {
    appendLog(`[GUI] Failed to start: ${e?.message || e}`);
    setStatusUi({ error: true });
  }
}

/**
 * Stop the bridge if running. Shared by the Stop button and `--stop` CLI
 * handling. No-ops if the bridge isn't running (the Stop button is disabled
 * in that state, but the CLI path has no such guard, hence the check here).
 */
async function stopBridgeFromUi() {
  if (!uiRunning) return;
  try {
    stopRequestedByUser = true;
    await bridge.stop();
    appendLog("[GUI] Bridge stopped");
  } finally {
    setStatusUi({ running: false, error: false });
    setModeUi("—");

    lastStartedConfigKey = "";
    updateApplyButton();
  }
}

/**
 * Apply parsed CLI args (from a fresh launch or a forwarded second instance).
 *
 * Order matters: preset load happens first, then ws/interval/log overrides
 * are applied on top (so CLI overrides always win over the preset's saved
 * values), then start/stop.
 *
 * @param {{start:boolean, stop:boolean, preset:string|null, ws:string|null, interval:number|null, log:boolean, warnings:string[]}} args
 */
async function applyCliArgs(args) {
  if (!args) return;

  for (const warning of args.warnings || []) {
    appendLog(`[GUI] CLI: ${warning}`);
  }

  let presetLoadFailed = false;

  if (args.preset) {
    const sel = document.getElementById("presetSelect");
    const wanted = String(args.preset).trim().toLowerCase();
    const match = Array.from(sel.options).find(
      (opt) => opt.value && opt.textContent.trim().toLowerCase() === wanted
    );
    if (match) {
      sel.value = match.value;
      syncPresetActionButtons();
      try {
        await loadPresetById(match.value);
      } catch (e) {
        presetLoadFailed = true;
        appendLog(`[GUI] Failed to load preset "${args.preset}": ${e?.message || e}`);
      }
    } else {
      appendLog(`[GUI] --preset "${args.preset}" not found; keeping current preset`);
    }
  }

  if (typeof args.ws === "string" && args.ws.trim()) {
    const wsValue = args.ws.trim();
    document.getElementById("wsUrl").value = /^wss?:\/\//i.test(wsValue)
      ? wsValue
      : `ws://${wsValue}`;
  }

  if (typeof args.interval === "number" && Number.isFinite(args.interval)) {
    setMeteringIntervalUi(args.interval);
  }

  if (args.log) {
    document.getElementById("logJson").checked = true;
  }

  updateApplyButton();

  if (args.start && !presetLoadFailed) {
    await startBridgeFromForm();
  } else if (args.stop) {
    await stopBridgeFromUi();
  }
}

/** Maps a status field name to its topbar dot element id. */
const STATUS_DOT_ELEMENT_IDS = {
  ipad: "indIpad",
  spdSxPro: "indSpdSxPro",
  midiMaestro: "indMidiMaestro",
  bomeMtp: "indBomeMtp",
  mixingStation: "indMixingStation",
  console1Osd: "indConsole1Osd",
  abletonLive: "indAbletonLive",
};

/** MIDI-device fields say "connected"; running-app fields say "running". */
const STATUS_DOT_VERB = {
  ipad: "connected",
  spdSxPro: "connected",
  midiMaestro: "connected",
  bomeMtp: "running",
  mixingStation: "running",
  console1Osd: "running",
  abletonLive: "running",
};

/**
 * Toggle the `.on` class on each topbar status dot per the given snapshot,
 * and keep `aria-label` in sync (color alone isn't accessible to screen
 * readers or keyboard users; `title` is hover-only).
 * @param {Record<string, boolean>} status
 */
function applyStatusIndicators(status) {
  if (!status) return;
  for (const [field, elementId] of Object.entries(STATUS_DOT_ELEMENT_IDS)) {
    const el = document.getElementById(elementId);
    if (!el) continue;
    const isOn = !!status[field];
    el.classList.toggle("on", isOn);
    const verb = STATUS_DOT_VERB[field];
    el.setAttribute("aria-label", `${el.title}: ${isOn ? verb : `not ${verb}`}`);
  }
}

async function init() {
  setFormFromConfig(
    makeDefaultConfig({ inputCount: state.inputTotalCount, busCount: state.busTotalCount })
  );

  setupTrackLayoutTabs();

  setupRestDropTargets();

  await refreshPresets();
  syncPresetActionButtons();
  await autoLoadLastPresetIfAny();

  setModeUi("—");
  bridge.onLog((line) => {
    updateModeFromLogLine(line);

    if (!HIDE_BRIDGE_LOG_IN_GUI) {
      appendLog(line);
    }

    const text = String(line || "");

    // If the bridge exits after the user hit Stop, don't treat it as an error.
    if (/\bBridge exited\b/i.test(text) && stopRequestedByUser) {
      stopRequestedByUser = false;
      setStatusUi({ error: false, running: false });
      return;
    }

    // If the bridge exits unexpectedly, reflect it in the status badge.
    if (/\bBridge exited\b/i.test(text) && !stopRequestedByUser) {
      setStatusUi({ error: true, running: false });
      return;
    }

    // Show error indicator if log line looks like an error.
    // (Avoid flagging normal shutdown paths.)
    if (/\b(error|failed|exception|cannot find module)\b/i.test(text)) {
      setStatusUi({ error: true });
    }
  });

  bridge.onCliArgs((args) => {
    applyCliArgs(args).catch((e) => {
      appendLog(`[GUI] Failed to apply CLI args: ${e?.message || e}`);
    });
  });

  bridge.onStatusUpdate((status) => {
    applyStatusIndicators(status);
  });

  // Keep status correct if GUI is reloaded.
  try {
    const st = await bridge.status();
    setStatusUi({ running: !!st?.running, error: false });
    // If bridge is already running and we haven't seen a mode line yet,
    // assume Standard until told otherwise.
    if (st?.running && currentMode === "—") setModeUi("Standard");
  } catch {
    setStatusUi({ running: false, error: false });
  }

  document.getElementById("btnStart").addEventListener("click", startBridgeFromForm);
  document.getElementById("btnStop").addEventListener("click", stopBridgeFromUi);

  document.getElementById("btnApply").addEventListener("click", async () => {
    if (!uiRunning) return;
    if (applyInProgress) return;
    applyInProgress = true;
    updateApplyButton();

    const config = getConfigFromForm();
    try {
      appendLog("[GUI] Applying changes (live)...");
      await bridge.applyConfig(config);
      appendLog("[GUI] Changes applied");
      setStatusUi({ running: true, error: false });
      setModeUi("Standard");
      stopRequestedByUser = false;
      lastStartedConfigKey = canonicalizeConfigForCompare(config);
    } catch (e) {
      appendLog(`[GUI] Live apply failed, restarting bridge: ${e?.message || e}`);
      try {
        stopRequestedByUser = true;
        await bridge.stop();
      } catch (e2) {
        appendLog(`[GUI] Stop failed during apply fallback: ${e2?.message || e2}`);
      }
      try {
        await bridge.start(config);
        appendLog("[GUI] Changes applied (restart)");
        setStatusUi({ running: true, error: false });
        setModeUi("Standard");
        stopRequestedByUser = false;
        lastStartedConfigKey = canonicalizeConfigForCompare(config);
      } catch (e3) {
        appendLog(`[GUI] Apply failed: ${e3?.message || e3}`);
        setStatusUi({ error: true, running: false });
      }
    } finally {
      applyInProgress = false;
      updateApplyButton();
    }
  });

  document.getElementById("btnClearLog").addEventListener("click", () => {
    document.getElementById("log").textContent = "";
  });

  document.getElementById("btnReloadPresets").addEventListener("click", async () => {
    await refreshPresets();
    syncPresetActionButtons();
  });

  document.getElementById("presetSelect").addEventListener("change", syncPresetActionButtons);

  document.getElementById("btnSavePreset").addEventListener("click", async () => {
    try {
      const name = document.getElementById("presetName").value;
      const payload = currentPresetPayload(name);
      const res = await presets.save(payload);
      appendLog(`[GUI] Preset saved: ${res.id}`);
      await refreshPresets();
      document.getElementById("presetSelect").value = res.id;
      syncPresetActionButtons();
      safeLocalStorageSet(LAST_PRESET_STORAGE_KEY, res.id);
    } catch (e) {
      appendLog(`[GUI] Failed to save preset: ${e?.message || e}`);
    }
  });

  document.getElementById("btnLoadPreset").addEventListener("click", async () => {
    const id = document.getElementById("presetSelect").value;
    if (!id) return;
    try {
      await loadPresetById(id);
    } catch (e) {
      appendLog(`[GUI] Failed to load preset: ${e?.message || e}`);
      setStatusUi({ error: true });
    }
  });

  document.getElementById("btnDeletePreset").addEventListener("click", async () => {
    const id = document.getElementById("presetSelect").value;
    if (!id || id === DEFAULT_PRESET_ID) return;
    try {
      await presets.delete(id);
      appendLog(`[GUI] Preset deleted: ${id}`);
      await refreshPresets();
      syncPresetActionButtons();
    } catch (e) {
      appendLog(`[GUI] Failed to delete preset: ${e?.message || e}`);
    }
  });

  document.getElementById("btnExportPreset").addEventListener("click", async () => {
    const id = document.getElementById("presetSelect").value;
    if (!id || id === DEFAULT_PRESET_ID) return;
    try {
      await presets.export(id);
      appendLog(`[GUI] Preset exported: ${id}`);
    } catch (e) {
      appendLog(`[GUI] Failed to export preset: ${e?.message || e}`);
    }
  });

  document.getElementById("btnImportPreset").addEventListener("click", async () => {
    try {
      const res = await presets.import();
      if (res?.canceled) return;
      appendLog(`[GUI] Preset imported: ${res.id}`);
      await refreshPresets();
      document.getElementById("presetSelect").value = res.id;
      syncPresetActionButtons();
      safeLocalStorageSet(LAST_PRESET_STORAGE_KEY, res.id);
    } catch (e) {
      appendLog(`[GUI] Failed to import preset: ${e?.message || e}`);
    }
  });

  document.getElementById("btnOpenPresetsFolder").addEventListener("click", async () => {
    try {
      await presets.openFolder();
      appendLog("[GUI] Opened presets folder");
    } catch (e) {
      appendLog(`[GUI] Failed to open presets folder: ${e?.message || e}`);
    }
  });

  document.getElementById("btnDetectFromMixer").addEventListener("click", async () => {
    try {
      setDetectStatus("Connecting...");
      const wsUrl = document.getElementById("wsUrl").value.trim() || "ws://localhost:8080";
      const info = await fetchMixingStationInformation(wsUrl);
      const { inputCount, busCount } = inferCountsFromInformation(info);

      state.inputTotalCount = inputCount;
      state.busTotalCount = busCount;

      // Keep custom counts within the detected total.
      state.inputCustomCount = clampInt(
        Number.isFinite(state.inputRows.length) ? state.inputRows.length : inputCount,
        0,
        inputCount
      );
      state.busCustomCount = clampInt(
        Number.isFinite(state.busRows.length) ? state.busRows.length : busCount,
        0,
        busCount
      );

      // Preserve current custom order as much as possible.
      if (!state.inputRows.length) state.inputRows = generateSequentialRows(state.inputCustomCount);
      state.inputRows = ensureRowsLength(state.inputRows, state.inputCustomCount, inputCount);

      if (!state.busRows.length) state.busRows = generateSequentialRows(state.busCustomCount);
      state.busRows = ensureRowsLength(state.busRows, state.busCustomCount, busCount);

      syncCountsToState();
      renderList("input");
      renderList("bus");

      // Rebuild sends mapping options when bus count changes.
      renderSendMappingOptions();
      updateApplyButton();

      setDetectStatus(`Detected: Inputs ${inputCount}, Buses ${busCount}`);
      appendLog(`[GUI] Mixer detected: Inputs=${inputCount}, Buses=${busCount}`);
    } catch (e) {
      setDetectStatus("");
      appendLog(`[GUI] Detect failed: ${e?.message || e}`);
    }
  });

  // Editor controls
  document.getElementById("btnInputGenerate").addEventListener("click", () => {
    const n = clampInt(Number(document.getElementById("inputCount").value), 0, 512);
    state.inputCustomCount = clampInt(n, 0, state.inputTotalCount);
    state.inputRows = ensureRowsLength(
      generateSequentialRows(state.inputCustomCount),
      state.inputCustomCount,
      state.inputTotalCount
    );
    syncCountsToState();
    renderList("input");
    updateApplyButton();
  });

  document.getElementById("btnBusGenerate").addEventListener("click", () => {
    const n = clampInt(Number(document.getElementById("busCount").value), 0, 512);
    state.busCustomCount = clampInt(n, 0, state.busTotalCount);
    state.busRows = ensureRowsLength(
      generateSequentialRows(state.busCustomCount),
      state.busCustomCount,
      state.busTotalCount
    );
    syncCountsToState();
    renderList("bus");
    renderSendMappingOptions();
    updateApplyButton();
  });

  document.getElementById("inputCount").addEventListener("change", () => {
    const n = clampInt(Number(document.getElementById("inputCount").value), 0, 512);
    state.inputCustomCount = clampInt(n, 0, state.inputTotalCount);
    state.inputRows = ensureRowsLength(
      state.inputRows,
      state.inputCustomCount,
      state.inputTotalCount
    );
    syncCountsToState();
    renderList("input");
    updateApplyButton();
  });

  document.getElementById("busCount").addEventListener("change", () => {
    const n = clampInt(Number(document.getElementById("busCount").value), 0, 512);
    state.busCustomCount = clampInt(n, 0, state.busTotalCount);
    state.busRows = ensureRowsLength(state.busRows, state.busCustomCount, state.busTotalCount);
    syncCountsToState();
    renderList("bus");

    // Custom layout changes should not affect available bus choices for Sends mapping.
    renderSendMappingOptions();
    updateApplyButton();
  });

  for (let i = 1; i <= 6; i++) {
    const sel = document.getElementById(`sendMap${i}`);
    if (!sel) continue;
    sel.addEventListener("change", () => {
      state.c1SendToMsBusNumber[i - 1] = clampInt(Number(sel.value), 1, state.busTotalCount);
      updateApplyButton();
    });
  }

  // Keep int <-> picker in sync.
  document.getElementById("mainColorPicker").addEventListener("input", (e) => {
    const intVal = rgbToSoftubeInt(e.target.value);
    if (intVal == null) return;
    document.getElementById("mainColorInt").value = `0x${intVal
      .toString(16)
      .padStart(6, "0")
      .toUpperCase()}`;
    updateApplyButton();
  });

  document.getElementById("busColorPicker").addEventListener("input", (e) => {
    const intVal = rgbToSoftubeInt(e.target.value);
    if (intVal == null) return;
    document.getElementById("busColorInt").value = `0x${intVal
      .toString(16)
      .padStart(6, "0")
      .toUpperCase()}`;
    updateApplyButton();
  });

  document.getElementById("mainColorInt").addEventListener("change", (e) => {
    const intVal = parseHexInt(e.target.value);
    if (intVal == null) return;
    document.getElementById("mainColorPicker").value = softubeIntToRgb(intVal);
    updateApplyButton();
  });

  document.getElementById("busColorInt").addEventListener("change", (e) => {
    const intVal = parseHexInt(e.target.value);
    if (intVal == null) return;
    document.getElementById("busColorPicker").value = softubeIntToRgb(intVal);
    updateApplyButton();
  });

  document.getElementById("wsUrl").addEventListener("change", updateApplyButton);
  document.getElementById("logJson").addEventListener("change", updateApplyButton);

  const meteringSlider = document.getElementById("metering2IntervalMs");
  if (meteringSlider) {
    meteringSlider.addEventListener("input", () => {
      setMeteringIntervalUi(meteringSlider.value);
      updateApplyButton();
    });
    meteringSlider.addEventListener("change", () => {
      setMeteringIntervalUi(meteringSlider.value);
      updateApplyButton();
    });
  }

  const meteringValue = document.getElementById("metering2IntervalMsValue");
  if (meteringValue) {
    meteringValue.addEventListener("input", () => {
      setMeteringIntervalUi(meteringValue.value);
      updateApplyButton();
    });
    meteringValue.addEventListener("change", () => {
      setMeteringIntervalUi(meteringValue.value);
      updateApplyButton();
    });
  }
}

init();
