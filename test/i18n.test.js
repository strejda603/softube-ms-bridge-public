const test = require("node:test");
const assert = require("node:assert/strict");
const {
  DEFAULT_LOCALE,
  listAvailableLocales,
  loadLocaleStrings,
  resolveLocale,
  createTranslator,
} = require("../app/i18n");

test("DEFAULT_LOCALE is en", () => {
  assert.equal(DEFAULT_LOCALE, "en");
});

test("listAvailableLocales includes en", () => {
  assert.ok(listAvailableLocales().includes("en"));
});

test("loadLocaleStrings('en') returns the English strings with known keys", () => {
  const strings = loadLocaleStrings("en");
  assert.equal(strings["app.title"], "Softube-MS-Bridge");
  assert.equal(strings["topbar.start"], "Start");
});

test("loadLocaleStrings falls back to English entirely for an unknown locale", () => {
  const en = loadLocaleStrings("en");
  const unknown = loadLocaleStrings("xx-not-a-real-locale");
  assert.deepEqual(unknown, en);
});

test("resolveLocale: unrecognized/unset locale falls back to en", () => {
  assert.equal(resolveLocale(undefined), "en");
  assert.equal(resolveLocale(""), "en");
  assert.equal(resolveLocale("xx-XX"), "en");
});

test("resolveLocale: exact match wins when available", () => {
  assert.equal(resolveLocale("en"), "en");
  assert.equal(resolveLocale("EN"), "en");
});

test("resolveLocale: falls back from region variant to base language if available, else en", () => {
  // No "en-GB.json" file exists, but the base "en" does — either way this must resolve, not throw.
  assert.equal(resolveLocale("en-GB"), "en");
});

test("createTranslator: known key resolves to its string", () => {
  const t = createTranslator({ greeting: "Hello" });
  assert.equal(t("greeting"), "Hello");
});

test("createTranslator: unknown key resolves to the key itself, not a crash", () => {
  const t = createTranslator({});
  assert.equal(t("does.not.exist"), "does.not.exist");
});

test("createTranslator: {placeholder} substitution", () => {
  const t = createTranslator({ greeting: "Hello, {name}!" });
  assert.equal(t("greeting", { name: "World" }), "Hello, World!");
});

test("createTranslator: missing placeholder var is left as-is rather than throwing", () => {
  const t = createTranslator({ greeting: "Hello, {name}!" });
  assert.equal(t("greeting", {}), "Hello, {name}!");
});

test("createTranslator: no vars arg on a template with no placeholders", () => {
  const t = createTranslator({ plain: "Just text" });
  assert.equal(t("plain"), "Just text");
});
