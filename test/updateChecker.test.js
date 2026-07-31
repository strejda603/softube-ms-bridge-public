const test = require("node:test");
const assert = require("node:assert/strict");
const { isNewerVersion, pickReleaseAsset } = require("../app/updateChecker");

test("isNewerVersion: minor bump is newer", () => {
  assert.equal(isNewerVersion("1.1.0", "1.2.0"), true);
});

test("isNewerVersion: older latest is not newer", () => {
  assert.equal(isNewerVersion("1.2.0", "1.1.0"), false);
});

test("isNewerVersion: equal versions are not newer", () => {
  assert.equal(isNewerVersion("1.1.0", "1.1.0"), false);
});

test("isNewerVersion: leading 'v' on latest is stripped", () => {
  assert.equal(isNewerVersion("1.1.0", "v1.2.0"), true);
});

test("isNewerVersion: patch bump is newer", () => {
  assert.equal(isNewerVersion("1.1.0", "1.1.1"), true);
});

test("isNewerVersion: integer comparison, not string comparison", () => {
  // "1.9.0" < "1.10.0" numerically, but ">" lexically as strings — must compare as integers.
  assert.equal(isNewerVersion("1.9.0", "1.10.0"), true);
  assert.equal(isNewerVersion("1.10.0", "1.9.0"), false);
});

test("isNewerVersion: malformed latest fails closed", () => {
  assert.equal(isNewerVersion("1.1.0", "not-a-version"), false);
});

test("isNewerVersion: malformed current fails closed", () => {
  assert.equal(isNewerVersion("not-a-version", "1.1.0"), false);
});

const MAC_WIN_LINUX_ASSETS = [
  { name: "Softube.Console.1.MS.Bridge-1.2.0-mac-arm64.dmg", browserDownloadUrl: "https://example.com/mac-arm64.dmg" },
  { name: "Softube.Console.1.MS.Bridge-1.2.0-mac-arm64.zip", browserDownloadUrl: "https://example.com/mac-arm64.zip" },
  { name: "Softube.Console.1.MS.Bridge-1.2.0-mac-x64.dmg", browserDownloadUrl: "https://example.com/mac-x64.dmg" },
  { name: "Softube.Console.1.MS.Bridge-1.2.0-mac-x64.zip", browserDownloadUrl: "https://example.com/mac-x64.zip" },
  { name: "Softube.Console.1.MS.Bridge-1.2.0-win-x64.exe", browserDownloadUrl: "https://example.com/win-x64.exe" },
  { name: "Softube.Console.1.MS.Bridge-1.2.0-linux-x64.deb", browserDownloadUrl: "https://example.com/linux-x64.deb" },
];

test("pickReleaseAsset: mac arm64 picks the arm64 dmg, not the zip", () => {
  const result = pickReleaseAsset(MAC_WIN_LINUX_ASSETS, "darwin", "arm64");
  assert.equal(result.name, "Softube.Console.1.MS.Bridge-1.2.0-mac-arm64.dmg");
});

test("pickReleaseAsset: mac x64 picks the x64 dmg", () => {
  const result = pickReleaseAsset(MAC_WIN_LINUX_ASSETS, "darwin", "x64");
  assert.equal(result.name, "Softube.Console.1.MS.Bridge-1.2.0-mac-x64.dmg");
});

test("pickReleaseAsset: win32 x64 picks the exe", () => {
  const result = pickReleaseAsset(MAC_WIN_LINUX_ASSETS, "win32", "x64");
  assert.equal(result.name, "Softube.Console.1.MS.Bridge-1.2.0-win-x64.exe");
});

test("pickReleaseAsset: linux x64 picks the deb", () => {
  const result = pickReleaseAsset(MAC_WIN_LINUX_ASSETS, "linux", "x64");
  assert.equal(result.name, "Softube.Console.1.MS.Bridge-1.2.0-linux-x64.deb");
});

test("pickReleaseAsset: no asset for platform returns null", () => {
  const macOnly = [MAC_WIN_LINUX_ASSETS[0]];
  assert.equal(pickReleaseAsset(macOnly, "linux", "x64"), null);
});

test("pickReleaseAsset: unsupported platform returns null", () => {
  assert.equal(pickReleaseAsset(MAC_WIN_LINUX_ASSETS, "freebsd", "x64"), null);
});

test("pickReleaseAsset: empty assets array returns null", () => {
  assert.equal(pickReleaseAsset([], "darwin", "arm64"), null);
});
