# Softube Console 1 Fader MK III ↔ Mixing Station Bridge

Node.js bridge that connects Softube Console 1 Fader MK III (over SysEx MIDI) with the Mixing Station WebSocket API.

It mirrors track state (name/color/volume/mute/solo/pan/selection + meters) and translates Console 1 controls into Mixing Station writes.

## Requirements

- Node.js (recommended: current LTS)
- Mixing Station with WebSocket API enabled/reachable (default is `ws://localhost:8080`)
- Softube Console 1 Fader Mk III MIDI ports available on your system

## Quick start

1. Install dependencies:

   `npm install`

2. Start the bridge:

   `npm start`

3. Confirm in the logs:

   - It opens the Console 1 MIDI input/output ports
   - It connects to Mixing Station WebSocket

## GUI launcher

This repo also includes an Electron-based launcher UI (settings + presets + live logs).

- Run in development:

  `npm run gui`

### Verbose GUI mode (terminal logs, quiet GUI console)

If you want the bridge to emit verbose debug logs (equivalent to `LOG_JSON=true`) **to the Terminal**
but keep the GUI "Console Output" panel clean, launch the GUI with `--verbose`:

- `npm run gui -- --verbose`

Or using the convenience script:

- `npm run gui:verbose`

## macOS app (.app) build

Build a standalone macOS app bundle (plus `.dmg` and `.zip` artifacts):

1. Install deps:

  `npm install`

2. Build:

  `npm run build:mac`

Artifacts are written to `dist/`.

## What it does

- Subscribes to required Mixing Station channel data and keeps a local cache.
- On connect/reconnect it buffers initial updates, then sends one full Console 1 `trackBatch` dump.
- After initialization, it sends incremental updates to Console 1 in small batches (to avoid SysEx spam).
- Coalesces fast Console 1 writes (faders/pan) before sending them to Mixing Station (to avoid WS spam).

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

### Send slot mapping (Send1..Send6 → which bus?)

Edit `C1_SEND_TO_MS_BUS_NUMBER` in [index.js](index.js):

- It maps Console 1 send slots (1..6) to Mixing Station bus numbers (1..16)
- Example in the current code: `Send4 → Bus7`, `Send5 → Bus9`, `Send6 → Bus13`

This mapping controls how Console 1 `send1..send6` and `send1On..send6On` are written to Mixing Station:

- `ch.<input>.mix.sends.<msSendIndex>.lvl`
- `ch.<input>.mix.sends.<msSendIndex>.on`

### “Sends mode” (mute/pan reflect a selected bus send)

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

## Shutdown behavior

On `SIGINT`/`SIGTERM` (Ctrl+C):

- Sends a Console 1 `RESET`
- Deactivates all tracks on Console 1
- Closes MIDI ports and the WebSocket

## Configuration of faders

Edit these in [index.js](index.js):

- `MIXING_STATION_WS_URL` (default `ws://localhost:8080`)
- `LOG_JSON` (set `true` to log JSON traffic)
- Layout ordering: `INPUT_TRACK_ORDER`, `BUS_TRACK_ORDER`
- Send mapping: `C1_SEND_TO_MS_BUS_NUMBER`

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