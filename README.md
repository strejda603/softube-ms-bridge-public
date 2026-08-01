# Softube Console 1 Fader MK III ↔ Mixing Station Bridge

[![Version](https://img.shields.io/badge/version-1.2.0-blue)](https://github.com/strejda603/softube-ms-bridge-public/releases/latest)
[![License](https://img.shields.io/badge/license-GPL--3.0-blue)](https://github.com/strejda603/softube-ms-bridge-public/blob/main/LICENSE)
![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey)
[![Node.js](https://img.shields.io/badge/node-LTS-339933?logo=node.js&logoColor=white)](https://nodejs.org/en)
[![GUI](https://img.shields.io/badge/GUI-Electron-47848F?logo=electron&logoColor=white)](https://www.electronjs.org/)
[![Ko-fi](https://img.shields.io/badge/support-Ko--fi-FF5E5B?logo=ko-fi&logoColor=white)](https://ko-fi.com/K3N223V22V)

Node.js bridge that connects Softube Console 1 Fader MK III (over SysEx MIDI) with the Mixing Station WebSocket API.

It mirrors track state (name/color/volume/mute/solo/pan/selection + meters) and translates Console 1 controls into Mixing Station writes.

## Table of Contents

- [Requirements](#requirements)
- [Quick start (CLI)](#quick-start-cli)
- [GUI launcher](#gui-launcher)
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

- Node.js (recommended: current LTS)
- Mixing Station with WebSocket API enabled/reachable (default is `ws://localhost:8080`)
- Softube On-Screen Display app installed
- Softube Console 1 Fader Mk III connected

## Quick start (CLI)

1. Install dependencies:

   `npm install`

2. Start the bridge:

   `npm start`

3. Confirm in the logs:

   - It opens the Console 1 MIDI input/output ports
   - It connects to Mixing Station WebSocket

## GUI launcher

This repo includes an Electron-based GUI launcher (settings + presets + live logs).

Settings include Mixing Station WS URL, metering interval, layout, send mapping, and colors.

- Run the GUI:

  `npm run gui`

### Verbose GUI mode

If you want verbose debug logs in the Terminal (and a quieter GUI log panel), run:

- `npm run gui -- --verbose`

Or use the convenience script:

- `npm run gui:verbose`

### Command-line arguments

The GUI accepts these flags, e.g. `npm run gui -- --preset "Show A" --start`:

| Flag | Effect |
|---|---|
| `--start` | Auto-start the bridge with the resolved config |
| `--stop` | Stop the bridge if running (window stays open) |
| `--preset "Name"` | Load a preset by its display name before starting |
| `--ws "host:port"` | Override the Mixing Station WebSocket URL for this session only (not saved) |
| `--interval <ms>` | Override the metering interval (30-1000ms) for this session only (not saved) |
| `--log` | Enable JSON debug logging for this session |
| `--verbose` | Print bridge logs to the launching Terminal (unchanged, existing flag) |

Only one GUI instance runs at a time. Launching again with flags (e.g. from a shell script or
`open -a "Softube Console 1 MS Bridge" --args --stop`) forwards those flags to the already-running
instance instead of opening a second window.

`--start` and `--stop` together are treated as a mistake and both are ignored (a warning is logged).

## What it supports

- Track layout with custom ordering (inputs + buses + main), banked 10-wide for Console 1
- Track state mirroring: name/color, fader, mute, solo, pan, selection
- Sends mode: selecting a Bus master temporarily maps mute/pan to the chosen bus send
- Metering: Console 1 active meter requests → Mixing Station `metering2`
- Stability features: reconnect + init buffering + batching to avoid WS/SysEx spam

Note: Bus master solo is intentionally ignored (both directions), to avoid surprising “solo the whole bus” behavior.

## Building the desktop app

Build a standalone app for your platform with `electron-builder`:

1. Install deps:

  `npm install`

2. Build for your platform:

| Platform | Command | Artifacts |
|---|---|---|
| macOS | `npm run build:mac` | `.dmg`, `.zip` (x64 + arm64) |
| Windows | `npm run build:win` | portable `.exe` (x64) |

Artifacts are written to `dist/`.

## Localization

The GUI is translatable — English (`app/locales/en.json`) and Czech (`app/locales/cs.json`)
ship today. Every static string in `app/renderer/index.html` is marked with a `data-i18n*`
attribute; `applyI18n()` in `app/renderer/renderer.js` walks the DOM at startup (and again on
every language switch) and fills them in from the active locale. Dynamic strings (button/status
text set from JS) go through the `t(key, vars?)` helper the same way.

A language selector lives at the bottom of the sidebar. Switching it takes effect immediately —
no restart needed — and the choice is remembered for next launch. On first launch (before any
choice is saved), the locale is picked from the OS's language (falling back to English for
anything unrecognized).

To add a language:

1. Copy `app/locales/en.json` to `app/locales/<code>.json` (e.g. `de.json`) and translate the
   values — keep the keys and any `{placeholder}` tokens exactly as-is. Set `meta.localeName`
   to how the language should read in its own selector entry (e.g. `"Deutsch"`, not `"German"`).
2. A partial translation is fine: `app/i18n.js`'s `loadLocaleStrings()` merges it over the
   English fallback, so an untranslated key just shows English rather than breaking.
3. The new locale shows up in the sidebar's language selector automatically — no other code
   changes needed.

The bridge process (`index.js`)'s own console output is a technical/debug log, not user-facing
UI, and is intentionally out of scope — it stays English-only, same as most server/daemon logs.

## Track layout (banks + ordering)

Console 1 Fader Mk III has 10 faders, so the bridge builds 10-wide banks.

- Input banks: based on `INPUT_TRACK_ORDER` (supports stereo groups).
- Bus banks: based on `BUS_TRACK_ORDER` (supports stereo groups).
- Main: placed only once (10th fader on the last bus bank).

Stereo groups are expressed as `[left, right]` pairs inside the order arrays. For these grouped stereo tracks:

- Pan is locked (Console 1 pan changes are ignored)
- Displayed pan is forced to center

Edit these constants in [index.js](index.js) to match your mixer/project layout:

- `INPUT_TRACK_ORDER`
- `BUS_TRACK_ORDER`

Also note the channel ranges the bridge assumes:

- Inputs: `ch.0 .. ch.31` (`INPUT_CHANNEL_COUNT = 32`)
- Buses: `ch.48 .. ch.63` (`BUS_CHANNEL_START = 48`, `BUS_CHANNEL_COUNT = 16`)
- Main stereo: `ch.70 + ch.71` (`MAIN_STEREO_CHANNELS = [70, 71]`)

## Sends

Console 1 exposes 6 send slots, while Mixing Station can have up to 16 bus sends.

### Send slot mapping

Edit `C1_SEND_TO_MS_BUS_NUMBER` in [index.js](index.js):

- It maps Console 1 send slots (1..6) to Mixing Station bus numbers (1..16)
- Example in the current code: `Send4 → Bus7`, `Send5 → Bus9`, `Send6 → Bus13`

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

When run via the GUI, the bridge process is spawned once when the GUI launches and stays alive
(holding the Console 1 Fader MIDI connection) until the GUI quits — it no longer restarts on every
Start/Stop. Start/Stop instead toggle between two states, both controlled from the GUI's Start/Stop
button:

- **Standby**: Console 1 Fader connected, no Mixing Station connection.
- **Running**: full bridging active.

## Shutdown behavior

On `SIGINT`/`SIGTERM` (Ctrl+C):

- Sends a Console 1 `RESET`
- Deactivates all tracks on Console 1
- Closes MIDI ports and the WebSocket

## Troubleshooting

- “MIDI port not found”:
  - The bridge looks for ports containing `Console 1 Fader Mk III DAW`.
  - If your system reports a different name, adjust the `preferredNames` defaults in `openSoftubeMidiInput()` / `openSoftubeMidiOutput()` in [index.js](index.js).

- WebSocket won’t connect:
  - Verify Mixing Station WebSocket API is enabled.
  - Check `MIXING_STATION_WS_URL` and network/firewall settings.

- Can’t exit sends mode:
  - Main is only placed once (10th fader on the last bus bank). Select Main to return to standard mode.

- Need more visibility:
  - Set `LOG_JSON = true` in [index.js](index.js) to log WS/MIDI JSON payloads.
  - Set `LOG_METERING = true` in [index.js](index.js) to log metering parsing + subscription details.

- A topbar status dot (Mixing Station/Console 1 On-Screen Display) never turns green despite the
  app being present:
  - The match string in [app/statusMonitor.js](app/statusMonitor.js) likely doesn't match what
    your system actually reports. Check the exact process name via `ps -Ao args= | grep -i <app
    name>`, and adjust the corresponding string in `computeStatus()`.

- Clicking Start does nothing:
  - Confirm the GUI window is open (the bridge process only exists while the GUI is running).
  - If you click Start within a second or two of launching the app, the bridge may not have found
    the Console 1 Fader yet — the GUI's status badge can briefly show "Running" even though the
    bridge itself logged an "Ignoring lifecycle:start" warning and stayed in standby. Check the
    GUI's log panel for `[Lifecycle]`-prefixed lines confirming the bridge found the Console 1
    Fader port before pressing Start. The same applies if the bridge process ever crashes and gets
    re-spawned: the very next Start click can race the fresh process's MIDI detection the same way.

## Support

If this bridge saves you from buying dedicated hardware, consider supporting continued
development on [Ko-fi](https://ko-fi.com/strejda603).

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/K3N223V22V)