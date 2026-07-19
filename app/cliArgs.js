/**
 * Pure CLI argument parsing for the Electron GUI launcher.
 *
 * Deliberately has no dependency on `electron` so it can be unit-tested
 * with the plain Node test runner (`node --test`).
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
 * Strip the leading Electron-supplied argv entries so only user-provided
 * flags remain.
 *
 * - Packaged app: `argv[0]` is the app executable; everything after is ours.
 * - Dev/unpackaged: `argv[0]` is the electron binary, `argv[1]` is the script
 *   path (e.g. `app/main.js`) we passed it; everything after that is ours.
 *
 * @param {string[]} argv - `process.argv` (or a forwarded `second-instance` argv)
 * @param {boolean} isPackaged - `app.isPackaged`
 * @returns {string[]}
 */
function getUserArgv(argv, isPackaged) {
  const list = Array.isArray(argv) ? argv : [];
  return isPackaged ? list.slice(1) : list.slice(2);
}

/**
 * Parse already-stripped user CLI args (see `getUserArgv`).
 *
 * @param {string[]} args
 * @returns {ParsedCliArgs}
 */
function parseCliArgs(args) {
  const result = {
    start: false,
    stop: false,
    preset: null,
    ws: null,
    interval: null,
    log: false,
    verbose: false,
    warnings: [],
  };

  const list = Array.isArray(args) ? args : [];

  const takeValue = (i) => {
    const next = list[i + 1];
    if (typeof next === "string" && next.length > 0 && !next.startsWith("--")) {
      return { value: next, consumed: true };
    }
    return { value: undefined, consumed: false };
  };

  for (let i = 0; i < list.length; i++) {
    const arg = list[i];
    switch (arg) {
      case "--start":
        result.start = true;
        break;
      case "--stop":
        result.stop = true;
        break;
      case "--log":
        result.log = true;
        break;
      case "--verbose":
        result.verbose = true;
        break;
      case "--preset": {
        const { value, consumed } = takeValue(i);
        if (consumed) {
          result.preset = value;
          i++;
        } else {
          result.warnings.push("--preset requires a value");
        }
        break;
      }
      case "--ws": {
        const { value, consumed } = takeValue(i);
        if (consumed) {
          result.ws = value;
          i++;
        } else {
          result.warnings.push("--ws requires a value");
        }
        break;
      }
      case "--interval": {
        const { value, consumed } = takeValue(i);
        if (consumed) {
          i++;
          const n = Number(value);
          if (Number.isFinite(n)) {
            result.interval = Math.trunc(n);
          } else {
            result.warnings.push("--interval requires a numeric value");
          }
        } else {
          result.warnings.push("--interval requires a numeric value");
        }
        break;
      }
      default:
        result.warnings.push(`Unknown argument: ${arg}`);
        break;
    }
  }

  if (result.start && result.stop) {
    result.warnings.push("--start and --stop given together; ignoring both");
    result.start = false;
    result.stop = false;
  }

  return result;
}

module.exports = { parseCliArgs, getUserArgv };
