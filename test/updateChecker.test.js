const test = require("node:test");
const assert = require("node:assert/strict");
const { isNewerVersion } = require("../app/updateChecker");

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
