"use strict";

/**
 * Parses a "major.minor.patch" version string into a 3-tuple of integers.
 * Returns null for anything that doesn't cleanly parse (caller treats that as
 * "not newer" rather than risking a false update prompt).
 * @param {string} version
 * @returns {[number, number, number]|null}
 */
function parseVersion(version) {
  const cleaned = String(version || "").replace(/^v/i, "");
  const parts = cleaned.split(".").map((part) => Number.parseInt(part, 10));
  if (parts.length !== 3 || parts.some((n) => Number.isNaN(n))) return null;
  return /** @type {[number, number, number]} */ (parts);
}

/**
 * @param {string} current e.g. "1.1.0" (from package.json)
 * @param {string} latest e.g. "v1.2.0" or "1.2.0" (from a GitHub release tag_name)
 * @returns {boolean} true only if `latest` is a strictly greater major.minor.patch than `current`
 */
function isNewerVersion(current, latest) {
  const a = parseVersion(current);
  const b = parseVersion(latest);
  if (!a || !b) return false;

  for (let i = 0; i < 3; i++) {
    if (b[i] > a[i]) return true;
    if (b[i] < a[i]) return false;
  }
  return false;
}

module.exports = { isNewerVersion };
