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

const PLATFORM_TAGS = { darwin: "mac", win32: "win", linux: "linux" };
const PLATFORM_EXTENSIONS = { darwin: ["dmg"], win32: ["exe"], linux: ["deb"] };

/**
 * Picks the release asset matching this platform + architecture, following the
 * `electron-builder` artifactName pattern used by this project's build.yml:
 * `${productName}-${version}-<platform>-<arch>.<ext>` (spaces rendered as dots).
 * @param {Array<{name: string, browserDownloadUrl: string}>} assets
 * @param {NodeJS.Platform} platform e.g. process.platform ("darwin"|"win32"|"linux")
 * @param {string} arch e.g. process.arch ("arm64"|"x64"|"ia32")
 * @returns {{name: string, browserDownloadUrl: string}|null}
 */
function pickReleaseAsset(assets, platform, arch) {
  const platformTag = PLATFORM_TAGS[platform];
  const extensions = PLATFORM_EXTENSIONS[platform];
  if (!platformTag || !extensions || !Array.isArray(assets)) return null;

  const candidates = assets.filter(
    (asset) =>
      asset &&
      typeof asset.name === "string" &&
      asset.name.includes(`-${platformTag}-`) &&
      extensions.some((ext) => asset.name.toLowerCase().endsWith(`.${ext}`))
  );

  const archMatch = candidates.find((asset) => asset.name.includes(`-${arch}.`));
  return archMatch || candidates[0] || null;
}

module.exports = { isNewerVersion, pickReleaseAsset };
