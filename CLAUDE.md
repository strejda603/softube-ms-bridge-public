# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Rust bridge connecting the Softube Console 1 Fader Mk III (hardware control surface,
talks SysEx MIDI) to Mixing Station (mixing console software, talks a JSON-over-WebSocket
API). It mirrors track state both ways: fader/mute/solo/pan/selection/sends from Console 1
→ Mixing Station, and channel data from Mixing Station → Console 1's on-screen display.
Ships as a headless CLI binary (`bridge-cli`) and a Tauri v2 + Svelte desktop GUI.

## Commands

```bash
# Rust workspace (crates/, src-tauri/)
cargo build --workspace          # build everything
cargo test --workspace           # run every crate's test suite
cargo test --package bridge-core # run just one crate
cargo run --package bridge-cli   # run the bridge headless

# Frontend (src/, the Svelte app the Tauri GUI serves)
npm install                      # frontend + Tauri CLI deps
npm run tauri dev                # Tauri desktop GUI, dev mode (hot reload)
npm run tauri build              # packaged app bundle (.app/.dmg on macOS, .exe/.msi on Windows)
npm run check                    # svelte-check + tsc, no emit
npx vitest run                   # frontend unit tests (or: npm run test:frontend)
npm test                         # frontend tests + full cargo test --workspace

# Dev tooling (Node.js scripts, talk directly to Mixing Station or a fake WS server —
# not tied to either bridge binary)
npm run repl                     # tools/ws-repl.js — interactive Mixing Station WS client
npm run fake:meters              # tools/fake-metering-proxy.js — simulate metering2 traffic
npm run fake:meters:proxy        # same, but proxies a real Mixing Station on 8080 while injecting fake meters on 8081
npm run fake:x32                 # tools/fake-metering-proxy.js --x32 — X32 OSC/UDP meter emulator
```

No lint script exists for Rust beyond `cargo clippy` (not wired into a script — run directly:
`cargo clippy --workspace --all-targets`). No lint/typecheck exists for the dev tooling under
`tools/` — plain JS, `node --check <file>.js` catches syntax errors only.

## Architecture

### Workspace layout — three Rust crates + one Tauri shell + one Svelte frontend

- **`crates/bridge-core`** — the actual bridge logic, a Rust port of what used to be a single
  `index.js` file. Pure, side-effect-free logic lives in small sibling modules (`value_coercion.rs`,
  `pan_utils.rs`, `midi_color_utils.rs`, `metering_utils.rs`, `metering2_message.rs`,
  `console1_status_bank.rs`, `track_layout.rs`, `sysex.rs`, `send_mapping.rs`,
  `dsp_field_metadata.rs`, `control_messages.rs`, `sends_mode.rs`, `ms_param_apply.rs`,
  `channel_data_message.rs`, `echo_suppression.rs`, `track_cache.rs`, `track_id.rs`,
  `console_information.rs`, `status_monitor.rs`, `update_queue.rs`, `bare_update_queue.rs`,
  `ms_write_queue.rs`, `midi_mixer_dispatch.rs`, `midi_dsp_dispatch.rs`), each with a real
  `cargo test` suite. `runtime.rs` is the reusable, command-driven, event-emitting async engine
  that ties all of this together — it's what both `bridge-cli` and `src-tauri` spawn and drive;
  the orchestration logic itself doesn't change shape based on which caller is driving it.
  `midi_io.rs`/`ws_engine.rs` are the actual MIDI/WebSocket I/O engines.
- **`crates/bridge-cli`** — thin headless binary (`main.rs`). Loads `bridge-config.json` (see
  below), spawns `bridge_core::runtime`, prints its events to stdout, forwards Ctrl+C as a clean
  shutdown. Deliberately thin — almost no logic of its own, matching the old `index.js`'s "no
  exports, no unit tests of its own" convention: the untested surface here is glue, not decision
  logic.
- **`crates/bridge-config`** — `RuntimeConfig`/`BridgeConfigPatch`, config file load/save/patch
  application, shared by both `bridge-cli` and `src-tauri`.
- **`src-tauri`** — the Tauri v2 desktop shell (`lib.rs`, `main.rs`). Spawns
  `bridge_core::runtime` as a managed background task, exposes it to the Svelte webview via
  `#[tauri::command]`s and `Emitter::emit` events, persists config via Tauri's own
  `app_config_dir()`, manages saved presets (`presets.rs`), polls MIDI-port/process-presence
  status for the topbar's 7 status dots (`status_gather.rs`), parses the `--lang` launch flag
  (`cli_args.rs`), and installs graceful shutdown for both window-close and Ctrl+C/SIGINT.
- **`src/`** — the Svelte 5 (runes) + TypeScript frontend the Tauri GUI serves: app shell,
  Connection/Track-Layout/Sends-&-Colors/Presets tabs, a collapsible log drawer, i18n
  (`src/lib/i18n.svelte.ts` + `src/locales/{en,cs}.json`), status dots, settings popover.

### The SysEx wire protocol (Console 1 side)

Console 1 SysEx frames are `0xF0 0x7D "stc1" <JSON bytes> 0xF7` — the entire payload is a
JSON object encoded byte-for-byte (see `sysex.rs`'s `SYSEX_MAGIC`/`parse_sysex_json`/
`build_sysex_frame`). Mixer primitives (`volume`, `mute`, `pan`, `send1..6`) are bare values;
DSP-section fields are `{value: ...}`-wrapped. `docs/softube_common.js` (Softube's own Cubase
MIDI Remote driver script, same protocol) is the reference for field names Console 1 can
send/receive — useful when extending coverage, but verify against real hardware before trusting
it, per the note below.

**Hard constraint discovered the hard way**: this specific Fader Mk III unit's firmware
rejects the **entire** outbound `trackBatch` SysEx message if it contains *any* field name
outside its fixed mixer schema (`track`, `isActive`, `trackId`, `color`, `name`, `volume`,
`meter`, `mute`, `solo`, `selected`, `maxVolumeValue`, `maxSendValue`, `pan`,
`send1..6`/`send1..6On`) — independent of message size or value shape (confirmed via
direct hardware A/B testing). `sysex.rs`'s `ConsoleTrackFields` type is the enforced
allowlist guard: it's structurally incapable of holding a disallowed field, so a
`From<&TrackInfo> for ConsoleTrackFields` conversion is the one place that decides what's
safe to put on the wire — a future `TrackInfo` field rename can't silently reintroduce a
disallowed field without a compile error. All outbound SysEx goes through `runtime.rs`'s
`send_sysex_and_log` choke point. Adding new outbound fields to Console 1 needs real-hardware
verification, not just reading the driver script — the driver targets the whole Console 1
product family, not necessarily what this specific SKU's firmware parses.

### Mixing Station side

WS API is JSON request/response + push: writes go to `/console/data/set/<path>/<format>`
(`format` is `"val"` for real units or `"norm"` for 0..1 normalized), reads are
subscribe/push via `/console/data/subscribe` and inbound `/console/data/get/<path>/<format>`
messages. `docs/ms-apidoc.md`, `docs/console-data-paths.json`, and `docs/ws-data-dump.jsonl`
(captured real traffic) are the reference for available paths.

### Track layout, sends, lifecycle

Console 1 Fader Mk III has 10 physical faders, so tracks are banked 10-wide
(`track_layout.rs`'s `build_track_layout`). `input_track_order`/`bus_track_order`
(`RuntimeConfig` fields, optionally `[left, right]` stereo pairs) drive ordering; Main is
placed once on the last bus bank's 10th fader. "Sends mode" is a selection-driven remapping
(`sends_mode.rs`) — selecting a bus master temporarily repoints Console 1's mute/pan onto
that bus's send slot. Console 1's 6 physical sends map onto Mixing Station's up-to-16 bus
sends via `c1_send_to_ms_bus_number` (`send_mapping.rs`). Bridge lifecycle is a 2-state
machine (`Standby`/`Running`, `console1_status_bank::Lifecycle`) tracked in `runtime.rs`,
separate from the unrelated Standard/Sends mode concept — don't conflate the two when reading
logs (`[Lifecycle]` vs `[Mode]` prefixes).

### Config and persistence

`bridge-cli` reads `bridge-config.json` (path overridable via `BRIDGE_CONFIG_PATH` env var) at
startup via `bridge_config::load_config_file`, with `MIXING_STATION_WS_URL`/`LOG_JSON`
environment variables taking precedence over the file when set (env > file > hardcoded
defaults — see `crates/bridge-cli/src/main.rs`'s `build_startup_config`). `src-tauri` persists
its own config file under Tauri's `app_config_dir()` and re-applies patches live without
restart via the `config_apply` Tauri command (`apply_config_patch` in `src-tauri/src/lib.rs`).
GUI presets are separate saved config snapshots (`src-tauri/src/presets.rs`), round-tripped
through Tauri commands.

## Testing notes

- Pure logic in `crates/bridge-core/src/*.rs` gets real `cargo test` unit tests — the
  established convention throughout this migration is to extract testable decision logic into
  small pure functions (taking already-resolved inputs, no direct `std::env`/`std::fs`/network
  I/O) and keep thin async-engine glue (`runtime.rs`'s dispatch wiring, `bridge-cli`'s/
  `src-tauri`'s `main()`/`run()`) untested by design — matching the old `index.js`'s own
  "no unit tests of its own, deliberate not a gap" convention. When adding new bridge logic
  that's testable in isolation, follow this split rather than adding untested logic directly
  into `runtime.rs`.
- `src/lib/*.ts` gets real `vitest` unit tests (`*.test.ts` siblings) for pure TypeScript
  modules; Svelte components themselves have no automated test harness in this repo.
- There is no automated harness for simulating real Console 1 SysEx or a live Mixing Station
  WS server — verifying wire-level/hardware behavior is a manual process: run `bridge-cli` or
  the Tauri GUI against real hardware, with `LOG_JSON=1` (env var or `bridge-config.json`'s
  `logJson`) to see every WS/MIDI JSON payload logged. `tools/fake-metering-proxy.js` can
  simulate Mixing Station's `metering2` push traffic against either real binary for testing
  the metering path without a live Mixing Station session.
