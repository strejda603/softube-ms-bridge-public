const test = require("node:test");
const assert = require("node:assert/strict");
const { parseCliArgs, getUserArgv } = require("../app/cliArgs");

test("no args: all fields default, no warnings", () => {
  const result = parseCliArgs([]);
  assert.deepEqual(result, {
    start: false,
    stop: false,
    preset: null,
    ws: null,
    interval: null,
    log: false,
    verbose: false,
    warnings: [],
  });
});

test("--start sets start", () => {
  const result = parseCliArgs(["--start"]);
  assert.equal(result.start, true);
  assert.deepEqual(result.warnings, []);
});

test("--stop sets stop", () => {
  const result = parseCliArgs(["--stop"]);
  assert.equal(result.stop, true);
});

test("--start and --stop together: both cleared, warning added", () => {
  const result = parseCliArgs(["--start", "--stop"]);
  assert.equal(result.start, false);
  assert.equal(result.stop, false);
  assert.equal(result.warnings.length, 1);
  assert.match(result.warnings[0], /--start and --stop/);
});

test("--preset consumes the next token", () => {
  const result = parseCliArgs(["--preset", "My Show"]);
  assert.equal(result.preset, "My Show");
  assert.deepEqual(result.warnings, []);
});

test("--preset with no value warns and leaves preset null", () => {
  const result = parseCliArgs(["--preset"]);
  assert.equal(result.preset, null);
  assert.equal(result.warnings.length, 1);
  assert.match(result.warnings[0], /--preset requires a value/);
});

test("--preset followed by another flag warns instead of consuming it", () => {
  const result = parseCliArgs(["--preset", "--start"]);
  assert.equal(result.preset, null);
  assert.equal(result.start, true);
  assert.equal(result.warnings.length, 1);
});

test("--ws consumes the next token", () => {
  const result = parseCliArgs(["--ws", "localhost:8080"]);
  assert.equal(result.ws, "localhost:8080");
});

test("--interval parses a numeric value", () => {
  const result = parseCliArgs(["--interval", "50"]);
  assert.equal(result.interval, 50);
});

test("--interval with a non-numeric value warns and leaves interval null", () => {
  const result = parseCliArgs(["--interval", "fast"]);
  assert.equal(result.interval, null);
  assert.equal(result.warnings.length, 1);
  assert.match(result.warnings[0], /--interval requires a numeric value/);
});

test("--log and --verbose set their flags", () => {
  const result = parseCliArgs(["--log", "--verbose"]);
  assert.equal(result.log, true);
  assert.equal(result.verbose, true);
});

test("unknown flag warns but does not throw", () => {
  const result = parseCliArgs(["--bogus"]);
  assert.equal(result.warnings.length, 1);
  assert.match(result.warnings[0], /Unknown argument: --bogus/);
});

test("combined realistic invocation", () => {
  const result = parseCliArgs([
    "--preset",
    "Show A",
    "--ws",
    "localhost:9090",
    "--interval",
    "75",
    "--log",
    "--start",
  ]);
  assert.equal(result.preset, "Show A");
  assert.equal(result.ws, "localhost:9090");
  assert.equal(result.interval, 75);
  assert.equal(result.log, true);
  assert.equal(result.start, true);
  assert.deepEqual(result.warnings, []);
});

test("getUserArgv: packaged app drops only the executable path", () => {
  const argv = ["/Applications/Bridge.app/Contents/MacOS/Bridge", "--start"];
  assert.deepEqual(getUserArgv(argv, true), ["--start"]);
});

test("getUserArgv: dev (unpackaged) drops executable + script path", () => {
  const argv = ["/usr/local/bin/electron", "app/main.js", "--start"];
  assert.deepEqual(getUserArgv(argv, false), ["--start"]);
});
