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
  // Prefer a same-platform installer over no download at all if the exact arch isn't listed.
  return archMatch || candidates[0] || null;
}

/**
 * Fetches the latest published release for a `owner/repo` GitHub repository.
 * Electron 43 bundles a Node runtime with a global `fetch`, so no HTTP dependency
 * is needed here.
 * @param {string} repo e.g. "strejda603/softube-ms-bridge-public"
 * @returns {Promise<{tagName: string, htmlUrl: string, assets: Array<{name: string, browserDownloadUrl: string}>}>}
 * @throws {Error} if the request fails or the API responds with a non-OK status
 */
async function fetchLatestRelease(repo) {
  const res = await fetch(`https://api.github.com/repos/${repo}/releases/latest`, {
    headers: { Accept: "application/vnd.github+json" },
  });
  if (!res.ok) {
    throw new Error(`GitHub API returned ${res.status} for ${repo}`);
  }
  const json = await res.json();
  return {
    tagName: json.tag_name,
    htmlUrl: json.html_url,
    assets: Array.isArray(json.assets)
      ? json.assets.map((asset) => ({ name: asset.name, browserDownloadUrl: asset.browser_download_url }))
      : [],
  };
}

module.exports = { isNewerVersion, pickReleaseAsset, fetchLatestRelease };
