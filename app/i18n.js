/**
 * Minimal i18n loader for the Electron GUI.
 *
 * Scope: this only translates the GUI (index.html/renderer.js) — the bridge process
 * (`index.js`)'s own console output is a technical/debug log, not user-facing UI, and stays
 * English-only by design (same rationale as most server/daemon logs).
 *
 * How it works: locale files are flat `{ "key": "string" }` JSON maps under `app/locales/`.
 * `en.json` is the source of truth (every key must exist there) and also the fallback for any
 * key missing from a non-English locale file, so a partial/in-progress translation never shows
 * a blank string or a raw key to the user.
 *
 * This module has no Electron dependency (only `fs`/`path`), so it works identically whether
 * it's `require()`d from the main process or from a preload script.
 */

const fs = require("fs");
const path = require("path");

const LOCALES_DIR = path.join(__dirname, "locales");
const DEFAULT_LOCALE = "en";

/**
 * List available locale codes (one per `app/locales/*.json` file).
 * @returns {string[]}
 * @example
 * listAvailableLocales(); // ["en"]
 */
function listAvailableLocales() {
  try {
    return fs
      .readdirSync(LOCALES_DIR)
      .filter((f) => f.endsWith(".json"))
      .map((f) => f.replace(/\.json$/, ""));
  } catch {
    return [DEFAULT_LOCALE];
  }
}

/**
 * Load a locale's strings, merged over the English fallback so missing keys in a
 * partial translation still resolve to something readable.
 *
 * @param {string} localeCode
 * @returns {Record<string, string>}
 * @example
 * loadLocaleStrings("en").appTitle; // "Softube-MS-Bridge"
 */
function loadLocaleStrings(localeCode) {
  const fallback = JSON.parse(fs.readFileSync(path.join(LOCALES_DIR, `${DEFAULT_LOCALE}.json`), "utf8"));
  if (localeCode === DEFAULT_LOCALE) return fallback;

  try {
    const raw = fs.readFileSync(path.join(LOCALES_DIR, `${localeCode}.json`), "utf8");
    const overrides = JSON.parse(raw);
    return { ...fallback, ...overrides };
  } catch {
    // Missing/invalid locale file: fall back to English entirely rather than fail to launch.
    return fallback;
  }
}

/**
 * Pick which locale to actually use: an explicit request (e.g. a future user setting or the
 * OS locale) if we have a file for it, otherwise English.
 *
 * @param {string|undefined|null} requestedLocale - e.g. `app.getLocale()`'s result
 * @returns {string}
 * @example
 * resolveLocale("cs-CZ"); // "en" (no cs.json shipped yet)
 */
function resolveLocale(requestedLocale) {
  if (!requestedLocale) return DEFAULT_LOCALE;
  const available = listAvailableLocales();
  const code = String(requestedLocale).toLowerCase();
  // Try exact match, then the base language (e.g. "cs-CZ" -> "cs").
  if (available.includes(code)) return code;
  const base = code.split("-")[0];
  if (available.includes(base)) return base;
  return DEFAULT_LOCALE;
}

/**
 * Build a `t(key, vars?)` translator over a loaded strings map. Unknown keys resolve to the
 * key itself (visibly wrong but never crashes) so a typo'd key is easy to spot during
 * development. `{placeholder}` tokens in the string are replaced from `vars`.
 *
 * @param {Record<string, string>} strings
 * @returns {(key: string, vars?: Record<string, string|number>) => string}
 * @example
 * const t = createTranslator({ greeting: "Hello, {name}!" });
 * t("greeting", { name: "World" }); // "Hello, World!"
 */
function createTranslator(strings) {
  return function t(key, vars) {
    const template = Object.prototype.hasOwnProperty.call(strings, key) ? strings[key] : key;
    if (!vars) return template;
    return template.replace(/\{(\w+)\}/g, (match, varName) =>
      Object.prototype.hasOwnProperty.call(vars, varName) ? String(vars[varName]) : match,
    );
  };
}

module.exports = {
  DEFAULT_LOCALE,
  LOCALES_DIR,
  listAvailableLocales,
  loadLocaleStrings,
  resolveLocale,
  createTranslator,
};
