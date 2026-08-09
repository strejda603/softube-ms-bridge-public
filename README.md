# Softube Console 1 Fader MK III ↔ Mixing Station Bridge

[![Version](https://img.shields.io/badge/version-2.0.0-blue)](https://github.com/strejda603/softube-ms-bridge-public/releases/latest)
[![License](https://img.shields.io/badge/license-GPL--3.0-blue)](https://github.com/strejda603/softube-ms-bridge-public/blob/main/LICENSE)
![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey)
[![Rust](https://img.shields.io/badge/rust-stable-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![GUI](https://img.shields.io/badge/GUI-Tauri%20%2B%20Svelte-24C8DB?logo=tauri&logoColor=white)](https://tauri.app/)
[![Ko-fi](https://img.shields.io/badge/support-Ko--fi-FF5E5B?logo=ko-fi&logoColor=white)](https://ko-fi.com/K3N223V22V)

Rust bridge that connects Softube Console 1 Fader MK III (over SysEx MIDI) with the Mixing Station WebSocket API. Ships as a headless CLI (`bridge-cli`) and a Tauri desktop GUI (settings, presets, live logs).

It mirrors track state (name/color/volume/mute/solo/pan/selection + meters) and translates Console 1 controls into Mixing Station writes.

## Table of Contents

- [Requirements](#requirements)
- [Quick start (headless CLI)](#quick-start-headless-cli)
- [Desktop GUI](#desktop-gui)
- [What it supports](#what-it-supports)
- [Building the desktop app](#building-the-desktop-app)
- [Localization](#localization)
- [Track layout (banks + ordering)](#track-layout-banks--ordering)
- [Sends](#sends)
- [Metering](#metering)
- [Bridge lifecycle (GUI mode)](#bridge-lifecycle-gui-mode)
- [Shutdown behavior](#shutdown-behavior)
- [Troubleshooting](#troubleshooting)
- [Support](#support)

## Requirements

- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain, via `rustup`) — needed to build/run either binary
- [Node.js](https://nodejs.org/en) (current LTS) — only needed to build the Svelte frontend for the desktop GUI, not for the headless CLI
- Mixing Station with WebSocket API enabled/reachable (default is `ws://localhost:8080`)
- Softube On-Screen Display app installed
- Softube Console 1 Fader Mk III connected

## Quick start (headless CLI)

1. Build and run the headless bridge:

   `cargo run --package bridge-cli`

2. Confirm in the logs:

   - It opens the Console 1 MIDI input/output ports
   - It connects to Mixing Station WebSocket

Configuration is read from `bridge-config.json` in the current directory (override the path with
the `BRIDGE_CONFIG_PATH` environment variable), with `MIXING_STATION_WS_URL` and `LOG_JSON`
environment variables taking precedence over the file when set. See
[`crates/bridge-config`](crates/bridge-config) for every supported field.

## Desktop GUI

The desktop app is built with [Tauri](https://tauri.app/) v2 (Rust shell) and
[Svelte](https://svelte.dev/) 5 (frontend) — settings, presets, and live logs, all in one window.

- Run in development mode:

  `npm install && npm run tauri dev`

- Run a built app: see [Building the desktop app](#building-the-desktop-app) below.

### Command-line arguments

All CLI flags are applied once, at launch, after the GUI's initial config load — same as the
old Electron launcher, minus its second-instance-forwarding behavior (running the app a second
time opens a second window rather than forwarding flags to the first, since no single-instance
lock is set up in the Tauri app).

| Flag | Effect |
|---|---|
| `--lang <code>` | Override the UI language for this launch only (e.g. `--lang cs`) — doesn't overwrite your saved language preference |
| `--preset <name>` | Load a saved preset by name (case-insensitive) before applying any other flag below |
| `--ws <url>` | Override the Mixing Station WebSocket URL for this launch (`ws://`/`wss://` prefix added automatically if omitted) |
| `--interval <ms>` | Override the metering interval for this launch (clamped to 30–1000ms) |
| `--log` | Enable JSON WS/MIDI logging for this launch |
| `--start` | Start the bridge automatically once the GUI has finished loading |
| `--stop` | Stop the bridge automatically once the GUI has finished loading (a no-op on a fresh launch, since nothing is running yet) |

`--start` and `--stop` given together cancel each other out (neither applies). `--preset`,
`--ws`, `--interval`, and `--log` are applied on top of the loaded preset (or the existing
config, if no preset matched), then `--start`/`--stop` runs last — a failed `--preset` load
suppresses `--start` so the bridge doesn't launch against an unintended config.

## What it supports

- Track layout with custom ordering (inputs + buses + main), banked 10-wide for Console 1
- Track state mirroring: name/color, fader, mute, solo, pan, selection
- Sends mode: selecting a Bus master temporarily maps mute/pan to the chosen bus send
- Metering: Console 1 active meter requests → Mixing Station `metering2` (hardware-verified)
- Stability features: reconnect + init buffering + batching to avoid WS/SysEx spam

Note: Bus master solo is intentionally ignored (both directions), to avoid surprising “solo the whole bus” behavior.

## Building the desktop app

Build a standalone app for your platform with the Tauri CLI:

1. Install deps:

   `npm install`

2. Build for your platform:

   `npm run tauri build`

Tauri writes platform-native bundles (`.app`/`.dmg` on macOS, `.exe`/`.msi` on Windows) under
`src-tauri/target/release/bundle/`. See the [Tauri distribution docs](https://tauri.app/distribute/)
for the exact artifact set on your platform.

## Localization

The GUI is translatable — English (`src/locales/en.json`) and Czech (`src/locales/cs.json`) ship
today. A gear icon in the topbar opens a settings popover with the language selector; switching it
takes effect immediately — no restart needed — and the choice is remembered (via `localStorage`)
for next launch. First launch (before any choice is saved) defaults to English; use `--lang <code>`
(see [Command-line arguments](#command-line-arguments)) to override it for one session without
changing the saved preference.

To add a language:

1. Copy `src/locales/en.json` to `src/locales/<code>.json` (e.g. `de.json`) and translate the
   values — keep the keys and any `{placeholder}` tokens exactly as-is. Set `meta.localeName`
   to how the language should read in its own selector entry (e.g. `"Deutsch"`, not `"German"`).
2. Register the new locale in `src/lib/i18n.svelte.ts`'s `locales` map.
3. The new locale then shows up in the settings popover's language selector automatically.

The bridge's own console/log output is technical/debug text, not user-facing UI, and is
intentionally out of scope — it stays English-only, same as most server/daemon logs.

## Track layout (banks + ordering)

Console 1 Fader Mk III has 10 faders, so the bridge builds 10-wide banks.

- Input banks: based on `inputTrackOrder` (supports stereo groups).
- Bus banks: based on `busTrackOrder` (supports stereo groups).
- Main: placed only once (10th fader on the last bus bank).

Stereo groups are expressed as `[left, right]` pairs inside the order arrays. For these grouped stereo tracks:

- Pan is locked (Console 1 pan changes are ignored)
- Displayed pan is forced to center

Edit these in the GUI's **Track Layout** tab (drag-and-drop reordering + stereo linking), or
directly in `bridge-config.json`:

- `inputTrackOrder`
- `busTrackOrder`

Channel ranges are auto-detected from Mixing Station on connect (via `/console/information`),
falling back to these defaults if unavailable:

- Inputs: `ch.0 .. ch.31` (32 inputs)
- Buses: `ch.48 .. ch.63` (16 buses)
- Main stereo: `ch.70 + ch.71`

## Sends

Console 1 exposes 6 send slots, while Mixing Station can have up to 16 bus sends.

### Send slot mapping

Edit `c1SendToMsBusNumber` in `bridge-config.json`, or the GUI's **Sends & Colors** tab:

- It maps Console 1 send slots (1..6) to Mixing Station bus numbers (1..16)
- Example: `Send4 → Bus7`, `Send5 → Bus9`, `Send6 → Bus13`

This mapping controls how Console 1 `send1..send6` and `send1On..send6On` are written to Mixing Station:

- `ch.<input>.mix.sends.<msSendIndex>.lvl`
- `ch.<input>.mix.sends.<msSendIndex>.on`

### Sends mode (selection-driven)

The bridge also implements a special “sends mode”, latched by selection:

- Select a Bus master → enters sends mode for that bus (active send index)
- Select Main → exits sends mode back to standard behavior

In sends mode, for input tracks only:

- Console 1 `mute` controls `mix.sends.<active>.on` (inverted semantics)
- Console 1 `pan` controls `mix.sends.<active>.pan`
- Volume is NOT remapped (still controls channel `mix.lvl`), so you need to press `send1..send6` button to control it!

When the mode changes, the bridge logs it:

- `[Mode] SENDS active (msSendIndex=…, bus=…)`
- `[Mode] STANDARD active`

## Metering

The bridge supports Console 1 meter requests (`activeMeters`) using Mixing Station `metering2`.
It converts dB values into Console 1’s expected 0..1 peak-like meter value.

## Bridge lifecycle (GUI mode)

When run via the GUI, the bridge runs as an in-process background task started when the app
launches and stays alive (holding the Console 1 Fader MIDI connection) for the life of the app —
it doesn't restart on every Start/Stop. Start/Stop instead toggle between two states:

- **Standby**: Console 1 Fader connected, no Mixing Station connection.
- **Running**: full bridging active.

This edition has no hardware Status Bank or physical Start/Stop trigger on the Console 1
Fader — Start/Stop is GUI-only, via the topbar buttons. The GUI's topbar shows 2 status
dots (Mixing Station, Console 1 On-Screen Display), green when present and red when not,
updating live roughly every 2 seconds.

## Shutdown behavior

On closing the GUI window, or on Ctrl+C (SIGINT) or `SIGTERM` (a plain `kill <pid>`) in either
binary:

- Sends a Console 1 `RESET`
- Deactivates all tracks on Console 1
- Closes MIDI ports and the WebSocket

`SIGTERM` handling is macOS/Linux-only — Windows has no equivalent signal a process can trap, so
`kill`-style termination there still ends the process immediately.

## Troubleshooting

- “MIDI port not found”:
  - The bridge looks for ports containing `Console 1 Fader Mk III DAW`.
  - If your system reports a different name, adjust `DEFAULT_PREFERRED_PORT_NAMES` in
    [`crates/bridge-core/src/midi_io.rs`](crates/bridge-core/src/midi_io.rs).

- WebSocket won’t connect:
  - Verify Mixing Station WebSocket API is enabled.
  - Check `mixingStationWsUrl` in `bridge-config.json` (or the GUI's Connection tab) and
    network/firewall settings.

- Can’t exit sends mode:
  - Main is only placed once (10th fader on the last bus bank). Select Main to return to standard mode.

- Need more visibility:
  - Set `logJson: true` in `bridge-config.json` (or toggle it in the GUI's Connection tab) to log
    WS/MIDI JSON payloads. There's currently no separate metering-only debug log level (JS's old
    `LOG_METERING` toggle hasn't been ported).

- A topbar status dot (Mixing Station/Console 1 On-Screen Display) never turns green
  despite the app being present:
  - The match string in [`src-tauri/src/status_gather.rs`](src-tauri/src/status_gather.rs) likely
    doesn't match what your system actually reports. Check the exact process name via
    `ps -Ao args= | grep -i <app name>`, and adjust the corresponding match string there.

## Support

If this bridge saves you from buying dedicated hardware, consider supporting continued
development on [Ko-fi](https://ko-fi.com/strejda603).

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/K3N223V22V)
