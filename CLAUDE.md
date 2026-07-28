# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Node.js bridge connecting the Softube Console 1 Fader Mk III (hardware control surface,
talks SysEx MIDI) to Mixing Station (mixing console software, talks a JSON-over-WebSocket
API). It mirrors track state both ways: fader/mute/solo/pan/selection/sends from Console 1
→ Mixing Station, and channel data from Mixing Station → Console 1's on-screen display.
Ships as both a headless CLI process and an Electron GUI wrapper.

## Commands

```bash
npm install              # deps (native @julusian/midi module, prebuilt binaries)
npm test                 # node --test "test/**/*.test.js" — run entire suite
node --test test/valueCoercion.test.js   # run a single test file
npm start                # run the bridge headless (node index.js)
npm run gui               # Electron GUI (settings + presets + live logs)
npm run gui:verbose       # GUI, but bridge logs print to the launching terminal
npm run repl              # tools/ws-repl.js — interactive Mixing Station WS client
npm run fake:meters       # tools/fake-metering-proxy.js — simulate metering2 traffic
npm run build:mac         # electron-builder --mac → dist/
```

No lint/typecheck script exists — this is plain JS with JSDoc annotations, not TypeScript.
`node --check <file>.js` catches syntax errors only (not runtime issues).

Debug env vars (set in shell or `bridge-config.json`): `LOG_JSON=1` logs every WS/MIDI JSON
payload; `LOG_METERING=1` logs metering parsing/subscription details.

## Architecture

### Two runtime entry points, one core

- **`index.js`** — the actual bridge. Self-executing: `require`-ing or running it
  immediately opens MIDI ports and starts connecting. It has **no `module.exports`**
  and **no unit tests of its own** — this is deliberate, not a gap. All MIDI/WS handler
  logic lives here as one large file, organized into `// --- SECTION --- ` banner blocks
  (grep `^// #` for the section map — CONSTANTS, TRACK LAYOUT, METERING, MS WRITES,
  BRIDGE LIFECYCLE, C1 MIDI MSG HANDLING, SYSEX TO C1, etc.).
- **`app/main.js`** — Electron main process. Spawns `index.js` as a plain Node child
  process (`ELECTRON_RUN_AS_NODE=1`), starting it in a `"standby"` lifecycle state, and
  controls it over a newline-delimited-JSON **stdin** channel (`{"type":"lifecycle:start",
  "config":{...}}` / `"lifecycle:stop"` / `"config:apply"`) rather than killing/respawning
  it on every Start/Stop. The bridge process talks back to `app/main.js` over stdout using
  `@@BRIDGE_EVENT@@{...json...}` prefixed lines.
- Pure, side-effect-free logic that needs unit tests is deliberately extracted into small
  sibling modules and `require()`'d into `index.js`: `valueCoercion.js`, `panUtils.js`,
  `midiColorUtils.js`, `meteringUtils.js`, `console1StatusBank.js`. Each has a matching
  `test/*.test.js` (flat `node:test` `test("name", ...)` calls, not `describe`/`it`).
  When adding new bridge logic that's testable in isolation, follow this split rather
  than adding untested logic directly to `index.js`.

### Electron preload is sandboxed — no local `require()`

`app/preload.js` runs in Electron's default sandbox (v20+): `require()` there only
resolves a small built-in whitelist, **not** local project files. Anything preload needs
from a local module (e.g. i18n strings) must be computed in `app/main.js` and handed over
via synchronous IPC (`ipcRenderer.sendSync` / `ipcMain.on(...) { event.returnValue = ... }`)
or inlined directly in `preload.js`. A `require("./something")` in preload doesn't throw a
visible error to a normal user — it silently kills the *entire* preload script, so
`window.bridge`/`window.presets`/etc. never get exposed and the whole renderer breaks.

### The SysEx wire protocol (Console 1 side)

Console 1 SysEx frames are `0xF0 0x7D "stc1" <JSON bytes> 0xF7` — the entire payload is a
JSON object encoded byte-for-byte (see `SYSEX_MAGIC`/`parseSysexJson` in `index.js`).
Mixer primitives (`volume`, `mute`, `pan`, `send1..6`) are bare values; DSP-section fields
are `{value: ...}`-wrapped. `docs/softube_common.js` (Softube's own Cubase MIDI Remote
driver script, same protocol) is the reference for field names Console 1 can send/receive
— useful when extending coverage, but verify against real hardware before trusting it, per
the note below.

**Hard constraint discovered the hard way**: this specific Fader Mk III unit's firmware
rejects the **entire** outbound `trackBatch` SysEx message if it contains *any* field name
outside its fixed mixer schema (`track`, `isActive`, `trackId`, `color`, `name`, `volume`,
`meter`, `mute`, `solo`, `selected`, `maxVolumeValue`, `maxSendValue`, `pan`,
`send1..6`/`send1..6On`) — independent of message size or value shape (confirmed via
direct hardware A/B testing). `CONSOLE1_OUTBOUND_UNSUPPORTED_FIELDS` in `index.js` is the
enforced allowlist-violation guard, applied centrally in `sendSysexToConsole1`'s
`JSON.stringify` replacer so no call site can accidentally reintroduce a message that
silently blanks every track on the hardware. Adding new outbound fields to Console 1 needs
real-hardware verification, not just reading the driver script — the driver targets the
whole Console 1 product family, not necessarily what this specific SKU's firmware parses.

### Mixing Station side

WS API is JSON request/response + push: writes go to `/console/data/set/<path>/<format>`
(`format` is `"val"` for real units or `"norm"` for 0..1 normalized), reads are
subscribe/push via `/console/data/subscribe` and inbound `/console/data/get/<path>/<format>`
messages. `docs/ms-apidoc.md`, `docs/console-data-paths.json`, and `docs/ws-data-dump.jsonl`
(captured real traffic) are the reference for available paths.

### Track layout, sends, lifecycle

Console 1 Fader Mk III has 10 physical faders, so tracks are banked 10-wide (`rebuildTrackLayout`
in `index.js`). `INPUT_TRACK_ORDER`/`BUS_TRACK_ORDER` (arrays, optionally `[left,right]` stereo
pairs) drive ordering; Main is placed once on the last bus bank's 10th fader. "Sends mode" is
a selection-driven remapping — selecting a bus master temporarily repoints Console 1's mute/pan
onto that bus's send slot. Console 1's 6 physical sends map onto Mixing Station's up-to-16 bus
sends via `C1_SEND_TO_MS_BUS_NUMBER`. Bridge lifecycle is a 2-state machine
(`"standby"`/`"running"`) tracked in `index.js`, separate from the unrelated Standard/Sends
mode concept — don't conflate the two when reading logs (`[Lifecycle]` vs `[Mode]` prefixes).

### Config and persistence

Runtime config lives in `bridge-config.json` (path overridable via `BRIDGE_CONFIG_PATH` env
var), loaded once at startup (`loadBridgeConfig`) and re-appliable live without restart via
the `config:apply` stdin message (`applyRuntimeConfig`). GUI presets are separate saved
config snapshots, round-tripped through `app/main.js`'s IPC handlers.

## Testing notes

- `index.js`'s MIDI/WS handlers are verified via `node --check` (syntax) plus manual runs
  against real hardware — there is no automated harness for simulating Console 1 SysEx or
  a Mixing Station WS server. When you need to verify wire-level behavior, the working
  pattern is: spawn `node index.js` with `LOG_JSON=1`, feed it `{"type":"lifecycle:start",
  "config":{}}\n` over stdin to trigger the handshake, and read its stdout log — don't
  guess at protocol behavior from the driver script alone.
- Everything else (small sibling modules) gets real `node:test` unit tests; follow the
  existing flat `test("moduleFn: scenario description", () => {...})` naming convention.
