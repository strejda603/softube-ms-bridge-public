import en from "../locales/en.json";
import cs from "../locales/cs.json";
import { LOCALE_STORAGE_KEY, resolveLocale } from "./localeResolve";

type LocaleStrings = Record<string, string>;

const locales: Record<string, LocaleStrings> = { en, cs };

/** Thin, untested glue (see `localeResolve.ts`'s doc comment for why) -- reads the persisted
 * locale choice. Wrapped in try/catch since `localStorage` can throw in restricted contexts,
 * matching `App.svelte`'s existing `safeLocalStorageGetInt` pattern. */
function readPersistedLocale(): string | null {
  try {
    return localStorage.getItem(LOCALE_STORAGE_KEY);
  } catch {
    return null;
  }
}

/** Thin, untested glue -- persists the locale choice. */
function writePersistedLocale(code: string) {
  try {
    localStorage.setItem(LOCALE_STORAGE_KEY, code);
  } catch {
    // ignore
  }
}

let currentLocale = $state(resolveLocale(readPersistedLocale(), Object.keys(locales)));

export function getLocale(): string {
  return currentLocale;
}

/** Switches the active locale if it's known, else falls back to "en". When `persist` is true
 * (the default -- a normal UI-driven switch), also saves the choice to `localStorage` so it
 * survives the next launch. Pass `persist: false` for a session-only override (the `--lang`
 * CLI launch flag, applied once in `App.svelte`'s `onMount`) that must not overwrite the
 * user's saved preference. */
export function setLocale(code: string, persist: boolean = true) {
  currentLocale = resolveLocale(code, Object.keys(locales));
  if (persist) writePersistedLocale(currentLocale);
}

export function listLocales(): { code: string; name: string }[] {
  return Object.keys(locales).map((code) => ({
    code,
    name: locales[code]["meta.localeName"] ?? code,
  }));
}

/** Translate a key, with optional `{placeholder}` interpolation. Unknown keys resolve to the
 * key itself (visibly broken but non-crashing), matching `app/i18n.js`'s original behavior. */
export function t(key: string, vars?: Record<string, string | number>): string {
  const strings = locales[currentLocale] ?? locales.en;
  const template = Object.hasOwn(strings, key)
    ? strings[key]
    : Object.hasOwn(locales.en, key)
      ? locales.en[key]
      : key;
  if (!vars) return template;
  return template.replace(/\{(\w+)\}/g, (match, name) =>
    name in vars ? String(vars[name]) : match
  );
}
