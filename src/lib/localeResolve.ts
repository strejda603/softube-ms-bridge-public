/** Pure locale-resolution logic, deliberately kept in its own plain `.ts` file rather than
 * `i18n.svelte.ts` -- that file uses the Svelte 5 `$state` rune, which only compiles via the
 * Svelte vite plugin. `vitest.config.ts` doesn't wire that plugin in (see its own comment), so
 * importing anything containing `$state` throws `ReferenceError: $state is not defined` at
 * *import* time, even for code paths that never touch the rune. Keeping this logic here keeps
 * it unit-testable without that constraint. */

export const LOCALE_STORAGE_KEY = "softubeMsBridge.locale";

/** Resolves a requested/persisted locale code against the known locale set, falling back to
 * "en" for anything unrecognized (including null/undefined, or an empty known list). Case-
 * insensitive, and falls back to the base language for a region-tagged code (e.g. "cs-CZ" ->
 * "cs"), matching the old Electron app's `app/i18n.js` behavior -- real OS locales are often
 * region-tagged and users don't reliably match case. */
export function resolveLocale(code: string | null | undefined, known: readonly string[]): string {
  if (code == null) return "en";
  const lower = code.toLowerCase();
  if (known.includes(lower)) return lower;
  const base = lower.split("-")[0];
  return known.includes(base) ? base : "en";
}

/** Like `resolveLocale`, but reports whether `code` was actually recognized (after the same
 * case/region normalization) rather than silently returning the "en" fallback -- lets a caller
 * distinguish "user explicitly requested en" from "code was invalid and fell back to en" before
 * deciding whether to apply an override at all. Used by App.svelte's --lang launch-override
 * handling so an unrecognized CLI value is ignored rather than silently forcing English over a
 * user's saved preference. */
export function isKnownLocale(code: string | null | undefined, known: readonly string[]): boolean {
  if (code == null) return false;
  const lower = code.toLowerCase();
  if (known.includes(lower)) return true;
  const base = lower.split("-")[0];
  return known.includes(base);
}
