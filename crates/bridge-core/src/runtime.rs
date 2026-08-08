//! The reusable, command-driven, event-emitting bridge runtime. This is what both
//! `bridge-cli` (a thin binary, Task 4 of this plan) and the future Tauri app (Plan 4)
//! spawn and drive -- the actual orchestration logic (moved here from `bridge-cli` in
//! Task 2 of this plan) never changes shape based on which caller is driving it.
//!
//! Mirrors the `spawn_midi_engine`/`spawn_ws_engine` handle pattern already established
//! in `midi_io.rs`/`ws_engine.rs`: a plain (non-async) `spawn_*` function that creates
//! channels, `tokio::spawn`s the real async work, and returns a handle immediately.
//!
//! The orchestration itself finds the Console 1 Fader Mk III's MIDI ports, runs the
//! standby/running lifecycle, responds to Console 1's handshake/control protocol, holds the
//! Mixing Station WebSocket connection open while `running`, and mirrors inbound Mixing
//! Station channel data onto Console 1's display -- and, in the other direction, dispatches
//! Console 1's own fader/mute/solo/pan/selection/send/DSP moves back out as Mixing Station
//! writes (see `handle_inbound_midi_message`).

use crate::console_information::{apply_console_information, ConsoleArchitecture};
use crate::bare_update_queue::BareUpdateQueue;
use crate::channel_data_message::{parse_channel_data_get_message, MsFormat};
use crate::console1_status_bank::{
    hardware_trigger_type_for, start_slot_display_for, status_slot_color_for, HardwareTrigger,
    Lifecycle, START_SLOT_OBJECT_ID,
};
use crate::control_messages::{
    decide_control_actions, parse_active_meters, parse_control_message, ControlAction,
    ParsedControlMessage,
};
use crate::echo_suppression::EchoSuppressionTracker;
use crate::metering2_message::{
    affected_metered_object_ids, build_metering2_subscribe_body, compute_slot_meter_norm,
    normalize_metering2_values, MeterDbCache, METERING2_SUBSCRIPTION_ID,
};
use crate::midi_dsp_dispatch::{
    handle_midi_comp_update, handle_midi_eq_update, handle_midi_filter_update,
};
use crate::midi_io::{
    spawn_midi_engine, InboundMidiMessage, MidiCommand, MidiEngineHandle,
    DEFAULT_PREFERRED_PORT_NAMES,
};
use crate::midi_mixer_dispatch::{
    handle_midi_bus_main_sends_mode_fader_proxy, handle_midi_mute_update, handle_midi_pan_update,
    handle_midi_selected_update, handle_midi_send_slots_update, handle_midi_solo_update,
    handle_midi_volume_update, MsWrite,
};
use crate::ms_param_apply::{
    apply_ms_param_to_track_with_send_mapping, MsParamApplyResult, MsValueFormat,
};
use crate::ms_write_queue::{build_ms_set_request, MsWriteQueue};
use crate::send_mapping::{build_send_mapping, SendMapping};
use crate::sends_mode::{
    clear_console1_send_slots, handle_sends_mode_input_update,
    maybe_latch_sends_mode_from_selection, mirror_console1_send_slots_from_volume, InputSendState,
    SendsModeInputUpdateArgs, SendsModeInputUpdateOutcome,
};
use crate::sysex::{build_sysex_frame, parse_sysex_json};
use crate::track_cache::{DefaultTrackColors, TrackCache, TrackInfo};
use crate::track_layout::{
    build_track_layout, object_ids_by_ms_channel, LayoutSlot, LayoutSlotKind, TrackLayoutParams,
};
use crate::status_monitor::StatusSnapshot;
use crate::update_queue::UpdateQueue;
use crate::value_coercion::trim_stereo_suffix_from_name;
use crate::ws_engine::{spawn_ws_engine, WsCommand, WsEngineHandle, WsEvent, WS_RECONNECT_DELAY};
use bridge_config::{BridgeConfigPatch, RuntimeConfig};
use rand::rngs::StdRng;
use rand::SeedableRng;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc;

/// How long after a connect the buffered initialization window stays open before it's
/// force-flushed. Matches `index.js`'s `INIT_FLUSH_TIMEOUT_MS`.
const INIT_FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(2500);
/// Init-flush delay used by the forced-full-resync paths (Start, and a live config change while
/// disconnected). Matches `index.js`'s `forceConsole1FullResync`, whose delay is
/// `Math.min(500, INIT_FLUSH_TIMEOUT_MS)`.
const FORCE_RESYNC_FLUSH_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

/// Timeout for the `/console/information` discovery request race -- matches
/// `index.js`'s `fetchAndApplyConsoleInformation(600)` call site (its own default parameter
/// is 500ms, but the actual call passes 600 explicitly; 600 is the real behavior to match).
const CONSOLE_INFO_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(600);
/// Debounce window for batching `trackBatch` updates — matches `index.js`'s `CONSOLE1_FLUSH_MS`.
const CONSOLE1_FLUSH_DELAY: std::time::Duration = std::time::Duration::from_millis(20);
/// Debounce window for batching bare DSP-field updates — matches `CONSOLE1_BARE_FLUSH_MS`.
const CONSOLE1_BARE_FLUSH_DELAY: std::time::Duration = std::time::Duration::from_millis(20);
/// Debounce window for batching outbound Mixing Station writes — matches `MS_WRITE_FLUSH_MS`.
const MS_WRITE_FLUSH_DELAY: std::time::Duration = std::time::Duration::from_millis(15);
/// Console 1 has 4 parametric EQ bands — matches `index.js`'s `EQ_BAND_COUNT`.
const EQ_BAND_COUNT: usize = 4;
/// Max entries per outbound `trackBatch` SysEx message — matches `index.js`'s
/// `CONSOLE1_FULL_TRACK_BATCH_CHUNK_SIZE`.
const FULL_TRACK_BATCH_CHUNK_SIZE: usize = 100;
/// How long shutdown waits for the WS engine to finish its close handshake before giving up
/// and exiting anyway. No JS counterpart — JS's `msWebSocket.close()` is fire-and-forget and
/// the process exits immediately after.
const WS_SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// A command to the running bridge runtime task.
pub enum BridgeCommand {
    /// Enter `running`. `Some(patch)` applies a config patch first (matching JS's
    /// `enterRunningState(config)` calling `applyRuntimeConfig(config || {})` before
    /// anything else) -- `None` starts with whatever config is already live.
    Start(Option<BridgeConfigPatch>),
    /// Enter `standby`.
    Stop,
    /// Live-apply a config patch without a lifecycle transition (JS's
    /// `applyRuntimeConfigAndResync`). See Task 3 for the full behavior this triggers.
    ConfigApply(BridgeConfigPatch),
    /// A fresh live-status snapshot (MIDI hotplug / process detection) arrived -- push it onto
    /// the Console 1 hardware's own status-bank LEDs via `apply_live_status_colors`. Sent by
    /// the Tauri status-poll thread (`bridge-tauri`'s `status_gather`), independent of
    /// lifecycle -- matches JS's `applyLiveStatusColors`, which runs regardless of Standby vs
    /// Running.
    StatusUpdate(StatusSnapshot),
    /// Run the full shutdown sequence (RESET, deactivate all tracks, clear cache,
    /// disable OSD, close WS, close MIDI) and exit the runtime task. Triggered by
    /// `bridge-cli`'s Ctrl+C handler today; a future Tauri window-close handler later.
    Shutdown,
}

/// An event emitted by the runtime on its outbound channel. Additive by design: Plan 4 is
/// expected to extend this with GUI-specific variants (e.g. track-state snapshots) rather
/// than reshape the existing ones.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum BridgeEvent {
    /// A human-readable log line -- replaces every `println!` that used to live directly
    /// in `bridge-cli`'s orchestration code, so both a thin CLI (print to stdout) and a
    /// future Tauri app (forward to the GUI's live log view) can consume the same stream.
    Log(String),
    /// The bridge lifecycle (`standby`/`running`) changed.
    LifecycleChanged(Lifecycle),
    /// A `BridgeCommand::ConfigApply` was processed. Mirrors JS's `applyRuntimeConfig`'s
    /// return shape (`{urlChanged, anythingChanged}`) -- see Task 3.
    #[serde(rename_all = "camelCase")]
    ConfigApplied {
        url_changed: bool,
        anything_changed: bool,
    },
    /// Emitted by `bridge-tauri`'s panic watcher (see `src-tauri/src/lib.rs`) when the
    /// runtime's spawned task panics at the `tokio::spawn`/`JoinHandle` boundary, instead of
    /// silently taking down the whole process. Scope note: this only covers a panic that
    /// propagates out of the runtime task's own top-level body (its main `select!` loop and
    /// shutdown sequence in [`spawn_bridge_runtime`]) -- panics inside the MIDI thread or WS
    /// engine are already isolated separately (swallowed at their own join points near the
    /// end of that function's spawned task) and never reach this variant.
    Crashed(String),
}

/// Handle to the running bridge runtime — holds the outbound command sender, the inbound
/// event receiver, and the task join handle. Send [`BridgeCommand::Shutdown`] and then
/// `.await` `join_handle` to stop it cleanly. Simply dropping this handle without sending
/// `Shutdown` first also works (dropping `command_tx` makes the runtime's `command_rx.recv()`
/// return `None`, which is treated as a stop signal), but doesn't guarantee the task has
/// fully exited before your own code continues.
pub struct BridgeRuntimeHandle {
    /// Sends commands to the runtime. Unbounded, but bounded in practice: commands originate
    /// from user-driven actions (CLI signals, GUI buttons), not from the message-rate paths.
    pub command_tx: mpsc::UnboundedSender<BridgeCommand>,
    /// Receives [`BridgeEvent`]s. Stays open for exactly as long as the runtime task lives,
    /// so a `None`/`Disconnected` here means the task has exited.
    pub events: mpsc::UnboundedReceiver<BridgeEvent>,
    /// The runtime's `tokio` task. Stop it with `.await` (it's a [`tokio::task::JoinHandle`],
    /// not a `std::thread::JoinHandle` — there is no `.join()` to call).
    pub join_handle: tokio::task::JoinHandle<()>,
}

struct BridgeState {
    lifecycle: Lifecycle,
    osd_enabled: bool,
    /// The full Console 1 layout (status/Start bank + input/bus/main banks), built from
    /// `config` and `console_architecture` — matches `startBridgeProcess()` calling
    /// `rebuildTrackLayout()` unconditionally before ever entering standby (using whatever
    /// architecture is known at that moment, which is the hardcoded defaults until the first
    /// real `/console/information` reply updates `console_architecture`), and rebuilt again on
    /// every connect once that reply (or its 600ms timeout) resolves -- see
    /// `handle_ws_connected`/`finish_ws_connect`.
    layout: Vec<LayoutSlot>,
    track_cache: TrackCache,
    rng: StdRng,
    config: RuntimeConfig,
    /// MS channel index where bus 1 starts. Kept in sync with `console_architecture
    /// .bus_channel_start` by `rebuild_layout_and_reset_caches` (the only place either
    /// changes) -- read directly by call sites that just need this one number, so they don't
    /// all need to know about `console_architecture`'s other fields.
    bus_channel_start: usize,
    /// Live-fetched Console 1/Mixing Station channel architecture (input/bus/main counts and
    /// offsets), starting at the project's hardcoded defaults (`index.js` lines 139-147) and
    /// refreshed on every connect via `/console/information` -- see
    /// `handle_ws_connected`/`finish_ws_connect`. Drives `rebuild_layout_and_reset_caches`'s
    /// `build_default_layout` call.
    console_architecture: ConsoleArchitecture,
    /// True from the moment a `/console/information` request is sent until its reply arrives
    /// or `CONSOLE_INFO_TIMEOUT` elapses, whichever comes first. Matches JS's
    /// `consoleInfoRequestState.pending`.
    console_info_pending: bool,
    /// The Mixing Station WS engine, alive only while `running`. `None` in standby, matching
    /// JS's `msWebSocket` being closed/unset outside a running connection. Present but not yet
    /// connected between `enter_running_state` and the first `WsEvent::Connected`.
    ws_handle: Option<WsEngineHandle>,
    /// Whether the WS engine currently holds an open connection. Distinct from
    /// `ws_handle.is_some()`: the handle exists for the engine's whole life, including the
    /// initial connect attempt and every reconnect wait, during both of which `ws_engine` drops
    /// outbound sends on the floor. Mirrors JS's `readyState === OPEN` check.
    ws_connected: bool,
    /// Mirrors JS's `isInitializing` — true from `WsEvent::Connected` until initialization is
    /// finalized (early-completion or the flush timeout).
    is_initializing: bool,
    /// Buffered channel updates received while `is_initializing`, applied in one pass when
    /// initialization finalizes. Matches JS's `initMessageBuffer`.
    init_message_buffer: Vec<BufferedInitUpdate>,
    /// Matches JS's `hasSentInitialTrackDump` — gates the one-time forced full dump.
    has_sent_initial_track_dump: bool,
    /// Active sends-mode MS send index — see `crate::sends_mode`.
    sends_mode_ms_send_index: Option<usize>,
    /// The send index whose `mix.sends.<n>.on`/`.pan` WS subscriptions are currently open.
    /// Tracked separately from `sends_mode_ms_send_index` so leaving sends mode can
    /// unsubscribe exactly what it subscribed. Matches JS's `sendsModeSubscribedMsSendIndex`.
    sends_mode_subscribed_ms_send_index: Option<usize>,
    input_send_state: InputSendState,
    /// Per-MS-input-channel cache of the true standard `(mix.on, mix.pan)` values. Kept current
    /// even while sends mode is displaying send-derived values in `track.mute`/`track.pan`, so
    /// leaving sends mode can restore the real ones immediately instead of showing stale
    /// send-derived state until Mixing Station answers the refresh GETs. Matches JS's
    /// `inputStdState` (which, like this, is never cleared — it only ever gets overwritten).
    input_std_state: HashMap<usize, (Option<serde_json::Value>, Option<serde_json::Value>)>,
    send_mapping: SendMapping,
    echo_tracker: EchoSuppressionTracker,
    update_queue: UpdateQueue,
    bare_update_queue: BareUpdateQueue,
    /// Outbound Console 1 → Mixing Station writes, coalesced per `(path, format)` and flushed
    /// on a 15ms debounce. Populated only by `handle_inbound_midi_message`.
    ms_write_queue: MsWriteQueue,
    /// MS channel -> layout slot object IDs, built once from `layout` at startup so inbound
    /// message dispatch doesn't recompute it per message.
    object_ids_by_ms_channel: HashMap<usize, Vec<usize>>,
    /// MS channels seen at least once during the current initialization window — drives
    /// early-finalize. Matches JS's `initSeenMsChannels`.
    init_seen_ms_channels: HashSet<i64>,
    /// Console 1 object IDs currently selected for metering (via hardware's `activeMeters`
    /// control message). Replaced wholesale on every `activeMeters` message, never merged --
    /// matches JS's `meteredObjectIds = new Set()` reset in `handleConsole1ControlJson`.
    metered_object_ids: HashSet<usize>,
    /// The MS channel list from the most recently sent metering2 subscribe request, in the
    /// same order as that request's `params` array. Inbound metering2 pushes are positional
    /// against this list (index 0 of an inbound value list corresponds to this list's first
    /// channel). Matches JS's `metering2ParamMsChannels`.
    metering2_param_ms_channels: Vec<i64>,
    /// Per-MS-channel latest recorded meter dB value, with 0.05dB change-detection built in --
    /// see `metering2_message::MeterDbCache`. Matches JS's `msChannelMeterDb`.
    meter_db_cache: MeterDbCache,
}

/// One buffered channel update captured during initialization, replayed once initialization
/// finalizes. Matches JS's `{channelIndex, paramPath, value, format}` `initMessageBuffer` entries.
struct BufferedInitUpdate {
    channel_index: i64,
    param_path: String,
    value: serde_json::Value,
    format: MsFormat,
}

fn build_default_layout(config: &RuntimeConfig, architecture: &ConsoleArchitecture) -> Vec<LayoutSlot> {
    let main_stereo_channels: Vec<usize> = architecture
        .main_stereo_channels
        .iter()
        .map(|&ch| ch as usize)
        .collect();
    let params = TrackLayoutParams {
        input_track_order: &config.input_track_order,
        bus_track_order: &config.bus_track_order,
        input_channel_count: architecture.input_channel_count as usize,
        bus_channel_start: architecture.bus_channel_start as usize,
        bus_channel_count: architecture.bus_channel_count as usize,
        main_stereo_channels: &main_stereo_channels,
    };
    build_track_layout(&params)
}

fn default_colors(config: &RuntimeConfig) -> DefaultTrackColors {
    DefaultTrackColors {
        bus_color: config.console1_bus_color,
        main_color: config.console1_main_color,
        status_off_color: 0x0000ff,
        status_on_color: 0x00ff00,
        start_color: 0xff841b,
        stop_color: 0x5a28f8,
    }
}

/// Send a `trackBatch` SysEx message for the fixed status/Start bank (bank 0) — used entering
/// standby and on RESET-while-standby, matching `sendStatusBankTracks`. Reads bank-0 slots
/// directly out of `state.layout` (already built by `build_track_layout`, which itself already
/// handles the status-bank-to-layout-slot conversion — Plan 2a's `track_layout` module, not
/// re-derived here).
fn send_status_bank_tracks(
    state: &mut BridgeState,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) {
    let colors = default_colors(&state.config);
    let bus_channel_start = state.bus_channel_start;
    let bank_zero_slots: Vec<LayoutSlot> = state
        .layout
        .iter()
        .filter(|s| s.object_id <= START_SLOT_OBJECT_ID)
        .cloned()
        .collect();
    let mut batch = Vec::new();
    for slot in &bank_zero_slots {
        let track = state
            .track_cache
            .get_or_create(
                slot.object_id,
                slot,
                state.lifecycle,
                &colors,
                bus_channel_start,
                &mut state.rng,
            )
            .clone();
        batch.push(track_info_to_trackbatch_json(&track));
    }
    send_sysex_and_log(midi_tx, event_tx, state.config.log_json, &json!({ "trackBatch": batch }));
}

/// Project a `TrackInfo` into the wire-safe `trackBatch` shape via `ConsoleTrackFields` (drops
/// DSP fields — see `sysex::ConsoleTrackFields`'s doc comment for why they can never appear
/// here). Routing through the shared `From<&TrackInfo> for ConsoleTrackFields` conversion
/// (rather than hand-writing the field list here) means a `TrackInfo` field rename can't
/// silently desync from what actually gets sent — it fails to compile instead.
fn track_info_to_trackbatch_json(track: &TrackInfo) -> serde_json::Value {
    let fields = crate::sysex::ConsoleTrackFields::from(track);
    serde_json::to_value(fields).expect("ConsoleTrackFields always serializes")
}

/// Push the 7 status-bank indicator LEDs' colors from a live status snapshot -- port of JS's
/// `applyLiveStatusColors`. Applies regardless of lifecycle state (this only touches per-slot
/// cached track objects, creating them on demand, and force-sends targeted updates -- never
/// `finalize_initialization`/a full track dump -- so it carries none of the standby-leak risk
/// those do). Only sends a `trackBatch` frame when at least one slot's color actually changed;
/// a stable status session between poll ticks sends nothing. On the very first poll tick after
/// startup, each of the 7 status slots is created fresh at `colors.status_off_color` and then
/// immediately compared against the real snapshot -- so if any indicator is already on at that
/// point, at least one entry lands in that first `trackBatch`; an all-off first snapshot (e.g.
/// nothing detected yet) matches the fresh off-color exactly and legitimately sends nothing.
/// Both outcomes are correct/intended, matching JS.
fn apply_live_status_colors(
    state: &mut BridgeState,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    snapshot: &StatusSnapshot,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) {
    let snapshot_value = serde_json::to_value(snapshot).expect("StatusSnapshot always serializes");
    let colors = default_colors(&state.config);
    let bus_channel_start = state.bus_channel_start;
    let status_slots: Vec<LayoutSlot> = state
        .layout
        .iter()
        .filter(|s| matches!(s.kind, LayoutSlotKind::Status { .. }))
        .cloned()
        .collect();

    let mut changed = Vec::new();
    for slot in &status_slots {
        let key = match slot.kind {
            LayoutSlotKind::Status { key, .. } => key,
            _ => continue,
        };
        state.track_cache.get_or_create(
            slot.object_id,
            slot,
            state.lifecycle,
            &colors,
            bus_channel_start,
            &mut state.rng,
        );
        let is_on = snapshot_value.get(key).unwrap_or(&Value::Null);
        let color = status_slot_color_for(is_on, colors.status_on_color, colors.status_off_color);
        if let Some(track) = state.track_cache.get_mut(slot.object_id) {
            if track.color != color {
                track.color = color;
                changed.push(json!({"trackId": track.track_id, "color": color}));
            }
        }
    }

    if !changed.is_empty() {
        send_sysex_and_log(midi_tx, event_tx, state.config.log_json, &json!({ "trackBatch": changed }));
    }
}

fn apply_start_slot_display(
    state: &mut BridgeState,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) {
    let colors = default_colors(&state.config);
    let display = start_slot_display_for(state.lifecycle, colors.start_color, colors.stop_color);
    if let Some(track) = state.track_cache.get_mut(START_SLOT_OBJECT_ID) {
        track.name = display.name.to_string();
        track.color = display.color;
        let updated = track.clone();
        // NOTE: partial update — not routed through ConsoleTrackFields, so field names here
        // aren't compile-time-checked against Console 1's allowlist like full-track sends are
        // (see ConsoleTrackFields). Every field used here (trackId, name, color) is in the
        // allowlist today; just no compiler backstop if a future edit adds a bad one.
        send_sysex_and_log(
            midi_tx,
            event_tx,
            state.config.log_json,
            &json!({
                "trackBatch": [{"trackId": updated.track_id, "name": updated.name, "color": updated.color}]
            }),
        );
    }
}

/// Tell Console 1 to drop every cached real-channel track (`{trackId, isActive: false}`),
/// leaving the status/Start bank alone. Port of `index.js`'s `deactivateRealChannelTracks`,
/// whose "skip `status`/`start` slots" filter is expressed here as the same
/// `object_id > START_SLOT_OBJECT_ID` bank-0 boundary the rest of this file already uses.
///
/// JS's equivalent takes a `forceSend` flag that bypasses `sendSysexToConsole1`'s OSD gate;
/// this runtime has no transport-layer gate at all (OSD gating lives in the queue-flush
/// functions), so these batches always go out — which is what every JS call site of this
/// function asks for anyway.
fn deactivate_real_channel_tracks(
    state: &BridgeState,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) {
    let entries: Vec<serde_json::Value> = state
        .track_cache
        .iter()
        .filter(|(&object_id, _)| object_id > START_SLOT_OBJECT_ID)
        .map(|(_, track)| json!({"trackId": &track.track_id, "isActive": false}))
        .collect();
    send_track_batches(&entries, midi_tx, state.config.log_json, event_tx);
}

/// Same, but for EVERY cached track including the status/Start bank. Port of
/// `batchDeactivateAllTracks`, used only by shutdown — standby entry uses the
/// real-channels-only variant above, matching JS's two distinct call sites.
fn deactivate_all_tracks(
    state: &BridgeState,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) {
    let entries: Vec<serde_json::Value> = state
        .track_cache
        .iter()
        .map(|(_, track)| json!({"trackId": &track.track_id, "isActive": false}))
        .collect();
    send_track_batches(&entries, midi_tx, state.config.log_json, event_tx);
}

fn enter_standby_state(
    state: &mut BridgeState,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) {
    state.lifecycle = Lifecycle::Standby;
    let _ = event_tx.send(BridgeEvent::Log("[Lifecycle] standby".to_string()));
    let _ = event_tx.send(BridgeEvent::LifecycleChanged(Lifecycle::Standby));
    state.ws_connected = false;
    if let Some(handle) = state.ws_handle.take() {
        let _ = handle.command_tx.send(WsCommand::Shutdown);
        // Deliberately not awaited: dropping the handle is sufficient per `WsEngineHandle`'s
        // documented drop semantics, and this function isn't async. The engine task exits on
        // its own; nothing here needs to observe that exit synchronously.
    }
    state.is_initializing = false;
    // Drop whatever is still sitting on the 20ms debounce timers. A Start→Stop inside that
    // window would otherwise flush stale real-channel fields to Console 1 a moment after the
    // display has already been reset to the status/Start bank. JS's `enterStandbyState` leaves
    // its queues alone here — this is a deliberate improvement on it, not a port.
    state.update_queue.take_all();
    state.bare_update_queue.take_all();
    deactivate_real_channel_tracks(state, midi_tx, event_tx);
    state.track_cache.reset_real_channels(START_SLOT_OBJECT_ID);
    send_status_bank_tracks(state, midi_tx, event_tx);
    apply_start_slot_display(state, midi_tx, event_tx);
}

/// Enter `running`: rebuild the track layout from the (possibly just-patched) config, open a
/// forced-resync initialization window, spawn the Mixing Station WS engine (torn down again on
/// the next standby transition, matching JS's per-Start/Stop `new WebSocket(...)`/`.close()`
/// cycle) and show "Stop" on the Start slot. Connection-open orchestration lives in
/// `handle_ws_connected`, fired when `WsEvent::Connected` arrives — not here, since spawning is
/// instant but the real TCP connect is not.
///
/// Ports the full body of JS's `enterRunningState`: `rebuildTrackLayout()` then
/// `forceConsole1FullResync("lifecycle:start")` then `connectMixingStationWebSocket()`. The
/// resync is armed unconditionally, exactly as in JS — a later `handle_ws_connected` supersedes
/// it with real data, but the timer is what guarantees Console 1 gets *some* track dump even
/// when Mixing Station is slow or unreachable.
///
/// Returns `true` to tell the caller to arm the init-flush timer at `FORCE_RESYNC_FLUSH_DELAY`
/// (the caller owns that timer), the same contract as `handle_ws_connected`.
#[must_use]
fn enter_running_state(
    state: &mut BridgeState,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) -> bool {
    state.lifecycle = Lifecycle::Running;
    let _ = event_tx.send(BridgeEvent::Log("[Lifecycle] running".to_string()));
    let _ = event_tx.send(BridgeEvent::LifecycleChanged(Lifecycle::Running));

    rebuild_layout_and_reset_caches(state);
    state.is_initializing = true;
    state.init_message_buffer.clear();
    state.has_sent_initial_track_dump = false;

    // A Start while already running (double-Start) drops the previous handle here without an
    // explicit `WsCommand::Shutdown`; the old engine task ends when its command channel closes
    // on drop, which is the same cleanup `reconnect_ws`'s explicit shutdown only accelerates.
    let handle = spawn_ws_engine(
        state.config.mixing_station_ws_url.clone(),
        WS_RECONNECT_DELAY,
    );
    state.ws_handle = Some(handle);
    apply_start_slot_display(state, midi_tx, event_tx);
    true
}

/// Rebuild `state.layout` from the current config and drop everything derived from the old one.
/// Shared by the two callers that can change the layout at runtime: `enter_running_state` (the
/// Start patch may reorder tracks) and `apply_config_patch`'s non-URL branch.
fn rebuild_layout_and_reset_caches(state: &mut BridgeState) {
    state.bus_channel_start = state.console_architecture.bus_channel_start as usize;
    state.layout = build_default_layout(&state.config, &state.console_architecture);
    state.object_ids_by_ms_channel = object_ids_by_ms_channel(&state.layout);
    state.track_cache.reset_real_channels(START_SLOT_OBJECT_ID);
    state.init_seen_ms_channels.clear();
    state.sends_mode_ms_send_index = None;
    state.sends_mode_subscribed_ms_send_index = None;
    state.input_send_state.clear();
    state.metered_object_ids = HashSet::new();
    state.metering2_param_ms_channels = Vec::new();
    state.meter_db_cache = MeterDbCache::new();
}

/// Port of `index.js`'s plain `applyRuntimeConfig` — update the live config and rebuild the
/// send mapping, with none of the reconnect/resync side effects its `...AndResync` sibling
/// (`apply_config_patch`) adds. This is what the Start path uses, matching JS's
/// `enterRunningState(config)` calling this variant rather than the resync-aware one.
///
/// JS rebuilds the send mapping only when `c1SendToMsBusNumber` is present in the patch;
/// rebuilding unconditionally is the same harmless superset `apply_config_patch` already
/// uses, since rebuilding from unchanged data produces an identical mapping.
fn apply_runtime_config(state: &mut BridgeState, patch: &BridgeConfigPatch) {
    let _ = bridge_config::apply_patch(&mut state.config, patch);
    state.send_mapping = build_send_mapping(
        &state.config.c1_send_to_ms_bus_number,
        state.console_architecture.bus_channel_count as usize,
    );
}

/// Port of `index.js`'s `applyRuntimeConfigAndResync` — live-apply a config patch without
/// restarting. A URL change just reconnects; any other change rebuilds the layout, drops the
/// layout-derived caches, and either reconnects (to get Mixing Station to re-push current
/// values, which a redundant subscribe on a live connection would not) or forces a Console 1
/// full resync on a 500ms timer (JS's `forceConsole1FullResync`, whose delay is
/// `Math.min(500, INIT_FLUSH_TIMEOUT_MS)`).
#[allow(clippy::too_many_arguments)]
async fn apply_config_patch(
    state: &mut BridgeState,
    patch: &BridgeConfigPatch,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    init_flush_timer: &mut std::pin::Pin<&mut tokio::time::Sleep>,
    init_flush_armed: &mut bool,
) {
    let before_url = state.config.mixing_station_ws_url.clone();
    let result = bridge_config::apply_patch(&mut state.config, patch);
    if state.config.log_json {
        let _ = event_tx.send(BridgeEvent::Log(format!("[config] apply result: {result:?}")));
    }

    if !result.anything_changed {
        let _ = event_tx.send(BridgeEvent::Log("[config] No changes to apply".to_string()));
        return;
    }

    let _ = event_tx.send(BridgeEvent::Log(format!(
        "[config] Applied updates (wsUrlChanged={}){}",
        if result.url_changed { "yes" } else { "no" },
        if before_url != state.config.mixing_station_ws_url {
            format!(" {} -> {}", before_url, state.config.mixing_station_ws_url)
        } else {
            String::new()
        }
    )));
    let _ = event_tx.send(BridgeEvent::ConfigApplied {
        url_changed: result.url_changed,
        anything_changed: result.anything_changed,
    });

    // JS rebuilds this only when `c1SendToMsBusNumber` was present in the incoming config; this
    // port doesn't track per-field presence at this call site, and rebuilding from unchanged
    // data is a no-op in effect, so doing it whenever anything changed is a safe superset.
    state.send_mapping = build_send_mapping(
        &state.config.c1_send_to_ms_bus_number,
        state.console_architecture.bus_channel_count as usize,
    );

    if result.url_changed {
        // Nothing is connected in standby, so there is nothing to reconnect — the next
        // `enter_running_state` spawns the engine at the new URL anyway.
        if state.lifecycle == Lifecycle::Running {
            reconnect_ws(state).await;
        }
        return;
    }

    rebuild_layout_and_reset_caches(state);

    if state.ws_connected {
        // Reconnect rather than re-subscribe on the live connection: Mixing Station only pushes
        // a path's current value for a genuinely *new* subscription, so re-subscribing here
        // would leave Console 1 showing stale values for the rebuilt layout.
        reconnect_ws(state).await;
    } else {
        state.is_initializing = true;
        state.init_message_buffer.clear();
        state.has_sent_initial_track_dump = false;
        *init_flush_armed = true;
        init_flush_timer
            .as_mut()
            .reset(tokio::time::Instant::now() + FORCE_RESYNC_FLUSH_DELAY);
        let _ = midi_tx; // reserved: no direct MIDI send in this branch, kept for signature symmetry
    }
}

/// Shuts down the existing WS engine (if any) and spawns a fresh one at the current
/// `state.config.mixing_station_ws_url`. Shared by both `apply_config_patch` branches
/// that need a reconnect (URL changed, or layout/mapping changed while connected).
async fn reconnect_ws(state: &mut BridgeState) {
    if let Some(handle) = state.ws_handle.take() {
        let _ = handle.command_tx.send(WsCommand::Shutdown);
        state.ws_connected = false;
    }
    let handle = spawn_ws_engine(
        state.config.mixing_station_ws_url.clone(),
        WS_RECONNECT_DELAY,
    );
    state.ws_handle = Some(handle);
}

fn ws_send_json(state: &BridgeState, value: serde_json::Value) {
    if let Some(handle) = &state.ws_handle {
        let _ = handle.command_tx.send(WsCommand::Send(value.to_string()));
    }
}

/// (Re)subscribes to Mixing Station metering2 based on `state.metered_object_ids`. No-ops
/// (without touching `state.metering2_param_ms_channels`) if the WS connection isn't open --
/// matches JS's `updateMetering2Subscription`'s `readyState !== OPEN` guard, which returns
/// before touching `metering2ParamMsChannels` at all, not just before sending.
fn update_metering2_subscription(state: &mut BridgeState) {
    if !state.ws_connected {
        return;
    }
    let (channels, body) = build_metering2_subscribe_body(
        &state.metered_object_ids,
        &state.layout,
        state.config.metering2_interval_ms,
    );
    state.metering2_param_ms_channels = channels;
    ws_send_json(state, body);
}

/// Immediately (within one 20ms flush-timer tick, via a force-sent queue entry) re-affirms the
/// current meter value for every active, currently-metered track. Port of JS's
/// `batchSendChangedMeters` -- uses the existing `UpdateQueue` (force-sent) instead of JS's
/// direct synchronous SysEx send: a raw hand-built `trackBatch` payload would bypass this
/// codebase's `ConsoleTrackFields`-enforced outbound allowlist safety net.
fn batch_send_changed_meters(state: &mut BridgeState) {
    if state.metered_object_ids.is_empty() {
        return;
    }
    let object_ids: Vec<usize> = state.metered_object_ids.iter().copied().collect();
    for object_id in object_ids {
        let Some(slot) = state.layout.get(object_id).cloned() else {
            continue;
        };
        let track = clone_or_create_track(state, object_id, &slot);
        if !track.is_active {
            continue;
        }
        let mut partial = HashMap::new();
        partial.insert("meter".to_string(), json!(track.meter));
        state.update_queue.queue(track.track_id.clone(), partial, true);
    }
}

/// Handles a Console 1 `activeMeters` control message: rebuilds `metered_object_ids` from the
/// given track ID list (unresolvable track IDs are silently skipped, matching JS's
/// `getObjectIdForTrackId` returning `undefined`), then (re)subscribes to metering2 and
/// immediately re-affirms the current meter values for the newly metered tracks. Port of the
/// `activeMeters` branch of `handleConsole1ControlJson` (`index.js:3065-3075`).
fn handle_active_meters_message(track_ids: Vec<String>, state: &mut BridgeState) {
    let metered: HashSet<usize> = track_ids
        .iter()
        .filter_map(|track_id| state.track_cache.object_id_for_track_id(track_id))
        .collect();
    state.metered_object_ids = metered;

    update_metering2_subscription(state);
    batch_send_changed_meters(state);
}

/// Minimum meter-norm delta (Console 1's 0..1 meter scale) that's worth re-queuing an update
/// for -- suppresses queue churn from dB changes too small to move the norm meaningfully.
const METER_NORM_CHANGE_THRESHOLD: f64 = 0.001;

/// For each of the given MS channels whose dB value just changed, finds the affected metered
/// track slots (via `metering2_message::affected_metered_object_ids`), and for each active one
/// whose newly-computed meter norm differs from its previous value by more than
/// `METER_NORM_CHANGE_THRESHOLD`, queues an updated `{"meter": [...]}` field (plain, non-forced
/// -- matches JS's `queueConsole1TrackUpdate(track.trackId, { meter: track.meter })`, which
/// passes no `opts`).
/// Port of `applyMeterUpdatesForMsChannels` (`index.js:989-1011`).
///
/// Unlike `batch_send_changed_meters`, which only re-affirms the already-cached track's
/// existing meter value, this function computes a genuinely new meter value and must persist it
/// back via `store_track` before queuing, since `clone_or_create_track` returns an owned clone,
/// not a live reference into the cache.
fn apply_meter_updates_for_ms_channels(state: &mut BridgeState, ms_channels: &[i64]) {
    let affected = affected_metered_object_ids(
        ms_channels,
        &state.object_ids_by_ms_channel,
        &state.metered_object_ids,
    );
    for object_id in affected {
        let Some(slot) = state.layout.get(object_id).cloned() else {
            continue;
        };
        let mut track = clone_or_create_track(state, object_id, &slot);
        if !track.is_active {
            continue;
        }
        let slot_channels: Vec<i64> = slot.ms_channels.iter().map(|&ch| ch as i64).collect();
        let next = compute_slot_meter_norm(&slot_channels, &state.meter_db_cache);
        let prev = track.meter.first().copied();
        if prev.is_none_or(|p| (p - next).abs() > METER_NORM_CHANGE_THRESHOLD) {
            track.meter = vec![next];
            let mut partial = HashMap::new();
            partial.insert("meter".to_string(), json!(track.meter));
            let track_id = track.track_id.clone();
            store_track(state, object_id, track);
            state.update_queue.queue(track_id, partial, false);
        }
    }
}

/// Sends a SysEx frame to Console 1, logging the JSON payload + wire-byte hex dump when
/// `log_json` is set -- single choke point for every outbound send, matching JS's
/// `sendSysexToConsole1` (`index.js:2624-2696`), including its `LOG_JSON`-gated log
/// (`index.js:2687-2691`). Every call site that used to do
/// `let frame = build_sysex_frame(&value); let _ = midi_tx.send(MidiCommand::Send(frame));`
/// should call this instead.
fn send_sysex_and_log(
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
    log_json: bool,
    value: &serde_json::Value,
) {
    if log_json {
        let _ = event_tx.send(BridgeEvent::Log(format!(
            "Sending SysEx JSON to Console 1: {value}"
        )));
    }
    let frame = build_sysex_frame(value);
    if log_json {
        let hex: String = frame.iter().map(|b| format!("{b:02x}")).collect();
        let _ = event_tx.send(BridgeEvent::Log(format!("SysEx data: {hex}")));
    }
    let _ = midi_tx.send(MidiCommand::Send(frame));
}

fn subscribe_to_channel_data(state: &BridgeState, path: &str, format: &str) {
    ws_send_json(
        state,
        json!({
            "path": "/console/data/subscribe",
            "method": "POST",
            "body": {"path": path, "format": format},
        }),
    );
}

fn unsubscribe_from_channel_data(state: &BridgeState, path: &str, format: &str) {
    ws_send_json(
        state,
        json!({
            "path": "/console/data/unsubscribe",
            "method": "POST",
            "body": {"path": path, "format": format},
        }),
    );
}

fn request_mixing_station_value(state: &BridgeState, path: &str, format: &str) {
    ws_send_json(
        state,
        json!({"path": format!("/console/data/get/{path}/{format}"), "method": "GET"}),
    );
}

/// Port of `index.js`'s `subscribeToRequiredChannelData`.
fn subscribe_to_required_channel_data(state: &BridgeState) {
    subscribe_to_channel_data(state, "ch.*.cfg.name", "val");
    subscribe_to_channel_data(state, "ch.*.cfg.color", "val");
    subscribe_to_channel_data(state, "ch.*.mix.lvl", "val");
    subscribe_to_channel_data(state, "ch.*.mix.pan", "norm");
    subscribe_to_channel_data(state, "ch.*.solo", "val");
    subscribe_to_channel_data(state, "ch.*.mix.on", "val");
    subscribe_to_channel_data(state, "ch.*.selected", "val");

    for &ms_send_index in &state.send_mapping.c1_to_ms_send_index {
        subscribe_to_channel_data(state, &format!("ch.*.mix.sends.{ms_send_index}.lvl"), "val");
        subscribe_to_channel_data(state, &format!("ch.*.mix.sends.{ms_send_index}.on"), "val");
    }

    subscribe_to_channel_data(state, "ch.*.preamp.filter.0.on", "val");
    subscribe_to_channel_data(state, "ch.*.preamp.filter.0.freq", "norm");
    subscribe_to_channel_data(state, "ch.*.preamp.filter.0.freq", "val");
    subscribe_to_channel_data(state, "ch.*.headamp.gain", "val");
    subscribe_to_channel_data(state, "ch.*.preamp.inv", "val");
    subscribe_to_channel_data(state, "ch.*.peq.on", "val");

    for i in 0..EQ_BAND_COUNT {
        subscribe_to_channel_data(state, &format!("ch.*.peq.bands.{i}.freq"), "norm");
        subscribe_to_channel_data(state, &format!("ch.*.peq.bands.{i}.freq"), "val");
        subscribe_to_channel_data(state, &format!("ch.*.peq.bands.{i}.gain"), "norm");
        subscribe_to_channel_data(state, &format!("ch.*.peq.bands.{i}.gain"), "val");
        subscribe_to_channel_data(state, &format!("ch.*.peq.bands.{i}.q"), "norm");
        subscribe_to_channel_data(state, &format!("ch.*.peq.bands.{i}.q"), "val");
        subscribe_to_channel_data(state, &format!("ch.*.peq.bands.{i}.type"), "val");
    }

    subscribe_to_channel_data(state, "ch.*.dyn.on", "val");
    subscribe_to_channel_data(state, "ch.*.dyn.ratio", "norm");
    subscribe_to_channel_data(state, "ch.*.dyn.ratio", "val");
    subscribe_to_channel_data(state, "ch.*.dyn.attack", "norm");
    subscribe_to_channel_data(state, "ch.*.dyn.attack", "val");
    subscribe_to_channel_data(state, "ch.*.dyn.release", "norm");
    subscribe_to_channel_data(state, "ch.*.dyn.release", "val");
    subscribe_to_channel_data(state, "ch.*.dyn.gain", "norm");
    subscribe_to_channel_data(state, "ch.*.dyn.gain", "val");
    subscribe_to_channel_data(state, "ch.*.dyn.thr", "norm");
    subscribe_to_channel_data(state, "ch.*.dyn.thr", "val");
    subscribe_to_channel_data(state, "ch.*.dyn.knee", "norm");
    subscribe_to_channel_data(state, "ch.*.dyn.knee", "val");
    subscribe_to_channel_data(state, "ch.*.dyn.mix", "norm");
}

/// Port of `index.js`'s `refreshDspFieldsForRealChannels`. Subscribing alone doesn't retro-push
/// current values for the Filter/EQ/Compressor paths (confirmed empirically — see the JS doc
/// comment this ports), so an explicit one-shot GET per field is needed after every (re)connect.
fn refresh_dsp_fields_for_real_channels(state: &BridgeState) {
    let mut channels: Vec<usize> =
        (0..state.console_architecture.input_channel_count as usize).collect();
    channels.extend(
        state.bus_channel_start
            ..state.bus_channel_start + state.console_architecture.bus_channel_count as usize,
    );
    channels.extend(
        state
            .console_architecture
            .main_stereo_channels
            .iter()
            .map(|&ch| ch as usize),
    );

    for ch in channels {
        request_mixing_station_value(state, &format!("ch.{ch}.preamp.filter.0.on"), "val");
        request_mixing_station_value(state, &format!("ch.{ch}.preamp.filter.0.freq"), "norm");
        request_mixing_station_value(state, &format!("ch.{ch}.preamp.filter.0.freq"), "val");
        request_mixing_station_value(state, &format!("ch.{ch}.headamp.gain"), "val");
        request_mixing_station_value(state, &format!("ch.{ch}.preamp.inv"), "val");
        request_mixing_station_value(state, &format!("ch.{ch}.peq.on"), "val");

        for i in 0..EQ_BAND_COUNT {
            request_mixing_station_value(state, &format!("ch.{ch}.peq.bands.{i}.freq"), "norm");
            request_mixing_station_value(state, &format!("ch.{ch}.peq.bands.{i}.freq"), "val");
            request_mixing_station_value(state, &format!("ch.{ch}.peq.bands.{i}.gain"), "norm");
            request_mixing_station_value(state, &format!("ch.{ch}.peq.bands.{i}.gain"), "val");
            request_mixing_station_value(state, &format!("ch.{ch}.peq.bands.{i}.q"), "norm");
            request_mixing_station_value(state, &format!("ch.{ch}.peq.bands.{i}.q"), "val");
            request_mixing_station_value(state, &format!("ch.{ch}.peq.bands.{i}.type"), "val");
        }

        request_mixing_station_value(state, &format!("ch.{ch}.dyn.on"), "val");
        request_mixing_station_value(state, &format!("ch.{ch}.dyn.ratio"), "norm");
        request_mixing_station_value(state, &format!("ch.{ch}.dyn.ratio"), "val");
        request_mixing_station_value(state, &format!("ch.{ch}.dyn.attack"), "norm");
        request_mixing_station_value(state, &format!("ch.{ch}.dyn.attack"), "val");
        request_mixing_station_value(state, &format!("ch.{ch}.dyn.release"), "norm");
        request_mixing_station_value(state, &format!("ch.{ch}.dyn.release"), "val");
        request_mixing_station_value(state, &format!("ch.{ch}.dyn.gain"), "norm");
        request_mixing_station_value(state, &format!("ch.{ch}.dyn.gain"), "val");
        request_mixing_station_value(state, &format!("ch.{ch}.dyn.thr"), "norm");
        request_mixing_station_value(state, &format!("ch.{ch}.dyn.thr"), "val");
        request_mixing_station_value(state, &format!("ch.{ch}.dyn.knee"), "norm");
        request_mixing_station_value(state, &format!("ch.{ch}.dyn.knee"), "val");
        request_mixing_station_value(state, &format!("ch.{ch}.dyn.mix"), "norm");
    }
}

/// Runs on `WsEvent::Connected`. Sends the `/console/information` architecture-discovery
/// request and starts its timeout race -- the rest of what a connect needs to do (rebuild the
/// layout from whatever architecture is now known, reset init/subscription state, subscribe,
/// send the handshake) is deferred to `finish_ws_connect`, run once the reply arrives (via
/// `handle_ws_message`) or `CONSOLE_INFO_TIMEOUT` elapses, whichever comes first.
///
/// Matches `index.js`'s `connectMixingStationWebSocket` `open` handler up through its
/// `await fetchAndApplyConsoleInformation(600)` call -- everything after that await is now
/// `finish_ws_connect`.
fn handle_ws_connected(state: &mut BridgeState, event_tx: &mpsc::UnboundedSender<BridgeEvent>) {
    let _ = event_tx.send(BridgeEvent::Log(
        "Connected to Mixing Station WebSocket".to_string(),
    ));
    state.ws_connected = true;
    state.console_info_pending = true;
    ws_send_json(state, json!({"path": "/console/information", "method": "GET"}));
}

/// Finishes what `WsEvent::Connected` started, once the `/console/information` race resolves
/// (reply or timeout) -- rebuilds the layout from the now-current `state.console_architecture`,
/// resets init/subscription state and the real-channel track cache, sends every subscribe and
/// one-shot refresh request, disables OSD, and sends the Console 1 handshake. Returns `true` to
/// tell the caller to arm the init-flush timer (the caller owns that timer future) -- the same
/// contract `handle_ws_connected` used to have, before this function existed.
///
/// Re-checks `state.ws_connected` first: standby (Stop) may have been processed while the race
/// was in flight (e.g. Start then Stop within the up-to-600ms round trip) -- same reasoning as
/// JS's re-check of `bridgeLifecycle !== "running"` right after its own await. If the
/// connection this race started for is gone, this is a no-op; the already-triggered
/// disconnect/standby path owns teardown.
///
/// Fires on every connect — the first one after a Start and any later auto-reconnect within
/// the same running session — so every reset here must be safe to repeat mid-session.
fn finish_ws_connect(
    state: &mut BridgeState,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) -> bool {
    if !state.ws_connected {
        return false;
    }

    // Rebuilds state.layout/bus_channel_start/object_ids_by_ms_channel from
    // state.console_architecture, and also resets init_seen_ms_channels/sends_mode
    // fields/input_send_state/the real-channel track cache -- deliberately re-done again just
    // below (harmless, idempotent) since those resets are also part of what this function
    // (formerly handle_ws_connected's own body) has always done on every connect.
    rebuild_layout_and_reset_caches(state);

    state.is_initializing = true;
    state.init_message_buffer.clear();
    state.init_seen_ms_channels.clear();
    state.has_sent_initial_track_dump = false;
    state.sends_mode_ms_send_index = None;
    state.sends_mode_subscribed_ms_send_index = None;
    state.input_send_state.clear();
    state.metered_object_ids = HashSet::new();
    state.metering2_param_ms_channels = Vec::new();
    state.meter_db_cache = MeterDbCache::new();
    // Unconditional on every connect, including an auto-reconnect with no Stop in between:
    // sends mode is reset to `None` above, so any mirrored `send1..6` fields left on cached
    // Bus/Main tracks would otherwise ship to Console 1 in the full dump that the
    // `has_sent_initial_track_dump = false` reset forces.
    state.track_cache.reset_real_channels(START_SLOT_OBJECT_ID);

    subscribe_to_required_channel_data(state);
    refresh_dsp_fields_for_real_channels(state);

    state.osd_enabled = false;
    send_sysex_and_log(
        midi_tx,
        event_tx,
        state.config.log_json,
        &json!({"handshake": {"dawName": "Mixing Station", "protocolVersion": [1, 2]}}),
    );
    let _ = event_tx.send(BridgeEvent::Log("Console 1 handshake sent.".to_string()));

    true
}

fn is_bus_or_main(kind: LayoutSlotKind) -> bool {
    matches!(kind, LayoutSlotKind::Bus | LayoutSlotKind::Main)
}

/// Per-slot MS value overrides applied before `ms_param_apply`: Main's name is fixed, Bus/Main
/// colors are Console 1-only, stereo-pair names lose their `L`/`R` suffix, and pan is either
/// ignored (stereo-linked pairs drive pan from the Console 1 hybrid knob) or forced to center.
///
/// `Err(())` means "fully handled, don't apply at all". Port of `index.js`'s
/// `getValueForApplyWithSlotOverrides`.
fn apply_slot_overrides(
    slot: &LayoutSlot,
    param_path: &str,
    value: serde_json::Value,
    config: &RuntimeConfig,
) -> Result<serde_json::Value, ()> {
    let mut value_out = value;

    if param_path == "cfg.name" {
        if slot.kind == LayoutSlotKind::Main {
            value_out = json!("Main");
        } else if slot.pan_locked && slot.ms_channels.len() == 2 {
            if let Some(s) = value_out.as_str() {
                value_out = json!(trim_stereo_suffix_from_name(s));
            }
        }
    }
    if param_path == "cfg.color" {
        if slot.kind == LayoutSlotKind::Main {
            value_out = json!(config.console1_main_color);
        } else if slot.kind == LayoutSlotKind::Bus {
            value_out = json!(config.console1_bus_color);
        }
    }
    if param_path == "mix.pan" && slot.pan_locked {
        if slot.ms_channels.len() == 2 {
            return Err(());
        }
        value_out = json!(0.5);
    }

    Ok(value_out)
}

/// Applies one channel update to one layout slot — the innermost step of the dispatch pipeline,
/// called once per affected `object_id` for a given inbound message. Queues whatever changed.
/// `suppress` disables queueing (echo suppression) but the update still lands in the cache,
/// matching JS's `applyChannelUpdateToSlot`.
#[allow(clippy::too_many_arguments)]
fn apply_channel_update_to_slot(
    state: &mut BridgeState,
    object_id: usize,
    channel_index: i64,
    param_path: &str,
    format: MsFormat,
    value: &serde_json::Value,
    suppress: bool,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) {
    let Some(slot) = state.layout.get(object_id).cloned() else {
        return;
    };
    // Stereo pairs listen only on their primary channel, so an L/R pair can't fight itself.
    if let Some(primary) = slot.ms_primary {
        if channel_index as usize != primary {
            return;
        }
    }
    // Bus/Main tracks always behave like standard tracks, so send-slot state never shows on them.
    if is_bus_or_main(slot.kind) && param_path.starts_with("mix.sends.") {
        return;
    }
    // Bus masters ignore Solo from both directions.
    if slot.kind == LayoutSlotKind::Bus && param_path == "solo" {
        return;
    }

    if let Some(next_sends_mode) =
        maybe_latch_sends_mode_from_selection(&slot, param_path, value, state.bus_channel_start)
    {
        set_sends_mode(state, next_sends_mode, event_tx);
    }

    remember_input_standard_state(state, &slot, param_path, value);

    let mut track = clone_or_create_track(state, object_id, &slot);

    let sends_outcome = handle_sends_mode_input_update(SendsModeInputUpdateArgs {
        slot: &slot,
        channel_index: channel_index as usize,
        param_path,
        value,
        sends_mode_ms_send_index: state.sends_mode_ms_send_index,
        mapping: &state.send_mapping,
        track: &mut track,
        input_send_state: &mut state.input_send_state,
    });
    match sends_outcome {
        SendsModeInputUpdateOutcome::Suppressed => {
            store_track(state, object_id, track);
            return;
        }
        SendsModeInputUpdateOutcome::Applied { changed } => {
            let track_id = track.track_id.clone();
            store_track(state, object_id, track);
            if !suppress && !changed.is_empty() {
                state.update_queue.queue(track_id, changed, false);
            }
            return;
        }
        SendsModeInputUpdateOutcome::NotHandled => {}
    }

    let Some(result) =
        apply_ms_param_with_overrides(state, &mut track, &slot, param_path, Some(format), value)
    else {
        store_track(state, object_id, track);
        return;
    };
    let mut changed = result.changed;

    // In sends mode Console 1 reads fader position out of the send slots, so Bus/Main faders
    // have to be proxied through them to keep displaying their real level.
    if state.sends_mode_ms_send_index.is_some()
        && is_bus_or_main(slot.kind)
        && param_path == "mix.lvl"
    {
        // `volume` is cloned out first: `&mut track` and `&track.volume` in one call would be
        // conflicting borrows of the same value.
        let volume = track.volume.clone();
        changed.extend(mirror_console1_send_slots_from_volume(&mut track, &volume));
    }

    let track_id = track.track_id.clone();
    store_track(state, object_id, track);

    if !suppress {
        if !changed.is_empty() {
            state.update_queue.queue(track_id.clone(), changed, false);
        }
        for (field, value) in result.bare_field_updates {
            state
                .bare_update_queue
                .queue(track_id.clone(), field, value);
        }
    }
}

/// Write a locally-mutated track clone back into the cache. The slot is always already
/// present — every caller fetches it via `clone_or_create_track` first.
fn store_track(state: &mut BridgeState, object_id: usize, track: TrackInfo) {
    if let Some(cached) = state.track_cache.get_mut(object_id) {
        *cached = track;
    }
}

/// Take an owned copy of a slot's cached track, creating its default state on first access.
/// Callers mutate the copy and hand it back via `store_track` — the cache can't be held
/// borrowed across the mutation because applying an update also reads other `state` fields.
fn clone_or_create_track(
    state: &mut BridgeState,
    object_id: usize,
    slot: &LayoutSlot,
) -> TrackInfo {
    let colors = default_colors(&state.config);
    let bus_channel_start = state.bus_channel_start;
    state
        .track_cache
        .get_or_create(
            object_id,
            slot,
            state.lifecycle,
            &colors,
            bus_channel_start,
            &mut state.rng,
        )
        .clone()
}

/// Apply one Mixing Station parameter to a track: per-slot overrides, then `ms_param_apply`.
/// Returns `None` when the overrides consumed the update entirely (a stereo-linked pan echo).
///
/// Deliberately free of sends-mode handling and queueing. `apply_channel_update_to_slot` layers
/// those on top for live messages; the initialization replay must have neither, matching JS's
/// `finalizeInitialization` — see its call site for why.
fn apply_ms_param_with_overrides(
    state: &BridgeState,
    track: &mut TrackInfo,
    slot: &LayoutSlot,
    param_path: &str,
    format: Option<MsFormat>,
    value: &serde_json::Value,
) -> Option<MsParamApplyResult> {
    let value_for_apply =
        apply_slot_overrides(slot, param_path, value.clone(), &state.config).ok()?;
    let ms_format = format.map(|f| match f {
        MsFormat::Val => MsValueFormat::Val,
        MsFormat::Norm => MsValueFormat::Norm,
    });
    Some(apply_ms_param_to_track_with_send_mapping(
        track,
        slot,
        state.bus_channel_start,
        param_path,
        &value_for_apply,
        ms_format,
        &state.send_mapping,
    ))
}

/// Record an input channel's true standard `mix.on`/`mix.pan`, whatever mode is active.
/// Port of `index.js`'s `maybeUpdateInputStdState` — called unconditionally so the cache stays
/// truthful while sends mode overwrites the displayed values.
fn remember_input_standard_state(
    state: &mut BridgeState,
    slot: &LayoutSlot,
    param_path: &str,
    value: &serde_json::Value,
) {
    if slot.kind != LayoutSlotKind::Input {
        return;
    }
    let Some(primary) = slot.ms_primary else {
        return;
    };
    let entry = state.input_std_state.entry(primary).or_default();
    match param_path {
        "mix.on" => entry.0 = Some(value.clone()),
        "mix.pan" => entry.1 = Some(value.clone()),
        _ => {}
    }
}

/// Restore every input track's standard mute/pan from `input_std_state` when leaving sends
/// mode, so Console 1 stops showing send-derived values immediately rather than after Mixing
/// Station answers the refresh GETs. Port of `applyStandardMutePanToInputsFromCache`.
fn apply_standard_mute_pan_to_inputs_from_cache(state: &mut BridgeState) {
    let input_object_ids: Vec<usize> = state
        .layout
        .iter()
        .filter(|slot| slot.kind == LayoutSlotKind::Input && slot.ms_primary.is_some())
        .map(|slot| slot.object_id)
        .collect();

    for object_id in input_object_ids {
        let slot = state.layout[object_id].clone();
        let Some(primary) = slot.ms_primary else {
            continue;
        };
        // Only tracks that already exist are restored — an absent one has nothing stale on
        // screen to correct, and creating it here would be a side effect JS doesn't have.
        let Some(mut track) = state.track_cache.get(object_id).cloned() else {
            continue;
        };
        let (cached_on, cached_pan) = state
            .input_std_state
            .get(&primary)
            .cloned()
            .unwrap_or_default();

        let mut changed = HashMap::new();
        if let Some(mix_on) = cached_on {
            if let Some(result) =
                apply_ms_param_with_overrides(state, &mut track, &slot, "mix.on", None, &mix_on)
            {
                changed.extend(result.changed);
            }
        }
        // Stereo-linked pairs are left alone: their pan is a Console 1 hybrid control, not
        // something Mixing Station owns. `apply_slot_overrides` centers the pan-locked rest.
        let pan_value = if slot.pan_locked && slot.ms_channels.len() == 2 {
            None
        } else if slot.pan_locked {
            Some(json!(0.5))
        } else {
            cached_pan
        };
        if let Some(pan) = pan_value {
            if let Some(result) =
                apply_ms_param_with_overrides(state, &mut track, &slot, "mix.pan", None, &pan)
            {
                changed.extend(result.changed);
            }
        }

        let track_id = track.track_id.clone();
        store_track(state, object_id, track);
        if !changed.is_empty() {
            state.update_queue.queue(track_id, changed, false);
        }
    }
}

/// Enter or leave sends mode: re-point every Bus/Main track's send-slot display, swap the
/// active send's WS subscriptions, and re-request the values the new mode displays.
/// Port of `index.js`'s `setSendsMode`.
fn set_sends_mode(
    state: &mut BridgeState,
    next: Option<usize>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) {
    if next == state.sends_mode_ms_send_index {
        return;
    }
    match next {
        Some(index) => {
            let _ = event_tx.send(BridgeEvent::Log(format!(
                "[Mode] SENDS active (msSendIndex={index}, bus={})",
                index + 1
            )));
        }
        None => {
            let _ = event_tx.send(BridgeEvent::Log("[Mode] STANDARD active".to_string()));
        }
    }
    state.sends_mode_ms_send_index = next;
    state.input_send_state.clear();

    let bus_main_object_ids: Vec<usize> = state
        .layout
        .iter()
        .filter(|slot| is_bus_or_main(slot.kind))
        .map(|slot| slot.object_id)
        .collect();
    for object_id in bus_main_object_ids {
        let slot = state.layout[object_id].clone();
        let mut track = clone_or_create_track(state, object_id, &slot);
        let changed = if next.is_some() {
            // Cloned out first for the same borrow reason as in `apply_channel_update_to_slot`.
            let volume = track.volume.clone();
            mirror_console1_send_slots_from_volume(&mut track, &volume)
        } else {
            clear_console1_send_slots(&mut track)
        };
        let track_id = track.track_id.clone();
        store_track(state, object_id, track);
        if !changed.is_empty() {
            state.update_queue.queue(track_id, changed, false);
        }
    }

    // Keep WS subscriptions minimal: only the active send index is ever subscribed.
    if let Some(previous) = state.sends_mode_subscribed_ms_send_index.take() {
        unsubscribe_from_channel_data(state, &format!("ch.*.mix.sends.{previous}.on"), "val");
        unsubscribe_from_channel_data(state, &format!("ch.*.mix.sends.{previous}.pan"), "norm");
    }
    match next {
        Some(index) => {
            subscribe_to_channel_data(state, &format!("ch.*.mix.sends.{index}.on"), "val");
            subscribe_to_channel_data(state, &format!("ch.*.mix.sends.{index}.pan"), "norm");
            state.sends_mode_subscribed_ms_send_index = Some(index);
            for ch in 0..state.console_architecture.input_channel_count as usize {
                request_mixing_station_value(
                    state,
                    &format!("ch.{ch}.mix.sends.{index}.on"),
                    "val",
                );
                request_mixing_station_value(
                    state,
                    &format!("ch.{ch}.mix.sends.{index}.pan"),
                    "norm",
                );
            }
        }
        None => {
            // Restore from cache first so Console 1 corrects immediately; the GETs below then
            // reconcile against whatever Mixing Station currently holds.
            apply_standard_mute_pan_to_inputs_from_cache(state);
            for ch in 0..state.console_architecture.input_channel_count as usize {
                request_mixing_station_value(state, &format!("ch.{ch}.mix.on"), "val");
                request_mixing_station_value(state, &format!("ch.{ch}.mix.pan"), "norm");
            }
        }
    }
}

/// Dispatches one inbound Mixing Station WebSocket message. Port of `index.js`'s
/// `handleWSMessage`. Returns `true` to tell the caller to arm the init-flush timer (the
/// caller owns that timer future) -- only the `/console/information` reply branch, via
/// `finish_ws_connect`, can produce `true`; every other branch returns `false`.
fn handle_ws_message(
    state: &mut BridgeState,
    text: &str,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) -> bool {
    let Ok(msg) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    let Some(path) = msg.get("path").and_then(|v| v.as_str()) else {
        return false;
    };

    // Console architecture discovery reply -- see `handle_ws_connected`/`finish_ws_connect`.
    // Ignored if we're not actually waiting on one (e.g. a stray/duplicate reply arriving
    // after the timeout already fired, or after a newer connect's own request superseded
    // this one), matching JS's `consoleInfoRequestState.pending` guard.
    if path == "/console/information" {
        if state.console_info_pending {
            state.console_info_pending = false;
            let body = msg.get("body").cloned().unwrap_or(Value::Null);
            state.console_architecture =
                apply_console_information(&body, state.console_architecture.clone());
            if state.config.log_json {
                let a = &state.console_architecture;
                let main = a
                    .main_stereo_channels
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                let _ = event_tx.send(BridgeEvent::Log(format!(
                    "[ConsoleInfo] total={} inputs=0..{} bus={}..{} main={main}",
                    a.total_channels,
                    a.input_channel_count - 1,
                    a.bus_channel_start,
                    a.bus_channel_start + a.bus_channel_count - 1,
                )));
            }
            return finish_ws_connect(state, midi_tx, event_tx);
        }
        return false;
    }

    // Metering pushes are recognised here so they can't fall through into channel-data
    // parsing. The subscription itself is triggered by Console 1's `activeMeters` control
    // message (`handle_active_meters_message`, C1→bridge direction) -- this branch only
    // handles the resulting inbound MS→bridge pushes.
    if path.starts_with("/console/metering2/") {
        let id: Option<i64> = path.rsplit('/').next().and_then(|s| s.parse().ok());
        if id != Some(METERING2_SUBSCRIPTION_ID) {
            return false;
        }
        if state.metering2_param_ms_channels.is_empty() {
            return false;
        }
        let body = msg.get("body").cloned().unwrap_or(Value::Null);
        let param_count = state.metering2_param_ms_channels.len();
        let Some(max_db_by_param) = normalize_metering2_values(&body, param_count) else {
            return false;
        };

        let count = max_db_by_param
            .len()
            .min(state.metering2_param_ms_channels.len());
        let updates: Vec<(i64, f64)> = (0..count)
            .map(|i| (state.metering2_param_ms_channels[i], max_db_by_param[i]))
            .collect();
        let changed_channels = state.meter_db_cache.apply_updates(&updates);

        if !changed_channels.is_empty() {
            apply_meter_updates_for_ms_channels(state, &changed_channels);
        }
        return false;
    }

    let Some(parsed) = parse_channel_data_get_message(&msg) else {
        return false;
    };
    if parsed.channel_index < 0 {
        return false;
    }
    let Some(object_ids) = state
        .object_ids_by_ms_channel
        .get(&(parsed.channel_index as usize))
        .cloned()
    else {
        return false;
    };

    for &object_id in &object_ids {
        ensure_track_for_object_id(state, object_id);
    }

    if state.is_initializing {
        state.init_message_buffer.push(BufferedInitUpdate {
            channel_index: parsed.channel_index,
            param_path: parsed.param_path.clone(),
            value: parsed.value.clone(),
            format: parsed.format,
        });
        state.init_seen_ms_channels.insert(parsed.channel_index);
        if should_finalize_initialization_early(state) {
            finalize_initialization(state, midi_tx, event_tx, "received all layout channels");
        }
        return false;
    }

    // Computed once per message, not per affected slot — every slot listening to this channel
    // sees the same echo verdict.
    let echo_key = format!(
        "ch.{}.{}|{}",
        parsed.channel_index,
        parsed.param_path,
        ms_format_name(parsed.format)
    );
    let suppress =
        state
            .echo_tracker
            .should_suppress(&echo_key, &parsed.value, std::time::Instant::now());

    for &object_id in &object_ids {
        apply_channel_update_to_slot(
            state,
            object_id,
            parsed.channel_index,
            &parsed.param_path,
            parsed.format,
            &parsed.value,
            suppress,
            event_tx,
        );
    }
    false
}

fn ms_format_name(format: MsFormat) -> &'static str {
    match format {
        MsFormat::Val => "val",
        MsFormat::Norm => "norm",
    }
}

fn ensure_track_for_object_id(state: &mut BridgeState, object_id: usize) {
    let Some(slot) = state.layout.get(object_id).cloned() else {
        return;
    };
    let colors = default_colors(&state.config);
    let bus_channel_start = state.bus_channel_start;
    state.track_cache.get_or_create(
        object_id,
        &slot,
        state.lifecycle,
        &colors,
        bus_channel_start,
        &mut state.rng,
    );
}

/// True once every MS channel the layout references has reported at least one value — the
/// initialization window can then close without waiting out `INIT_FLUSH_TIMEOUT`.
fn should_finalize_initialization_early(state: &BridgeState) -> bool {
    state.init_seen_ms_channels.len() >= state.object_ids_by_ms_channel.len()
}

/// End the initialization buffering window: replay everything buffered into the track cache,
/// then send one full forced track dump. Port of `index.js`'s `finalizeInitialization`.
fn finalize_initialization(
    state: &mut BridgeState,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
    reason: &str,
) {
    if !state.is_initializing {
        return;
    }
    state.is_initializing = false;

    for object_id in 0..state.layout.len() {
        ensure_track_for_object_id(state, object_id);
    }

    // Replay is cache-population only — deliberately NOT routed through
    // `apply_channel_update_to_slot`. Sends-mode latching must not run here: Mixing Station
    // reports whatever channel was already `selected` in its own UI at connect time, and
    // latching off that would boot the bridge straight into sends mode with no Console 1
    // action behind it. Queueing is skipped for the same reason JS skips it — the forced full
    // dump below supersedes any incremental update.
    let buffered = std::mem::take(&mut state.init_message_buffer);
    for update in buffered {
        let Some(object_ids) = state
            .object_ids_by_ms_channel
            .get(&(update.channel_index as usize))
            .cloned()
        else {
            continue;
        };
        for object_id in object_ids {
            let Some(slot) = state.layout.get(object_id).cloned() else {
                continue;
            };
            // Stereo pairs read only from their primary channel, same as live dispatch.
            if let Some(primary) = slot.ms_primary {
                if update.channel_index as usize != primary {
                    continue;
                }
            }
            let mut track = clone_or_create_track(state, object_id, &slot);
            apply_ms_param_with_overrides(
                state,
                &mut track,
                &slot,
                &update.param_path,
                Some(update.format),
                &update.value,
            );
            store_track(state, object_id, track);
        }
    }

    if !state.has_sent_initial_track_dump {
        let _ = event_tx.send(BridgeEvent::Log(format!(
            "Finalizing initialization ({reason}). Sending initial track dump..."
        )));
        let tracks: Vec<serde_json::Value> = state
            .track_cache
            .iter()
            .map(|(_, track)| track_info_to_trackbatch_json(track))
            .collect();
        for chunk in tracks.chunks(FULL_TRACK_BATCH_CHUNK_SIZE) {
            send_sysex_and_log(midi_tx, event_tx, state.config.log_json, &json!({ "trackBatch": chunk }));
        }
        state.has_sent_initial_track_dump = true;
    }
}

fn queued_update_to_json(
    track_id: &str,
    fields: &HashMap<String, serde_json::Value>,
) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("trackId".to_string(), json!(track_id));
    for (field, value) in fields {
        obj.insert(field.clone(), value.clone());
    }
    serde_json::Value::Object(obj)
}

fn send_track_batches(
    entries: &[serde_json::Value],
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    log_json: bool,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) {
    for chunk in entries.chunks(FULL_TRACK_BATCH_CHUNK_SIZE) {
        send_sysex_and_log(midi_tx, event_tx, log_json, &json!({ "trackBatch": chunk }));
    }
}

/// Flush the batched `trackBatch` queue. Forced entries always go out; non-forced entries stay
/// queued until a flush happens to run while OSD is enabled, accumulating with later updates
/// for the same track. Port of `queueConsole1TrackUpdate`'s flush body.
fn flush_update_queue(
    state: &mut BridgeState,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) {
    let forced: Vec<serde_json::Value> = state
        .update_queue
        .take_forced()
        .iter()
        .map(|(track_id, update)| queued_update_to_json(track_id, &update.fields))
        .collect();
    send_track_batches(&forced, midi_tx, state.config.log_json, event_tx);

    if state.osd_enabled {
        let normal: Vec<serde_json::Value> = state
            .update_queue
            .take_all()
            .iter()
            .map(|(track_id, update)| queued_update_to_json(track_id, &update.fields))
            .collect();
        send_track_batches(&normal, midi_tx, state.config.log_json, event_tx);
    }
}

/// Flush the batched bare DSP-field queue. Unlike `flush_update_queue`, entries are always
/// drained here and simply dropped when OSD is disabled — never re-queued, matching
/// `queueConsole1BareFieldUpdate`'s unconditional `clear()` before its OSD check.
fn flush_bare_update_queue(
    state: &mut BridgeState,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) {
    let entries = state.bare_update_queue.take_all();
    if !state.osd_enabled {
        return;
    }
    for entry in entries {
        let real_val = state
            .track_cache
            .object_id_for_track_id(&entry.track_id)
            .and_then(|object_id| state.track_cache.get(object_id))
            .and_then(|track| track.dsp_real_values.get(&entry.field))
            .map(|v| json!(v));
        let payload = crate::bare_update_queue::build_bare_field_sysex_payload(
            &entry.field,
            &entry.value,
            real_val.as_ref(),
        );
        let mut track_obj = serde_json::Map::new();
        track_obj.insert(
            "trackId".to_string(),
            serde_json::Value::String(entry.track_id),
        );
        track_obj.insert(entry.field, payload);
        send_sysex_and_log(
            midi_tx,
            event_tx,
            state.config.log_json,
            &serde_json::Value::Object(track_obj),
        );
    }
}

/// Flush the batched outbound Mixing Station write queue. Port of `queueMsWrite`'s flush-timer
/// body. Entries are always drained; if the WS isn't connected the whole batch is silently
/// dropped rather than re-queued, matching JS's `msWriteQueue.clear(); return;` — and matching
/// `flush_bare_update_queue`'s same "drain regardless, conditionally send" shape.
fn flush_ms_write_queue(state: &mut BridgeState) {
    let entries = state.ms_write_queue.take_all();
    // Gated on a live connection, not merely on the engine existing: while the engine is
    // making its initial connect attempt or waiting out a reconnect, `ws_engine` silently
    // drops every outbound send. Calling `note_write` for a frame that was never transmitted
    // would arm a 150ms echo-suppression window against nothing, so a genuine Mixing Station
    // update that happened to carry a matching value could be swallowed as a false echo.
    if !state.ws_connected {
        return;
    }
    for entry in entries {
        // The bridge's own writes are recorded here so Mixing Station's echo of them can be
        // recognised and suppressed in `handle_ws_message` — same `<path>|<format>` key shape
        // that side builds.
        state.echo_tracker.note_write(
            &format!("{}|{}", entry.path, entry.format),
            entry.value.clone(),
            std::time::Instant::now(),
        );
        ws_send_json(
            state,
            build_ms_set_request(&entry.path, &entry.format, &entry.value),
        );
    }
}

/// Arm each queue's debounce timer if it has work and isn't already armed. Called only from
/// the event-loop arms that can enqueue — deliberately *not* from the flush arms themselves,
/// so entries left behind by an OSD-disabled flush wait for the next real update instead of
/// re-arming the timer every 20ms forever (matches JS, where only `queueConsole1TrackUpdate`
/// starts the timer).
fn arm_queue_flush_timers(
    state: &BridgeState,
    update_flush_armed: &mut bool,
    update_flush_timer: std::pin::Pin<&mut tokio::time::Sleep>,
    bare_flush_armed: &mut bool,
    bare_flush_timer: std::pin::Pin<&mut tokio::time::Sleep>,
    ms_write_flush_armed: &mut bool,
    ms_write_flush_timer: std::pin::Pin<&mut tokio::time::Sleep>,
) {
    if !*update_flush_armed && !state.update_queue.is_empty() {
        *update_flush_armed = true;
        update_flush_timer.reset(tokio::time::Instant::now() + CONSOLE1_FLUSH_DELAY);
    }
    if !*bare_flush_armed && !state.bare_update_queue.is_empty() {
        *bare_flush_armed = true;
        bare_flush_timer.reset(tokio::time::Instant::now() + CONSOLE1_BARE_FLUSH_DELAY);
    }
    if !*ms_write_flush_armed && !state.ms_write_queue.is_empty() {
        *ms_write_flush_armed = true;
        ms_write_flush_timer.reset(tokio::time::Instant::now() + MS_WRITE_FLUSH_DELAY);
    }
}

/// Returns `true` when the caller must arm the init-flush timer — see `enter_running_state`.
fn handle_hardware_trigger(
    trigger: HardwareTrigger,
    state: &mut BridgeState,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) -> bool {
    match trigger {
        HardwareTrigger::Start => enter_running_state(state, midi_tx, event_tx),
        HardwareTrigger::Stop => {
            enter_standby_state(state, midi_tx, event_tx);
            false
        }
    }
}

/// Returns `true` when the caller must arm the init-flush timer at `FORCE_RESYNC_FLUSH_DELAY`
/// (the caller owns that timer), the same contract as `handle_hardware_trigger`/
/// `handle_ws_connected`.
#[must_use]
fn handle_control_message(
    action: ControlAction,
    state: &mut BridgeState,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) -> bool {
    match action {
        ControlAction::EnableOsd => {
            state.osd_enabled = true;
            let _ = event_tx.send(BridgeEvent::Log("Enabling OSD.".to_string()));
            false
        }
        ControlAction::DisableOsd => {
            state.osd_enabled = false;
            let _ = event_tx.send(BridgeEvent::Log("Disabling OSD.".to_string()));
            false
        }
        ControlAction::ResendHandshake => {
            state.osd_enabled = false;
            send_sysex_and_log(
                midi_tx,
                event_tx,
                state.config.log_json,
                &json!({"handshake": {"dawName": "Mixing Station", "protocolVersion": [1, 2]}}),
            );
            false
        }
        ControlAction::ReaffirmStatusBank => {
            send_status_bank_tracks(state, midi_tx, event_tx);
            apply_start_slot_display(state, midi_tx, event_tx);
            false
        }
        // Port of JS's `finalizeInitialization("handshake ack")` call inside
        // `handleConsole1ControlJson`'s handshake-ack branch (index.js:3054-3063).
        // `finalize_initialization` is already fully self-contained -- it no-ops via its own
        // `is_initializing` guard when there's nothing pending to finalize -- so this needs no
        // new machinery, just the call this branch was previously skipping in favor of a log
        // line. Runs synchronously, so the caller never needs to arm a timer for this action.
        ControlAction::FinalizeInitialization => {
            finalize_initialization(state, midi_tx, event_tx, "handshake ack");
            false
        }
        // Port of JS's `scheduleConsole1FullResync` (index.js:2595-2611): re-arms a fresh
        // initialization window and tells the caller to schedule `finalize_initialization` at
        // `FORCE_RESYNC_FLUSH_DELAY` -- the exact same "arm a resync soon" sequence
        // `apply_config_patch` already uses for a different trigger (a config change applied
        // while disconnected, `runtime.rs:505-511`). If a resync window is already open, JS's
        // own early return means "let the existing timer handle it" -- don't clobber one
        // already in flight.
        ControlAction::ScheduleFullResync => {
            if state.is_initializing {
                return false;
            }
            let _ = event_tx.send(BridgeEvent::Log(
                "[Lifecycle] Scheduling a full Console 1 resync.".to_string(),
            ));
            state.is_initializing = true;
            state.init_message_buffer.clear();
            state.has_sent_initial_track_dump = false;
            true
        }
    }
}

/// Port of `index.js`'s `handleStatusOrStartSlotMidiMessage`. Status/Start bank slots aren't
/// backed by any Mixing Station channel, so no MS writes are ever produced here.
///
/// For the Start slot specifically: detects the hardware Start/Stop trigger, then works around
/// a real Console 1 firmware quirk confirmed via direct hardware testing — pushing
/// `selected:false` back to the just-triggered Start slot's own trackId alone does NOT clear
/// its selection latch on real hardware. Console 1 tracks "currently selected object" as its
/// own mutually-exclusive latch that only moves when a *different* object gets selected. So:
/// force-deselect the Start slot, then force-select a neighboring status-bank slot
/// (`START_SLOT_OBJECT_ID - 2`, the last status indicator — it must be an ACTIVE slot; the
/// empty spacer immediately before Start was confirmed NOT to work as the unlatch target).
fn handle_status_or_start_slot_midi_message(
    parsed: &serde_json::Value,
    slot: &LayoutSlot,
    state: &mut BridgeState,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) -> bool {
    if slot.kind != LayoutSlotKind::Start {
        return false;
    }
    let selected = parsed
        .get("selected")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let Some(trigger) = hardware_trigger_type_for(state.lifecycle, &selected) else {
        return false;
    };
    let _ = event_tx.send(BridgeEvent::Log(format!(
        "[Lifecycle] hardware trigger: {trigger:?}"
    )));
    let arm_init_flush = handle_hardware_trigger(trigger, state, midi_tx, event_tx);

    let start_track_id = clone_or_create_track(state, slot.object_id, slot).track_id;
    if let Some(track) = state.track_cache.get_mut(slot.object_id) {
        track.selected = false;
    }
    state.update_queue.queue(
        start_track_id,
        HashMap::from([("selected".to_string(), json!(false))]),
        true,
    );

    let neighbor_object_id = START_SLOT_OBJECT_ID - 2;
    let neighbor_slot = state.layout[neighbor_object_id].clone();
    let neighbor_track_id =
        clone_or_create_track(state, neighbor_object_id, &neighbor_slot).track_id;
    if let Some(track) = state.track_cache.get_mut(neighbor_object_id) {
        track.selected = true;
    }
    state.update_queue.queue(
        neighbor_track_id,
        HashMap::from([("selected".to_string(), json!(true))]),
        true,
    );

    arm_init_flush
}

/// Dispatches one inbound Console 1 SysEx message. Control messages route to
/// `handle_control_message`; status/Start bank slots to the handler above; everything else runs
/// the full 10-handler mixer/DSP fan-out, in `index.js`'s exact `handleMidiMessage` order.
///
/// Returns `true` when the caller must arm the init-flush timer at `FORCE_RESYNC_FLUSH_DELAY` --
/// either because a hardware Start trigger entered Running (see `enter_running_state`), or
/// because a dispatched control message (currently only `ControlAction::ScheduleFullResync`)
/// requested a fresh resync window.
fn handle_inbound_midi_message(
    raw: &[u8],
    state: &mut BridgeState,
    midi_tx: &std::sync::mpsc::Sender<MidiCommand>,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) -> bool {
    let Some(parsed) = parse_sysex_json(raw) else {
        return false;
    };
    if state.config.log_json {
        let _ = event_tx.send(BridgeEvent::Log(format!(
            "Received SysEx JSON from MIDI input: {parsed}"
        )));
    }

    if let Some(control_msg) = parse_control_message(&parsed) {
        // RESET/ENABLE/DISABLE all `return` before ever reaching `activeMeters` in JS
        // (`index.js:3023-3079`) -- a handshake ack does NOT, so `activeMeters` can fire
        // alongside a `HandshakeAck` in the same message, but never alongside these three.
        let skip_active_meters = matches!(
            control_msg,
            ParsedControlMessage::Reset
                | ParsedControlMessage::Enable
                | ParsedControlMessage::Disable
        );
        let mut must_arm_init_flush = false;
        for action in decide_control_actions(
            control_msg,
            state.lifecycle,
            state.has_sent_initial_track_dump,
        ) {
            if handle_control_message(action, state, midi_tx, event_tx) {
                must_arm_init_flush = true;
            }
        }
        if !skip_active_meters {
            if let Some(track_ids) = parse_active_meters(&parsed) {
                handle_active_meters_message(track_ids, state);
            }
        }
        return must_arm_init_flush;
    }
    if let Some(track_ids) = parse_active_meters(&parsed) {
        // Neither a recognized `cmd` nor a handshake ack matched, but `activeMeters` is
        // present on its own -- matches JS's independent-if fall-through (no `cmd`/`handshake`
        // branch matched, `if (parsed.activeMeters)` is still checked).
        handle_active_meters_message(track_ids, state);
        return false;
    }

    let Some(track_id) = parsed.get("trackId").and_then(|v| v.as_str()) else {
        return false;
    };
    let Some(object_id) = state.track_cache.object_id_for_track_id(track_id) else {
        return false;
    };
    let Some(slot) = state.layout.get(object_id).cloned() else {
        return false;
    };

    // Handled before the MS-channel-driven dispatch below, which assumes a real `ms_primary`
    // and would otherwise silently drop these on the `ms_primary` guard.
    if matches!(
        slot.kind,
        LayoutSlotKind::Status { .. } | LayoutSlotKind::Start
    ) {
        return handle_status_or_start_slot_midi_message(&parsed, &slot, state, midi_tx, event_tx);
    }
    if slot.kind == LayoutSlotKind::Empty {
        return false;
    }
    let Some(primary_channel) = slot.ms_primary else {
        return false;
    };

    let bus_channel_start = state.bus_channel_start;
    let mut track = clone_or_create_track(state, object_id, &slot);
    let track_id = track.track_id.clone();

    let mut writes: Vec<MsWrite> = Vec::new();

    let changed = handle_midi_bus_main_sends_mode_fader_proxy(
        &parsed,
        &slot,
        &mut track,
        state.sends_mode_ms_send_index,
        &mut writes,
    );
    if !changed.is_empty() {
        state.update_queue.queue(track_id.clone(), changed, false);
    }

    let changed = handle_midi_volume_update(
        &parsed,
        &slot,
        &mut track,
        state.sends_mode_ms_send_index,
        &mut writes,
    );
    if !changed.is_empty() {
        state.update_queue.queue(track_id.clone(), changed, false);
    }

    let changed = handle_midi_mute_update(
        &parsed,
        &slot,
        &mut track,
        state.sends_mode_ms_send_index,
        &state.send_mapping,
        &mut writes,
    );
    if !changed.is_empty() {
        state.update_queue.queue(track_id.clone(), changed, false);
    }

    let changed = handle_midi_solo_update(&parsed, &slot, &mut track, &mut writes);
    if !changed.is_empty() {
        state.update_queue.queue(track_id.clone(), changed, false);
    }

    let changed = handle_midi_pan_update(
        &parsed,
        &slot,
        &mut track,
        primary_channel,
        state.sends_mode_ms_send_index,
        &state.send_mapping,
        &mut writes,
        state.config.log_json,
        event_tx,
    );
    if !changed.is_empty() {
        state.update_queue.queue(track_id.clone(), changed, false);
    }

    let sends_mode_latch = handle_midi_selected_update(
        &parsed,
        &slot,
        primary_channel,
        bus_channel_start,
        &mut writes,
    );
    if let Some(next_sends_mode) = sends_mode_latch {
        // `set_sends_mode` reads and rewrites `state.track_cache` DIRECTLY for every Bus/Main
        // object_id (mirroring each one's volume into its send slots) — and the slot that just
        // latched sends mode IS a Bus/Main slot, so that includes this message's own
        // `object_id`. In JS `track` is a live cache reference, so that call both sees this
        // message's earlier mutations and has its own writes survive to the end. Here `track`
        // is a detached clone, so both halves have to be restored by hand: publish first so
        // the mirroring reads this message's fresh volume, then re-read so the writeback below
        // can't clobber what the mirroring just wrote.
        store_track(state, object_id, track);
        set_sends_mode(state, next_sends_mode, event_tx);
        track = clone_or_create_track(state, object_id, &slot);
    }

    let changed =
        handle_midi_send_slots_update(&parsed, &slot, &mut track, &state.send_mapping, &mut writes);
    if !changed.is_empty() {
        state.update_queue.queue(track_id.clone(), changed, false);
    }

    for (field, value) in handle_midi_filter_update(&parsed, &slot, &mut track, &mut writes) {
        state
            .bare_update_queue
            .queue(track_id.clone(), field, value);
    }

    for (field, value) in handle_midi_eq_update(&parsed, &slot, &mut track, &mut writes) {
        state
            .bare_update_queue
            .queue(track_id.clone(), field, value);
    }

    for (field, value) in handle_midi_comp_update(&parsed, &slot, &mut track, &mut writes) {
        state
            .bare_update_queue
            .queue(track_id.clone(), field, value);
    }

    store_track(state, object_id, track);

    for write in writes {
        let format = ms_format_name(write.format).to_string();
        state.ms_write_queue.queue(write.path, format, write.value);
    }

    false
}

/// Spawns the bridge runtime in `standby` (no WS opened yet, MIDI ports opened immediately),
/// matching `bridge-cli`'s historical startup behavior. Returns as soon as the task is
/// spawned — the MIDI engine's own port discovery blocks on its dedicated thread, not here.
pub fn spawn_bridge_runtime(initial_config: RuntimeConfig) -> BridgeRuntimeHandle {
    let (command_tx, mut command_rx) = mpsc::unbounded_channel::<BridgeCommand>();
    let (event_tx, event_rx) = mpsc::unbounded_channel::<BridgeEvent>();

    let join_handle = tokio::spawn(async move {
        let config = initial_config;
        let console_architecture = ConsoleArchitecture {
            total_channels: 80,
            input_channel_count: 32,
            bus_channel_start: 48,
            bus_channel_count: 16,
            main_stereo_channels: vec![70, 71],
        };
        let bus_channel_start = console_architecture.bus_channel_start as usize;
        let layout = build_default_layout(&config, &console_architecture);
        let object_ids_by_ms_channel = object_ids_by_ms_channel(&layout);
        let send_mapping = build_send_mapping(
            &config.c1_send_to_ms_bus_number,
            console_architecture.bus_channel_count as usize,
        );
        let mut state = BridgeState {
            lifecycle: Lifecycle::Standby,
            osd_enabled: false,
            layout,
            track_cache: TrackCache::new(),
            rng: StdRng::from_rng(&mut rand::rng()),
            config,
            bus_channel_start,
            console_architecture,
            console_info_pending: false,
            ws_handle: None,
            ws_connected: false,
            is_initializing: false,
            init_message_buffer: Vec::new(),
            has_sent_initial_track_dump: false,
            sends_mode_ms_send_index: None,
            sends_mode_subscribed_ms_send_index: None,
            input_send_state: InputSendState::new(),
            input_std_state: HashMap::new(),
            send_mapping,
            echo_tracker: EchoSuppressionTracker::new(),
            update_queue: UpdateQueue::new(),
            bare_update_queue: BareUpdateQueue::new(),
            ms_write_queue: MsWriteQueue::new(),
            object_ids_by_ms_channel,
            init_seen_ms_channels: HashSet::new(),
            metered_object_ids: HashSet::new(),
            metering2_param_ms_channels: Vec::new(),
            meter_db_cache: MeterDbCache::new(),
        };

        let (inbound_tx, mut inbound_rx) =
            tokio::sync::mpsc::unbounded_channel::<InboundMidiMessage>();
        let engine: MidiEngineHandle =
            spawn_midi_engine(DEFAULT_PREFERRED_PORT_NAMES, inbound_tx).await;

        enter_standby_state(&mut state, &engine.command_tx, &event_tx);

        tokio::pin! {
            // Parked far in the future until a connect arms it — `Sleep` has to already exist and
            // be pinned to be `reset()`-able in place, and an unarmed branch is `if`-guarded off
            // anyway, so the initial deadline is never actually observed. The three queue-flush
            // timers below work the same way, armed by their queues going non-empty.
            let init_flush_timer = tokio::time::sleep(std::time::Duration::from_secs(3600));
            let update_flush_timer = tokio::time::sleep(std::time::Duration::from_secs(3600));
            let bare_flush_timer = tokio::time::sleep(std::time::Duration::from_secs(3600));
            let ms_write_flush_timer = tokio::time::sleep(std::time::Duration::from_secs(3600));
            // Same parked-until-armed shape as the others, but guarded by
            // `state.console_info_pending` directly (set true/false by
            // `handle_ws_connected`/`handle_ws_message`) rather than a separate local bool --
            // that field already *is* the "is this timer meaningful" signal.
            let console_info_timeout = tokio::time::sleep(std::time::Duration::from_secs(3600));
        }
        let mut init_flush_armed = false;
        let mut update_flush_armed = false;
        let mut bare_flush_armed = false;
        let mut ms_write_flush_armed = false;

        loop {
            tokio::select! {
                cmd = command_rx.recv() => match cmd {
                    // Matches JS's `enterRunningState(config)`: the config is applied *before*
                    // anything else, and with the plain `applyRuntimeConfig` — the Start path
                    // never runs the resync-aware variant, so it emits no config events.
                    Some(BridgeCommand::Start(patch)) => {
                        if let Some(patch) = patch {
                            apply_runtime_config(&mut state, &patch);
                        }
                        if enter_running_state(&mut state, &engine.command_tx, &event_tx) {
                            init_flush_armed = true;
                            init_flush_timer.as_mut().reset(tokio::time::Instant::now() + FORCE_RESYNC_FLUSH_DELAY);
                        }
                    }
                    Some(BridgeCommand::Stop) => {
                        enter_standby_state(&mut state, &engine.command_tx, &event_tx)
                    }
                    // Standby-leak guard, matching JS's `config:apply` stdin handler: the
                    // resync path creates and activates real channel tracks, and
                    // `finalize_initialization` has no lifecycle check of its own, so a patch
                    // applied in standby would push a full track dump to hardware that should
                    // only be showing the status/Start bank. Callers are supposed to send this
                    // only while running, but that's an assumption about the caller — a caller
                    // whose own state has drifted could still send one, so guard here too.
                    Some(BridgeCommand::ConfigApply(patch)) => {
                        if state.lifecycle == Lifecycle::Running {
                            apply_config_patch(
                                &mut state,
                                &patch,
                                &event_tx,
                                &engine.command_tx,
                                &mut init_flush_timer.as_mut(),
                                &mut init_flush_armed,
                            ).await;
                        } else {
                            let _ = event_tx.send(BridgeEvent::Log(
                                "[Lifecycle] Ignoring config:apply -- not running.".to_string(),
                            ));
                        }
                    }
                    Some(BridgeCommand::StatusUpdate(snapshot)) => {
                        apply_live_status_colors(&mut state, &engine.command_tx, &snapshot, &event_tx);
                    }
                    Some(BridgeCommand::Shutdown) | None => break,
                },
                received = inbound_rx.recv() => match received {
                    Some((_stamp, message)) => {
                        if state.config.log_json {
                            let _ = event_tx.send(BridgeEvent::Log(format!(
                                "Received MIDI message: {message:?}"
                            )));
                        }
                        if handle_inbound_midi_message(&message, &mut state, &engine.command_tx, &event_tx) {
                            init_flush_armed = true;
                            init_flush_timer.as_mut().reset(tokio::time::Instant::now() + FORCE_RESYNC_FLUSH_DELAY);
                        }
                        arm_queue_flush_timers(
                            &state,
                            &mut update_flush_armed,
                            update_flush_timer.as_mut(),
                            &mut bare_flush_armed,
                            bare_flush_timer.as_mut(),
                            &mut ms_write_flush_armed,
                            ms_write_flush_timer.as_mut(),
                        );
                    }
                    None => break, // channel closed, MIDI thread exited
                },
                // Polled only while a WS engine exists; `pending()` parks this branch forever in
                // standby so `select!` just ignores it rather than spinning on a missing channel.
                ws_event = async {
                    match &mut state.ws_handle {
                        Some(handle) => handle.events.recv().await,
                        None => std::future::pending().await,
                    }
                } => match ws_event {
                    Some(WsEvent::Connected) => {
                        handle_ws_connected(&mut state, &event_tx);
                        console_info_timeout.as_mut().reset(tokio::time::Instant::now() + CONSOLE_INFO_TIMEOUT);
                    }
                    Some(WsEvent::Message(text)) => {
                        if handle_ws_message(&mut state, &text, &engine.command_tx, &event_tx) {
                            init_flush_armed = true;
                            init_flush_timer.as_mut().reset(tokio::time::Instant::now() + INIT_FLUSH_TIMEOUT);
                        }
                        arm_queue_flush_timers(
                            &state,
                            &mut update_flush_armed,
                            update_flush_timer.as_mut(),
                            &mut bare_flush_armed,
                            bare_flush_timer.as_mut(),
                            &mut ms_write_flush_armed,
                            ms_write_flush_timer.as_mut(),
                        );
                    }
                    Some(WsEvent::Disconnected) => {
                        let _ = event_tx.send(BridgeEvent::Log(
                            "Mixing Station WebSocket disconnected, will auto-reconnect.".to_string(),
                        ));
                        state.ws_connected = false;
                        // The initialization window belongs to the connection that just died —
                        // finalizing it against a socket that no longer exists would dump tracks
                        // built from a half-received snapshot. The next `Connected` reopens it.
                        state.is_initializing = false;
                        init_flush_armed = false;
                    }
                    None => {
                        // Engine task exited without us asking it to. Drop the handle so this
                        // branch goes dormant again instead of busy-looping on a closed channel.
                        state.ws_handle = None;
                        state.ws_connected = false;
                    }
                },
                _ = &mut init_flush_timer, if init_flush_armed => {
                    init_flush_armed = false;
                    finalize_initialization(&mut state, &engine.command_tx, &event_tx, "timeout");
                    arm_queue_flush_timers(
                        &state,
                        &mut update_flush_armed,
                        update_flush_timer.as_mut(),
                        &mut bare_flush_armed,
                        bare_flush_timer.as_mut(),
                        &mut ms_write_flush_armed,
                        ms_write_flush_timer.as_mut(),
                    );
                }
                _ = &mut update_flush_timer, if update_flush_armed => {
                    update_flush_armed = false;
                    flush_update_queue(&mut state, &engine.command_tx, &event_tx);
                }
                _ = &mut bare_flush_timer, if bare_flush_armed => {
                    bare_flush_armed = false;
                    flush_bare_update_queue(&mut state, &engine.command_tx, &event_tx);
                }
                _ = &mut ms_write_flush_timer, if ms_write_flush_armed => {
                    ms_write_flush_armed = false;
                    flush_ms_write_queue(&mut state);
                }
                _ = &mut console_info_timeout, if state.console_info_pending => {
                    state.console_info_pending = false;
                    if finish_ws_connect(&mut state, &engine.command_tx, &event_tx) {
                        init_flush_armed = true;
                        init_flush_timer.as_mut().reset(tokio::time::Instant::now() + INIT_FLUSH_TIMEOUT);
                    }
                }
            }
        }

        let reset_frame = build_sysex_frame(&json!({"cmd": "RESET"}));
        let _ = engine.command_tx.send(MidiCommand::Send(reset_frame));
        let _ = event_tx.send(BridgeEvent::Log(
            "Sent RESET command to Console 1.".to_string(),
        ));

        deactivate_all_tracks(&state, &engine.command_tx, &event_tx);
        let _ = event_tx.send(BridgeEvent::Log(
            "Deactivated all tracks on Console 1.".to_string(),
        ));

        state.track_cache = TrackCache::new();
        let _ = event_tx.send(BridgeEvent::Log("Cleared track cache.".to_string()));

        state.osd_enabled = false;
        let _ = event_tx.send(BridgeEvent::Log("Disabled OSD.".to_string()));

        // Closed before the MIDI engine is torn down (JS closes MIDI first) so the engine task gets
        // scheduled to actually run its close handshake instead of being dropped mid-flight when
        // the runtime task returns. The wait is bounded: a server that never answers the close
        // frame must not hold up exit.
        if let Some(handle) = state.ws_handle.take() {
            let _ = handle.command_tx.send(WsCommand::Shutdown);
            let _ = tokio::time::timeout(WS_SHUTDOWN_GRACE, handle.join_handle).await;
            let _ = event_tx.send(BridgeEvent::Log(
                "Closed Mixing Station WebSocket.".to_string(),
            ));
        }

        let _ = engine.command_tx.send(MidiCommand::Shutdown);
        // Like the WS wait above, this is a best-effort shutdown step: both the `JoinError` from
        // `spawn_blocking` and any panic payload propagated from the MIDI thread itself are
        // intentionally swallowed, so a MIDI-thread panic can't take the runtime's shutdown path
        // down with it.
        let midi_thread = engine.join_handle;
        let _ = tokio::task::spawn_blocking(move || midi_thread.join()).await;
    });

    BridgeRuntimeHandle {
        command_tx,
        events: event_rx,
        join_handle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update_queue::QueuedUpdate;
    use bridge_config::TrackOrderEntry;

    /// A throwaway event sender for tests that don't assert on emitted events. The receiver is
    /// dropped immediately, so every `send` fails silently — which is exactly what the moved
    /// code's `let _ = event_tx.send(...)` sites already tolerate.
    fn discarding_event_tx() -> mpsc::UnboundedSender<BridgeEvent> {
        mpsc::unbounded_channel().0
    }

    #[test]
    fn send_sysex_and_log_emits_two_log_lines_when_log_json_is_set() {
        let (midi_tx, midi_rx) = std::sync::mpsc::channel();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<BridgeEvent>();

        send_sysex_and_log(&midi_tx, &event_tx, true, &json!({"cmd": "RESET"}));

        assert!(midi_rx.try_recv().is_ok(), "the frame should still be sent regardless of logging");
        let first = event_rx.try_recv().expect("expected a log line");
        match first {
            BridgeEvent::Log(line) => assert!(line.starts_with("Sending SysEx JSON to Console 1:")),
            other => panic!("expected BridgeEvent::Log, got {other:?}"),
        }
        let second = event_rx.try_recv().expect("expected a second log line");
        match second {
            BridgeEvent::Log(line) => assert!(line.starts_with("SysEx data:")),
            other => panic!("expected BridgeEvent::Log, got {other:?}"),
        }
        assert!(event_rx.try_recv().is_err(), "expected exactly two log lines");
    }

    #[test]
    fn send_sysex_and_log_sends_the_frame_but_logs_nothing_when_log_json_is_unset() {
        let (midi_tx, midi_rx) = std::sync::mpsc::channel();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<BridgeEvent>();

        send_sysex_and_log(&midi_tx, &event_tx, false, &json!({"cmd": "RESET"}));

        assert!(midi_rx.try_recv().is_ok());
        assert!(event_rx.try_recv().is_err(), "no log lines expected when log_json is false");
    }

    /// `send_sysex_and_log`'s own tests above only prove the choke point's internal branching
    /// on a literal `true`/`false` -- they say nothing about whether `state.config.log_json`
    /// actually reaches it from any real call site. This proves the wiring end-to-end through
    /// one representative migrated call site (`send_status_bank_tracks`): with
    /// `state.config.log_json` set on live `BridgeState`, a real higher-level send must still
    /// produce the `LOG_JSON` log line, not just an unconditional SysEx send.
    #[test]
    fn log_json_true_on_bridge_state_flows_through_a_migrated_call_site_to_send_sysex_and_log() {
        let mut state = running_state_without_ws_engine();
        state.config.log_json = true;
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<BridgeEvent>();

        send_status_bank_tracks(&mut state, &midi_tx, &event_tx);

        let mut saw_sysex_log = false;
        while let Ok(event) = event_rx.try_recv() {
            if let BridgeEvent::Log(line) = event {
                if line.starts_with("Sending SysEx JSON to Console 1:") {
                    saw_sysex_log = true;
                    break;
                }
            }
        }
        assert!(
            saw_sysex_log,
            "expected state.config.log_json = true to produce a 'Sending SysEx JSON to \
             Console 1:' log line via send_status_bank_tracks -- if this fails, log_json \
             stopped flowing from BridgeState into send_sysex_and_log at this call site"
        );
    }

    #[test]
    fn flush_bare_update_queue_sends_trackid_as_a_named_field_not_an_object_key() {
        let mut state = running_state_without_ws_engine();
        state.osd_enabled = true;
        state.bare_update_queue.queue(
            "TESTID01".to_string(),
            "filterLcOn".to_string(),
            json!(true),
        );
        let (midi_tx, midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        flush_bare_update_queue(&mut state, &midi_tx, &event_tx);

        let frame = midi_rx.try_recv().expect("expected a bare-field send");
        match frame {
            MidiCommand::Send(bytes) => {
                let value = parse_sysex_json(&bytes).expect("valid SysEx JSON frame");
                assert_eq!(value["trackId"], json!("TESTID01"));
                assert_eq!(value["filterLcOn"]["value"], json!(1));
                assert_eq!(value["filterLcOn"]["name"], json!("Low Cut On"));
                // Exactly two top-level keys: "trackId" and the field name -- not a
                // trackId-keyed object with no "trackId" field at all (the bug this guards).
                assert_eq!(value.as_object().unwrap().len(), 2);
            }
            MidiCommand::Shutdown => panic!("expected MidiCommand::Send, got Shutdown"),
        }
    }

    #[test]
    fn flush_bare_update_queue_drains_but_sends_nothing_when_osd_disabled() {
        let mut state = running_state_without_ws_engine();
        state.osd_enabled = false;
        state.bare_update_queue.queue(
            "TESTID01".to_string(),
            "filterLcOn".to_string(),
            json!(true),
        );
        let (midi_tx, midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        flush_bare_update_queue(&mut state, &midi_tx, &event_tx);

        assert!(
            state.bare_update_queue.is_empty(),
            "must still drain the queue"
        );
        assert!(
            midi_rx.try_recv().is_err(),
            "must send nothing while OSD disabled"
        );
    }

    #[test]
    fn bridge_event_serializes_with_a_type_tag_and_camel_case_fields() {
        let log = serde_json::to_value(BridgeEvent::Log("hi".to_string())).unwrap();
        assert_eq!(log, json!({"type": "log", "data": "hi"}));

        let lifecycle =
            serde_json::to_value(BridgeEvent::LifecycleChanged(Lifecycle::Running)).unwrap();
        assert_eq!(
            lifecycle,
            json!({"type": "lifecycleChanged", "data": "running"})
        );

        let applied = serde_json::to_value(BridgeEvent::ConfigApplied {
            url_changed: true,
            anything_changed: false,
        })
        .unwrap();
        assert_eq!(
            applied,
            json!({
                "type": "configApplied",
                "data": {"urlChanged": true, "anythingChanged": false}
            })
        );

        let crashed = serde_json::to_value(BridgeEvent::Crashed("oops".to_string())).unwrap();
        assert_eq!(crashed, json!({"type": "crashed", "data": "oops"}));
    }

    #[tokio::test]
    async fn spawn_bridge_runtime_returns_a_usable_handle() {
        let handle = spawn_bridge_runtime(RuntimeConfig::default());
        handle.command_tx.send(BridgeCommand::Shutdown).unwrap();
        let result =
            tokio::time::timeout(std::time::Duration::from_secs(2), handle.join_handle).await;
        assert!(
            result.is_ok(),
            "runtime should shut down promptly on Shutdown"
        );
    }

    #[tokio::test]
    async fn dropping_the_handle_without_shutdown_still_lets_the_task_exit() {
        let handle = spawn_bridge_runtime(RuntimeConfig::default());
        let join_handle = {
            let handle = handle;
            drop(handle.command_tx);
            drop(handle.events);
            handle.join_handle
        };
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), join_handle).await;
        assert!(
            result.is_ok(),
            "dropping command_tx should let the runtime's command channel return None and exit"
        );
    }

    #[test]
    fn bridge_command_variants_are_constructible() {
        let _ = BridgeCommand::Start(None);
        let _ = BridgeCommand::Start(Some(BridgeConfigPatch::default()));
        let _ = BridgeCommand::Stop;
        let _ = BridgeCommand::ConfigApply(BridgeConfigPatch::default());
        let _ = BridgeCommand::Shutdown;
    }

    #[test]
    fn bridge_event_variants_are_constructible() {
        let _ = BridgeEvent::Log("hello".to_string());
        let _ = BridgeEvent::LifecycleChanged(Lifecycle::Running);
        let _ = BridgeEvent::ConfigApplied {
            url_changed: true,
            anything_changed: true,
        };
        let _ = BridgeEvent::Crashed("panic".to_string());
    }

    /// Regression test: the runtime task must own the event sender, so `events` reports
    /// "open, nothing yet" while the task runs and only disconnects once it exits. A sender
    /// left behind in `spawn_bridge_runtime`'s own frame would close the channel on return.
    #[tokio::test]
    async fn the_event_channel_stays_open_until_the_runtime_task_exits() {
        let BridgeRuntimeHandle {
            command_tx,
            mut events,
            join_handle,
        } = spawn_bridge_runtime(RuntimeConfig::default());

        command_tx.send(BridgeCommand::Shutdown).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), join_handle)
            .await
            .expect("runtime should shut down promptly on Shutdown")
            .expect("runtime task should not panic");

        // Drain whatever the standby entry + shutdown sequence emitted; the channel must then
        // report Disconnected rather than Empty, proving the sender died with the task.
        while let Ok(event) = events.try_recv() {
            let _ = event;
        }
        assert_eq!(
            events.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        );
    }

    fn slot(kind: LayoutSlotKind, ms_channels: Vec<usize>, pan_locked: bool) -> LayoutSlot {
        LayoutSlot {
            object_id: 20,
            kind,
            ms_primary: ms_channels.first().copied(),
            ms_channels,
            pan_locked,
        }
    }

    #[test]
    fn apply_slot_overrides_forces_main_name() {
        let config = RuntimeConfig::default();
        let slot = slot(LayoutSlotKind::Main, vec![70, 71], false);
        let out = apply_slot_overrides(&slot, "cfg.name", json!("LR"), &config).unwrap();
        assert_eq!(out, json!("Main"));
    }

    #[test]
    fn apply_slot_overrides_trims_stereo_suffix_from_paired_input_name() {
        let config = RuntimeConfig::default();
        let slot = slot(LayoutSlotKind::Input, vec![4, 5], true);
        let out = apply_slot_overrides(&slot, "cfg.name", json!("Keys L"), &config).unwrap();
        assert_eq!(out, json!("Keys"));
    }

    #[test]
    fn apply_slot_overrides_leaves_mono_input_name_alone() {
        let config = RuntimeConfig::default();
        let slot = slot(LayoutSlotKind::Input, vec![4], false);
        let out = apply_slot_overrides(&slot, "cfg.name", json!("Vocal L"), &config).unwrap();
        assert_eq!(out, json!("Vocal L"));
    }

    #[test]
    fn apply_slot_overrides_replaces_bus_and_main_colors_with_configured_ones() {
        let config = RuntimeConfig::default();
        let bus = slot(LayoutSlotKind::Bus, vec![50], false);
        let main = slot(LayoutSlotKind::Main, vec![70, 71], false);
        assert_eq!(
            apply_slot_overrides(&bus, "cfg.color", json!(0x123456), &config).unwrap(),
            json!(config.console1_bus_color)
        );
        assert_eq!(
            apply_slot_overrides(&main, "cfg.color", json!(0x123456), &config).unwrap(),
            json!(config.console1_main_color)
        );
    }

    #[test]
    fn apply_slot_overrides_keeps_input_color_from_mixing_station() {
        let config = RuntimeConfig::default();
        let slot = slot(LayoutSlotKind::Input, vec![4], false);
        let out = apply_slot_overrides(&slot, "cfg.color", json!(0x123456), &config).unwrap();
        assert_eq!(out, json!(0x123456));
    }

    #[test]
    fn apply_slot_overrides_ignores_pan_for_stereo_linked_pairs() {
        let config = RuntimeConfig::default();
        let slot = slot(LayoutSlotKind::Input, vec![4, 5], true);
        assert!(apply_slot_overrides(&slot, "mix.pan", json!(0.75), &config).is_err());
    }

    #[test]
    fn apply_slot_overrides_centers_pan_for_non_stereo_pan_locked_slots() {
        let config = RuntimeConfig::default();
        let slot = slot(LayoutSlotKind::Main, vec![70], true);
        let out = apply_slot_overrides(&slot, "mix.pan", json!(0.75), &config).unwrap();
        assert_eq!(out, json!(0.5));
    }

    #[test]
    fn apply_slot_overrides_passes_unlocked_pan_through_untouched() {
        let config = RuntimeConfig::default();
        let slot = slot(LayoutSlotKind::Input, vec![4], false);
        let out = apply_slot_overrides(&slot, "mix.pan", json!(0.75), &config).unwrap();
        assert_eq!(out, json!(0.75));
    }

    #[test]
    fn apply_slot_overrides_passes_unrelated_params_through_untouched() {
        let config = RuntimeConfig::default();
        let slot = slot(LayoutSlotKind::Main, vec![70, 71], false);
        let out = apply_slot_overrides(&slot, "mix.lvl", json!(-6.0), &config).unwrap();
        assert_eq!(out, json!(-6.0));
    }

    /// A `running` state with no WS engine attached, so every `ws_send_json` is a no-op.
    fn running_state_without_ws_engine() -> BridgeState {
        let config = RuntimeConfig::default();
        let console_architecture = ConsoleArchitecture {
            total_channels: 80,
            input_channel_count: 32,
            bus_channel_start: 48,
            bus_channel_count: 16,
            main_stereo_channels: vec![70, 71],
        };
        let bus_channel_start = console_architecture.bus_channel_start as usize;
        let layout = build_default_layout(&config, &console_architecture);
        let object_ids_by_ms_channel = object_ids_by_ms_channel(&layout);
        let send_mapping = build_send_mapping(
            &config.c1_send_to_ms_bus_number,
            console_architecture.bus_channel_count as usize,
        );
        BridgeState {
            lifecycle: Lifecycle::Running,
            osd_enabled: true,
            layout,
            track_cache: TrackCache::new(),
            rng: StdRng::seed_from_u64(7),
            config,
            bus_channel_start,
            console_architecture,
            console_info_pending: false,
            ws_handle: None,
            ws_connected: false,
            is_initializing: false,
            init_message_buffer: Vec::new(),
            has_sent_initial_track_dump: true,
            sends_mode_ms_send_index: Some(0),
            sends_mode_subscribed_ms_send_index: Some(0),
            input_send_state: InputSendState::new(),
            input_std_state: HashMap::new(),
            send_mapping,
            echo_tracker: EchoSuppressionTracker::new(),
            update_queue: UpdateQueue::new(),
            bare_update_queue: BareUpdateQueue::new(),
            ms_write_queue: MsWriteQueue::new(),
            object_ids_by_ms_channel,
            init_seen_ms_channels: HashSet::new(),
            metered_object_ids: HashSet::new(),
            metering2_param_ms_channels: Vec::new(),
            meter_db_cache: MeterDbCache::new(),
        }
    }

    #[test]
    fn update_metering2_subscription_does_nothing_when_not_connected() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = false;
        state.metered_object_ids = [10].into_iter().collect();
        update_metering2_subscription(&mut state);
        assert!(state.metering2_param_ms_channels.is_empty());
    }

    #[test]
    fn update_metering2_subscription_updates_param_channels_when_connected() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        // Object ID 10 is the first real Input slot in the default layout (object IDs 0..=9
        // are the fixed status/Start bank -- see `START_SLOT_OBJECT_ID`), MS channel 0.
        state.metered_object_ids = [10].into_iter().collect();
        update_metering2_subscription(&mut state);
        assert_eq!(state.metering2_param_ms_channels, vec![0]);
    }

    #[test]
    fn handle_active_meters_message_replaces_metered_object_ids_wholesale() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        state.metered_object_ids = [999].into_iter().collect(); // stale, unrelated object ID
        let slot = state.layout[10].clone();
        let input_track_id = clone_or_create_track(&mut state, 10, &slot).track_id;
        handle_active_meters_message(vec![input_track_id], &mut state);
        assert_eq!(state.metered_object_ids, [10].into_iter().collect());
    }

    #[test]
    fn handle_active_meters_message_ignores_unresolvable_track_ids() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        handle_active_meters_message(vec!["NOT-A-REAL-TRACK-ID".to_string()], &mut state);
        assert!(state.metered_object_ids.is_empty());
    }

    #[test]
    fn handle_active_meters_message_queues_a_forced_meter_update_for_active_metered_tracks() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        let slot = state.layout[10].clone();
        let track = clone_or_create_track(&mut state, 10, &slot);
        assert!(track.is_active, "the default Input slot at object_id 10 must be active for this test to be meaningful");
        let track_id = track.track_id.clone();

        handle_active_meters_message(vec![track_id.clone()], &mut state);

        let all = state.update_queue.take_all();
        let entry = all.iter().find(|(id, _)| id == &track_id);
        assert!(entry.is_some(), "expected a queued update for the metered track");
        let (_id, queued) = entry.unwrap();
        assert!(queued.force_send, "batchSendChangedMeters's port must force-send");
        assert!(queued.fields.contains_key("meter"));
    }

    #[test]
    fn active_meters_is_skipped_when_combined_with_reset_in_the_same_message() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        let slot = state.layout[10].clone();
        let track_id = clone_or_create_track(&mut state, 10, &slot).track_id;

        let parsed = json!({"cmd": "RESET", "activeMeters": [track_id]});
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<BridgeEvent>();
        let raw = build_sysex_frame(&parsed);

        handle_inbound_midi_message(&raw, &mut state, &midi_tx, &event_tx);

        assert!(
            state.metered_object_ids.is_empty(),
            "RESET must return before activeMeters is ever checked, matching JS"
        );
    }

    #[test]
    fn active_meters_fires_on_its_own_with_no_cmd_or_handshake_present() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        let slot = state.layout[10].clone();
        let track_id = clone_or_create_track(&mut state, 10, &slot).track_id;

        let parsed = json!({"activeMeters": [track_id]});
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<BridgeEvent>();
        let raw = build_sysex_frame(&parsed);

        handle_inbound_midi_message(&raw, &mut state, &midi_tx, &event_tx);

        assert_eq!(state.metered_object_ids, [10].into_iter().collect());
    }

    #[test]
    fn active_meters_fires_alongside_a_handshake_ack_in_the_same_message() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        let slot = state.layout[10].clone();
        let track_id = clone_or_create_track(&mut state, 10, &slot).track_id;

        let parsed = json!({"handshake": {"ack": true}, "activeMeters": [track_id]});
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<BridgeEvent>();
        let raw = build_sysex_frame(&parsed);

        handle_inbound_midi_message(&raw, &mut state, &midi_tx, &event_tx);

        assert_eq!(state.metered_object_ids, [10].into_iter().collect());
    }

    #[test]
    fn metering2_push_is_ignored_when_no_active_subscription() {
        let mut state = running_state_without_ws_engine();
        // metering2_param_ms_channels is empty by default -- no subscription active.
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<BridgeEvent>();
        let text = r#"{"path":"/console/metering2/0","body":{"v":[-20.0]}}"#;
        let armed = handle_ws_message(&mut state, text, &midi_tx, &event_tx);
        assert!(!armed);
        assert!(state.update_queue.is_empty());
    }

    #[test]
    fn metering2_push_is_ignored_when_subscription_id_does_not_match() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        let slot = state.layout[10].clone();
        clone_or_create_track(&mut state, 10, &slot);
        state.metered_object_ids = [10].into_iter().collect();
        update_metering2_subscription(&mut state);
        assert_eq!(state.metering2_param_ms_channels, vec![0]);

        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<BridgeEvent>();
        // Subscription ID 99 doesn't match METERING2_SUBSCRIPTION_ID (0) -- must be ignored even
        // though a real, matching subscription (and its resulting queue behavior) is now in place.
        let text = r#"{"path":"/console/metering2/99","body":{"v":[-20.0]}}"#;
        let armed = handle_ws_message(&mut state, text, &midi_tx, &event_tx);
        assert!(!armed);
        assert!(state.update_queue.is_empty());
    }

    #[test]
    fn metering2_push_updates_meter_db_cache_and_queues_a_meter_update() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        let slot = state.layout[10].clone(); // object_id 10, an Input slot, MS channel 0
        let track = clone_or_create_track(&mut state, 10, &slot);
        assert!(track.is_active);
        let track_id = track.track_id.clone();

        // Subscribe to object_id 10 first, so metering2_param_ms_channels/object_ids_by_ms_channel
        // line up the way a real activeMeters message would have set them up.
        state.metered_object_ids = [10].into_iter().collect();
        update_metering2_subscription(&mut state);
        assert_eq!(state.metering2_param_ms_channels, vec![0]);

        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<BridgeEvent>();
        let text = r#"{"path":"/console/metering2/0","body":{"v":[-6.0]}}"#;
        let armed = handle_ws_message(&mut state, text, &midi_tx, &event_tx);
        assert!(!armed, "a metering2 push never requests an init-flush re-arm");

        assert_eq!(
            state.meter_db_cache.get(0),
            Some(-6.0),
            "the cache must be updated with the new dB value"
        );
        let all = state.update_queue.take_all();
        let entry = all.iter().find(|(id, _)| id == &track_id);
        assert!(entry.is_some(), "expected a queued meter update for the affected track");
        let (_id, queued) = entry.unwrap();
        assert!(
            !queued.force_send,
            "applyMeterUpdatesForMsChannels's port uses a plain (non-forced) queue call"
        );
        assert!(queued.fields.contains_key("meter"));
    }

    #[test]
    fn metering2_push_below_change_threshold_does_not_requeue() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        let slot = state.layout[10].clone();
        clone_or_create_track(&mut state, 10, &slot);
        state.metered_object_ids = [10].into_iter().collect();
        update_metering2_subscription(&mut state);
        state.update_queue.take_all();

        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<BridgeEvent>();
        // First push establishes -6.0dB.
        handle_ws_message(
            &mut state,
            r#"{"path":"/console/metering2/0","body":{"v":[-6.0]}}"#,
            &midi_tx,
            &event_tx,
        );
        state.update_queue.take_all();
        // Second push is a 0.04dB delta -- within the cache's 0.05dB threshold, so
        // meter_db_cache.apply_updates must report no change and this must not re-queue.
        handle_ws_message(
            &mut state,
            r#"{"path":"/console/metering2/0","body":{"v":[-6.04]}}"#,
            &midi_tx,
            &event_tx,
        );
        assert!(state.update_queue.is_empty());
    }

    #[test]
    fn metering2_binary_push_updates_meter_db_cache_and_queues_a_meter_update() {
        // Real Mixing Station always sends JSON (`v`) in practice -- this bridge never requests
        // binary (`METERING2_BINARY` is hardcoded `false`, matching JS) -- but JS's own comment
        // notes "some MS versions/configs may send binary payloads even if we asked for JSON",
        // and `normalize_metering2_values`'s `b` branch already decodes it (tested in isolation
        // in `metering2_message.rs`). This proves that decode path is actually wired through the
        // real inbound dispatch (`handle_ws_message`), not just exercised by its own unit tests.
        use base64::{engine::general_purpose::STANDARD, Engine as _};

        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        let slot = state.layout[10].clone(); // object_id 10, an Input slot, MS channel 0
        let track = clone_or_create_track(&mut state, 10, &slot);
        assert!(track.is_active);
        let track_id = track.track_id.clone();

        state.metered_object_ids = [10].into_iter().collect();
        update_metering2_subscription(&mut state);
        assert_eq!(state.metering2_param_ms_channels, vec![0]);

        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<BridgeEvent>();

        // decode_metering2_binary packs big-endian i16 values scaled by 100 -- a reading of
        // -6.00dB is the wire integer -600 -- base64-encoded with padding stripped, matching
        // metering2_message.rs's own `binary_payload_decodes_via_existing_decoder` fixture.
        let mut buf = Vec::new();
        buf.extend_from_slice(&(-600i16).to_be_bytes());
        let encoded = STANDARD.encode(&buf).trim_end_matches('=').to_string();
        let text = format!(r#"{{"path":"/console/metering2/0","body":{{"b":"{encoded}"}}}}"#);

        let armed = handle_ws_message(&mut state, &text, &midi_tx, &event_tx);
        assert!(!armed, "a metering2 push never requests an init-flush re-arm");

        assert_eq!(
            state.meter_db_cache.get(0),
            Some(-6.0),
            "the binary-decoded dB value must reach the cache, same as a JSON push does"
        );
        let all = state.update_queue.take_all();
        let entry = all.iter().find(|(id, _)| id == &track_id);
        assert!(entry.is_some(), "expected a queued meter update from a binary push");
        assert!(entry.unwrap().1.fields.contains_key("meter"));
    }

    #[test]
    fn metering2_push_within_norm_threshold_does_not_requeue_even_when_cache_registers_a_change() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        let slot = state.layout[10].clone();
        let mut track = clone_or_create_track(&mut state, 10, &slot);
        let existing_norm = crate::metering_utils::db_to_console1_meter_norm(-6.0);
        track.meter = vec![existing_norm];
        store_track(&mut state, 10, track);

        state.metered_object_ids = [10].into_iter().collect();
        update_metering2_subscription(&mut state);

        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<BridgeEvent>();
        // First-ever push to this channel -- meter_db_cache.apply_updates reports it as "changed"
        // (prev is None on first write), but the resulting norm is identical to what track.meter
        // was already set to, so the separate 0.001 norm-delta gate must suppress the queue.
        handle_ws_message(
            &mut state,
            r#"{"path":"/console/metering2/0","body":{"v":[-6.0]}}"#,
            &midi_tx,
            &event_tx,
        );
        assert!(state.update_queue.is_empty());
    }

    #[test]
    fn metering2_push_correctly_pairs_multiple_channels_by_position_and_persists_meter_values() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;

        // object_id 10 -> MS channel 0, object_id 11 -> MS channel 1 (the first two real Input
        // slots after the fixed status/Start bank, which ends at START_SLOT_OBJECT_ID == 9).
        let slot_a = state.layout[10].clone();
        let slot_b = state.layout[11].clone();
        clone_or_create_track(&mut state, 10, &slot_a);
        clone_or_create_track(&mut state, 11, &slot_b);

        state.metered_object_ids = [10, 11].into_iter().collect();
        update_metering2_subscription(&mut state);
        // Confirm the subscription really has 2 channels, in ascending sorted order (per
        // build_metering2_subscribe_body's doc comment), before relying on positional pairing.
        assert_eq!(state.metering2_param_ms_channels, vec![0, 1]);
        let channels = state.metering2_param_ms_channels.clone();

        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let (event_tx, _event_rx) = mpsc::unbounded_channel::<BridgeEvent>();
        // 3 value-groups pushed against a 2-channel subscription -- the 3rd must be clamped
        // away by .min(...), not cause a panic, and the first 2 must land on the RIGHT channel
        // by position.
        let text = r#"{"path":"/console/metering2/0","body":{"v":[[-6.0],[-30.0],[-3.0]]}}"#;
        handle_ws_message(&mut state, text, &midi_tx, &event_tx);

        // Pins positional pairing: channel[0] got -6.0, channel[1] got -30.0, not swapped.
        assert_eq!(state.meter_db_cache.get(channels[0]), Some(-6.0));
        assert_eq!(state.meter_db_cache.get(channels[1]), Some(-30.0));

        // Pins store_track: the cached TrackInfo's meter field must reflect the new value, not
        // still be the default -- read it back via state.track_cache.get(object_id) rather than
        // through clone_or_create_track (which would just create a fresh default if the write
        // never happened, silently passing the assertion for the wrong reason).
        let track_a = state.track_cache.get(10).expect("track was created earlier");
        assert_ne!(track_a.meter, vec![0.0], "meter must have been persisted via store_track");
        let track_b = state.track_cache.get(11).expect("track was created earlier");
        assert_ne!(track_b.meter, vec![0.0], "meter must have been persisted via store_track");
    }

    #[test]
    fn rebuild_layout_and_reset_caches_uses_the_live_console_architecture() {
        let mut state = running_state_without_ws_engine();
        let original_layout_len = state.layout.len();

        state.console_architecture = ConsoleArchitecture {
            total_channels: 40,
            input_channel_count: 8,
            bus_channel_start: 24,
            bus_channel_count: 4,
            main_stereo_channels: vec![28, 29],
        };
        rebuild_layout_and_reset_caches(&mut state);

        assert_eq!(state.bus_channel_start, 24);
        // 8 inputs + 4 buses + Main + the fixed 10-slot status/Start bank is a very different
        // slot count than the default (32 inputs + 16 buses + Main + status/Start bank) --
        // proves build_default_layout genuinely used the new architecture, not the old one.
        assert_ne!(state.layout.len(), original_layout_len);
        assert!(state
            .object_ids_by_ms_channel
            .values()
            .flatten()
            .any(|&object_id| object_id > START_SLOT_OBJECT_ID));
    }

    #[test]
    fn rebuild_layout_and_reset_caches_resets_metering_state() {
        let mut state = running_state_without_ws_engine();
        state.metered_object_ids = [10].into_iter().collect();
        state.metering2_param_ms_channels = vec![0, 1, 2];
        state.meter_db_cache.apply_updates(&[(0, -20.0)]);

        rebuild_layout_and_reset_caches(&mut state);

        assert!(state.metered_object_ids.is_empty());
        assert!(state.metering2_param_ms_channels.is_empty());
        assert_eq!(state.meter_db_cache.get(0), None);
    }

    // #[tokio::test] rather than the plain #[test] every other synchronous handler test in
    // this file uses -- WsEngineHandle::join_handle requires an actual JoinHandle, and
    // tokio::spawn panics ("no reactor running") outside a Tokio runtime context.
    #[tokio::test]
    async fn ws_connected_sends_the_console_information_request_and_marks_it_pending() {
        let mut state = running_state_without_ws_engine();
        let (ws_command_tx, mut ws_command_rx) = mpsc::unbounded_channel::<WsCommand>();
        let (_ws_event_tx, ws_event_rx) = mpsc::unbounded_channel::<WsEvent>();
        state.ws_handle = Some(WsEngineHandle {
            command_tx: ws_command_tx,
            events: ws_event_rx,
            join_handle: tokio::spawn(async {}),
        });
        let event_tx = discarding_event_tx();

        handle_ws_connected(&mut state, &event_tx);

        assert!(state.console_info_pending);
        assert!(state.ws_connected);
        let sent = ws_command_rx.try_recv().expect("expected a WS send");
        match sent {
            WsCommand::Send(text) => {
                let value: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(value, json!({"path": "/console/information", "method": "GET"}));
            }
            WsCommand::Shutdown => panic!("expected WsCommand::Send, got Shutdown"),
        }
    }

    #[test]
    fn console_information_reply_resolves_pending_and_finishes_connecting() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        state.console_info_pending = true;
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        let reply = json!({
            "path": "/console/information",
            "body": {
                "totalChannels": 40,
                "channelTypes": [
                    {"name": "Input", "offset": 0, "count": 8},
                    {"name": "Bus", "offset": 24, "count": 4},
                    {"name": "Main", "offset": 28, "count": 2}
                ]
            }
        });
        let arm_init_flush = handle_ws_message(&mut state, &reply.to_string(), &midi_tx, &event_tx);

        assert!(arm_init_flush, "finish_ws_connect should report true, matching the old handle_ws_connected contract");
        assert!(!state.console_info_pending);
        assert_eq!(state.console_architecture.input_channel_count, 8);
        assert_eq!(state.bus_channel_start, 24);
        assert!(state.is_initializing);
    }

    #[test]
    fn console_information_reply_is_ignored_when_not_pending() {
        let mut state = running_state_without_ws_engine();
        state.console_info_pending = false;
        let original_architecture = state.console_architecture.clone();
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        let reply = json!({
            "path": "/console/information",
            "body": {"totalChannels": 999}
        });
        let arm_init_flush = handle_ws_message(&mut state, &reply.to_string(), &midi_tx, &event_tx);

        assert!(!arm_init_flush);
        assert_eq!(state.console_architecture, original_architecture);
    }

    #[test]
    fn finish_ws_connect_times_out_gracefully_and_still_finishes_connecting() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        state.console_info_pending = true;
        let original_architecture = state.console_architecture.clone();
        let (midi_tx, midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        // Simulates the timeout branch firing before any reply arrived: architecture is left
        // exactly as it was (the timeout branch never touches console_architecture), and
        // finish_ws_connect still runs with whatever was already known.
        state.console_info_pending = false;
        let arm_init_flush = finish_ws_connect(&mut state, &midi_tx, &event_tx);

        assert!(arm_init_flush);
        assert_eq!(state.console_architecture, original_architecture);
        assert!(state.is_initializing);
        // A handshake frame should still have gone out even though no reply ever arrived.
        assert!(midi_rx.try_recv().is_ok());
    }

    #[test]
    fn finish_ws_connect_is_a_no_op_if_disconnected_in_the_meantime() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = false; // e.g. Stop was processed while the race was in flight
        let (midi_tx, midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        let arm_init_flush = finish_ws_connect(&mut state, &midi_tx, &event_tx);

        assert!(!arm_init_flush);
        assert!(!state.is_initializing);
        assert!(midi_rx.try_recv().is_err(), "no handshake should be sent");
    }

    #[test]
    fn finish_ws_connect_resets_metering_state() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        state.metered_object_ids = [10].into_iter().collect();
        state.metering2_param_ms_channels = vec![0, 1, 2];
        state.meter_db_cache.apply_updates(&[(0, -20.0)]);
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        finish_ws_connect(&mut state, &midi_tx, &event_tx);

        assert!(state.metered_object_ids.is_empty());
        assert!(state.metering2_param_ms_channels.is_empty());
        assert_eq!(state.meter_db_cache.get(0), None);
    }

    /// Extracts the numeric channel index from a `/console/data/get/ch.<N>.<rest>/<format>`
    /// request path, or `None` for anything else (e.g. a `/console/data/subscribe` wildcard
    /// message, which `set_sends_mode` also sends and which this helper must ignore).
    fn requested_get_channel(text: &str) -> Option<usize> {
        let value: Value = serde_json::from_str(text).ok()?;
        let path = value.get("path")?.as_str()?;
        let rest = path.strip_prefix("/console/data/get/ch.")?;
        rest.split('.').next()?.parse().ok()
    }

    fn ws_handle_for_capturing_sends() -> (mpsc::UnboundedReceiver<WsCommand>, WsEngineHandle) {
        let (command_tx, command_rx) = mpsc::unbounded_channel::<WsCommand>();
        let (_event_tx, events) = mpsc::unbounded_channel::<WsEvent>();
        (
            command_rx,
            WsEngineHandle {
                command_tx,
                events,
                join_handle: tokio::spawn(async {}),
            },
        )
    }

    // #[tokio::test] rather than plain #[test] -- ws_handle_for_capturing_sends's
    // WsEngineHandle::join_handle requires an actual JoinHandle, and tokio::spawn panics
    // ("no reactor running") outside a Tokio runtime context (same reason as
    // ws_connected_sends_the_console_information_request_and_marks_it_pending above).
    #[tokio::test]
    async fn refresh_dsp_fields_for_real_channels_uses_the_live_console_architecture() {
        let mut state = running_state_without_ws_engine();
        state.console_architecture = ConsoleArchitecture {
            total_channels: 10,
            input_channel_count: 2,
            bus_channel_start: 4,
            bus_channel_count: 1,
            main_stereo_channels: vec![8, 9],
        };
        // refresh_dsp_fields_for_real_channels reads the separately-tracked
        // state.bus_channel_start (not console_architecture.bus_channel_start) for the bus
        // loop's start index -- normally kept in sync by rebuild_layout_and_reset_caches, which
        // this test bypasses by setting console_architecture directly, so it must be set here too.
        state.bus_channel_start = 4;
        let (mut command_rx, handle) = ws_handle_for_capturing_sends();
        state.ws_handle = Some(handle);

        refresh_dsp_fields_for_real_channels(&state);

        let mut requested_channels = std::collections::HashSet::new();
        while let Ok(WsCommand::Send(text)) = command_rx.try_recv() {
            if let Some(ch) = requested_get_channel(&text) {
                requested_channels.insert(ch);
            }
        }

        // Old hardcoded default (32 inputs, so channel 31 would be touched) must NOT appear.
        assert!(!requested_channels.contains(&31));
        // The live architecture's real channels must all appear.
        for expected in [0, 1, 4, 8, 9] {
            assert!(
                requested_channels.contains(&expected),
                "expected channel {expected} to be requested, got {requested_channels:?}"
            );
        }
    }

    // #[tokio::test] for the same reason as the test above -- see its comment.
    #[tokio::test]
    async fn set_sends_mode_uses_the_live_console_architecture_for_its_ms_channel_loop() {
        let mut state = running_state_without_ws_engine();
        state.console_architecture.input_channel_count = 2;
        // running_state_without_ws_engine() defaults sends_mode_ms_send_index to Some(0)
        // already -- set_sends_mode's `if next == state.sends_mode_ms_send_index { return; }`
        // guard would otherwise short-circuit before the channel loop under test ever runs.
        // None -> Some(0) mirrors a real STANDARD -> SENDS transition.
        state.sends_mode_ms_send_index = None;
        let (mut command_rx, handle) = ws_handle_for_capturing_sends();
        state.ws_handle = Some(handle);
        let event_tx = discarding_event_tx();

        set_sends_mode(&mut state, Some(0), &event_tx);

        let mut requested_channels = std::collections::HashSet::new();
        while let Ok(WsCommand::Send(text)) = command_rx.try_recv() {
            if let Some(ch) = requested_get_channel(&text) {
                requested_channels.insert(ch);
            }
        }

        assert!(
            !requested_channels.contains(&31),
            "must not use the old hardcoded 32-input default"
        );
        assert!(requested_channels.contains(&0));
        assert!(requested_channels.contains(&1));
    }

    // #[tokio::test] for the same reason as the two tests above -- see the first one's comment.
    #[tokio::test]
    async fn set_sends_mode_none_arm_uses_the_live_console_architecture_for_its_ms_channel_loop() {
        let mut state = running_state_without_ws_engine();
        state.console_architecture.input_channel_count = 2;
        // Unlike the Some(index) test above, no sends_mode_ms_send_index precondition reset is
        // needed here: running_state_without_ws_engine() defaults it to Some(0), and Some(0) !=
        // None (the `next` this test passes), so set_sends_mode's early-return guard
        // (`if next == state.sends_mode_ms_send_index { return; }`) doesn't fire -- this test
        // exercises a real SENDS -> STANDARD deselection from that Some(0) starting state.
        let (mut command_rx, handle) = ws_handle_for_capturing_sends();
        state.ws_handle = Some(handle);
        let event_tx = discarding_event_tx();

        set_sends_mode(&mut state, None, &event_tx);

        let mut requested_channels = std::collections::HashSet::new();
        while let Ok(WsCommand::Send(text)) = command_rx.try_recv() {
            if let Some(ch) = requested_get_channel(&text) {
                requested_channels.insert(ch);
            }
        }

        assert!(
            !requested_channels.contains(&31),
            "must not use the old hardcoded 32-input default"
        );
        assert!(requested_channels.contains(&0));
        assert!(requested_channels.contains(&1));
    }

    /// The Start slot's own `selected:false` is not enough to clear Console 1's selection latch
    /// on real hardware — a *different* object has to be selected. Both halves must survive
    /// `enter_standby_state`, which drains the update queue partway through this same call.
    #[test]
    fn start_slot_trigger_unlatches_by_also_force_selecting_a_neighbor() {
        let mut state = running_state_without_ws_engine();
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();
        send_status_bank_tracks(&mut state, &midi_tx, &event_tx);

        let start_track_id = state
            .track_cache
            .get(START_SLOT_OBJECT_ID)
            .unwrap()
            .track_id
            .clone();
        let neighbor_object_id = START_SLOT_OBJECT_ID - 2;
        let neighbor_track_id = state
            .track_cache
            .get(neighbor_object_id)
            .unwrap()
            .track_id
            .clone();

        let frame = build_sysex_frame(&json!({"trackId": start_track_id, "selected": true}));
        handle_inbound_midi_message(&frame, &mut state, &midi_tx, &event_tx);

        assert_eq!(state.lifecycle, Lifecycle::Standby);
        assert!(
            !state
                .track_cache
                .get(START_SLOT_OBJECT_ID)
                .unwrap()
                .selected
        );
        assert!(state.track_cache.get(neighbor_object_id).unwrap().selected);

        let forced: HashMap<String, QueuedUpdate> =
            state.update_queue.take_forced().into_iter().collect();
        assert_eq!(forced[&start_track_id].fields["selected"], json!(false));
        assert_eq!(forced[&neighbor_track_id].fields["selected"], json!(true));
    }

    fn all_off_status_snapshot() -> StatusSnapshot {
        StatusSnapshot {
            ipad: false,
            spd_sx_pro: false,
            midi_maestro: false,
            bome_mtp: false,
            mixing_station: false,
            console1_osd: false,
            ableton_live: false,
        }
    }

    #[test]
    fn apply_live_status_colors_sends_on_color_for_a_true_indicator() {
        let mut state = running_state_without_ws_engine();
        let (midi_tx, midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();
        let mut snapshot = all_off_status_snapshot();
        snapshot.ipad = true;

        apply_live_status_colors(&mut state, &midi_tx, &snapshot, &event_tx);

        let frame = midi_rx.try_recv().expect("expected a trackBatch send");
        match frame {
            MidiCommand::Send(bytes) => {
                let value = parse_sysex_json(&bytes).expect("valid SysEx JSON frame");
                let batch = value["trackBatch"].as_array().unwrap();
                let ipad_track_id = state.track_cache.get(0).unwrap().track_id.clone();
                let ipad_entry = batch
                    .iter()
                    .find(|e| e["trackId"] == json!(ipad_track_id))
                    .expect("iPad slot should be in the batch");
                assert_eq!(ipad_entry["color"], json!(0x00ff00));
            }
            // MidiCommand has no Debug impl -- match the only other variant explicitly
            // instead of a Debug-requiring wildcard arm (same fix as the DSP-trackid plan).
            MidiCommand::Shutdown => panic!("expected MidiCommand::Send, got Shutdown"),
        }
        assert_eq!(state.track_cache.get(0).unwrap().color, 0x00ff00);
        assert!(midi_rx.try_recv().is_err(), "expected exactly one send");
    }

    #[test]
    fn apply_live_status_colors_sends_off_color_for_a_false_indicator() {
        let mut state = running_state_without_ws_engine();
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();
        // Force the cached color away from the default off-color first, so the off-color
        // branch has something real to correct rather than trivially matching the fresh
        // create_default_track_for_slot default.
        apply_live_status_colors(&mut state, &midi_tx, &{
            let mut s = all_off_status_snapshot();
            s.ipad = true;
            s
        }, &event_tx);
        assert_eq!(state.track_cache.get(0).unwrap().color, 0x00ff00);

        let (midi_tx2, midi_rx2) = std::sync::mpsc::channel();
        apply_live_status_colors(&mut state, &midi_tx2, &all_off_status_snapshot(), &event_tx);

        assert_eq!(state.track_cache.get(0).unwrap().color, 0x0000ff);
        assert!(midi_rx2.try_recv().is_ok(), "expected a send correcting ipad back to off");
    }

    #[test]
    fn apply_live_status_colors_is_a_no_op_when_nothing_changed() {
        let mut state = running_state_without_ws_engine();
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();
        let snapshot = all_off_status_snapshot();
        // First call creates the slots at their default off-color, matching the snapshot --
        // nothing should differ.
        apply_live_status_colors(&mut state, &midi_tx, &snapshot, &event_tx);

        let (midi_tx2, midi_rx2) = std::sync::mpsc::channel();
        apply_live_status_colors(&mut state, &midi_tx2, &snapshot, &event_tx);

        assert!(
            midi_rx2.try_recv().is_err(),
            "second call with an unchanged snapshot should send nothing"
        );
    }

    #[test]
    fn apply_live_status_colors_works_while_standby() {
        let mut state = running_state_without_ws_engine();
        state.lifecycle = Lifecycle::Standby;
        let (midi_tx, midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();
        let mut snapshot = all_off_status_snapshot();
        snapshot.mixing_station = true;

        apply_live_status_colors(&mut state, &midi_tx, &snapshot, &event_tx);

        assert!(midi_rx.try_recv().is_ok(), "must force-send in standby too, matching JS");
    }

    #[test]
    fn apply_live_status_colors_only_touches_status_slots() {
        let mut state = running_state_without_ws_engine();
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();
        let mut snapshot = all_off_status_snapshot();
        snapshot.ipad = true;

        apply_live_status_colors(&mut state, &midi_tx, &snapshot, &event_tx);

        // Bank 0 is object ids 0..=9, but only 7 of those 10 are Status-kind slots (ids 0-2
        // and 4-7, per build_status_bank_slots_layout_and_indicator_order); ids 3 and 8 are
        // Empty spacers and id 9 (START_SLOT_OBJECT_ID) is the Start slot. Asserting all three
        // non-Status bank-0 slots -- plus the first real input slot at id 10 -- distinguishes
        // "filtered by LayoutSlotKind::Status" from the coarser (and wrong) "filtered by
        // object id <= 9".
        assert!(state.track_cache.get(3).is_none(), "Empty spacer at id 3 must not be created");
        assert!(state.track_cache.get(8).is_none(), "Empty spacer at id 8 must not be created");
        assert!(
            state.track_cache.get(START_SLOT_OBJECT_ID).is_none(),
            "Start slot must not be created"
        );
        assert!(state.track_cache.get(10).is_none(), "first real input slot must not be created");
    }

    #[test]
    fn apply_live_status_colors_batches_multiple_changed_slots_into_one_trackbatch_send() {
        let mut state = running_state_without_ws_engine();
        let (midi_tx, midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();
        let mut snapshot = all_off_status_snapshot();
        snapshot.ipad = true;
        snapshot.mixing_station = true;

        apply_live_status_colors(&mut state, &midi_tx, &snapshot, &event_tx);

        let frame = midi_rx.try_recv().expect("expected a trackBatch send");
        let batch = match frame {
            MidiCommand::Send(bytes) => {
                let value = parse_sysex_json(&bytes).expect("valid SysEx JSON frame");
                value["trackBatch"]
                    .as_array()
                    .expect("trackBatch is an array")
                    .clone()
            }
            MidiCommand::Shutdown => panic!("expected MidiCommand::Send, got Shutdown"),
        };
        assert!(
            midi_rx.try_recv().is_err(),
            "both changed slots must be batched into a single trackBatch send, not two"
        );

        assert_eq!(
            batch.len(),
            2,
            "trackBatch should contain exactly the 2 changed slots, got {batch:?}"
        );

        let ipad_track_id = state.track_cache.get(0).unwrap().track_id.clone();
        let mixing_station_object_id = state
            .layout
            .iter()
            .find(|s| matches!(s.kind, LayoutSlotKind::Status { key, .. } if key == "mixingStation"))
            .unwrap()
            .object_id;
        let mixing_station_track_id = state
            .track_cache
            .get(mixing_station_object_id)
            .unwrap()
            .track_id
            .clone();

        let ipad_entry = batch
            .iter()
            .find(|e| e["trackId"] == json!(ipad_track_id))
            .expect("ipad slot should be in the batch");
        assert_eq!(ipad_entry["color"], json!(0x00ff00));

        let mixing_station_entry = batch
            .iter()
            .find(|e| e["trackId"] == json!(mixing_station_track_id))
            .expect("mixingStation slot should be in the batch");
        assert_eq!(mixing_station_entry["color"], json!(0x00ff00));
    }

    #[test]
    fn finalize_initialization_control_action_calls_the_real_function() {
        let mut state = running_state_without_ws_engine();
        state.is_initializing = true;
        state.has_sent_initial_track_dump = false;
        let (midi_tx, midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        let must_arm_timer = handle_control_message(
            ControlAction::FinalizeInitialization,
            &mut state,
            &midi_tx,
            &event_tx,
        );

        assert!(
            !state.is_initializing,
            "finalize_initialization should have completed the window"
        );
        assert!(state.has_sent_initial_track_dump);
        assert!(
            !must_arm_timer,
            "finalize_initialization runs synchronously, no timer needed"
        );

        let sent_trackbatch = midi_rx.try_iter().any(|cmd| match cmd {
            MidiCommand::Send(frame) => String::from_utf8_lossy(&frame).contains("trackBatch"),
            MidiCommand::Shutdown => false,
        });
        assert!(
            sent_trackbatch,
            "finalize_initialization must actually send a trackBatch dump to Console 1, not just flip state flags"
        );
    }

    #[test]
    fn finalize_initialization_control_action_is_a_no_op_when_nothing_is_pending() {
        let mut state = running_state_without_ws_engine();
        state.is_initializing = false;
        state.has_sent_initial_track_dump = true;
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        let must_arm_timer = handle_control_message(
            ControlAction::FinalizeInitialization,
            &mut state,
            &midi_tx,
            &event_tx,
        );

        assert!(!must_arm_timer);
        assert!(
            state.has_sent_initial_track_dump,
            "should remain unchanged, nothing to finalize"
        );
    }

    #[test]
    fn schedule_full_resync_control_action_arms_a_fresh_init_window() {
        let mut state = running_state_without_ws_engine();
        state.is_initializing = false;
        state.has_sent_initial_track_dump = true;
        state.init_message_buffer.push(BufferedInitUpdate {
            channel_index: 3,
            param_path: "mix.lvl".to_string(),
            format: crate::channel_data_message::MsFormat::Val,
            value: serde_json::json!(-6.0),
        });
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        let must_arm_timer = handle_control_message(
            ControlAction::ScheduleFullResync,
            &mut state,
            &midi_tx,
            &event_tx,
        );

        assert!(
            must_arm_timer,
            "caller must arm the init-flush timer at FORCE_RESYNC_FLUSH_DELAY"
        );
        assert!(state.is_initializing);
        assert!(!state.has_sent_initial_track_dump);
        assert!(
            state.init_message_buffer.is_empty(),
            "a fresh window starts with an empty buffer"
        );
    }

    #[test]
    fn schedule_full_resync_control_action_does_not_clobber_an_in_flight_resync() {
        let mut state = running_state_without_ws_engine();
        state.is_initializing = true;
        state.has_sent_initial_track_dump = false;
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        let must_arm_timer = handle_control_message(
            ControlAction::ScheduleFullResync,
            &mut state,
            &midi_tx,
            &event_tx,
        );

        assert!(
            !must_arm_timer,
            "let the existing timer handle it, matching JS's early return"
        );
    }

    #[test]
    fn handshake_ack_while_running_with_dump_not_sent_finalizes_end_to_end() {
        let mut state = running_state_without_ws_engine();
        state.is_initializing = true;
        state.has_sent_initial_track_dump = false;
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        let frame = build_sysex_frame(&json!({"handshake": {"ack": true}}));
        let must_arm_timer = handle_inbound_midi_message(&frame, &mut state, &midi_tx, &event_tx);

        assert!(!must_arm_timer);
        assert!(
            state.has_sent_initial_track_dump,
            "the full wire-to-effect path must finalize, not just log"
        );
    }

    #[test]
    fn reset_while_running_schedules_a_full_resync_end_to_end() {
        let mut state = running_state_without_ws_engine();
        state.is_initializing = false;
        state.has_sent_initial_track_dump = true;
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        let frame = build_sysex_frame(&json!({"cmd": "RESET"}));
        let must_arm_timer = handle_inbound_midi_message(&frame, &mut state, &midi_tx, &event_tx);

        assert!(
            must_arm_timer,
            "RESET while running must tell the caller to arm the resync timer"
        );
        assert!(state.is_initializing);
    }

    /// End-to-end through the dispatcher: a Console 1 fader move on a real input slot both
    /// updates the cached track optimistically and lands an MS write on the outbound queue.
    #[test]
    fn input_fader_move_queues_an_ms_write_and_updates_the_cached_track() {
        let mut state = running_state_without_ws_engine();
        state.sends_mode_ms_send_index = None;
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        let slot = state
            .layout
            .iter()
            .find(|s| s.kind == LayoutSlotKind::Input && s.ms_primary.is_some())
            .cloned()
            .expect("default layout has input slots");
        let track_id = clone_or_create_track(&mut state, slot.object_id, &slot).track_id;

        let frame = build_sysex_frame(&json!({"trackId": track_id, "volume": -6.0}));
        handle_inbound_midi_message(&frame, &mut state, &midi_tx, &event_tx);

        assert_eq!(
            state.track_cache.get(slot.object_id).unwrap().volume,
            json!(-6.0)
        );
        let writes = state.ms_write_queue.take_all();
        for ch in &slot.ms_channels {
            let expected_path = format!("ch.{ch}.mix.lvl");
            let entry = writes
                .iter()
                .find(|e| e.path == expected_path)
                .unwrap_or_else(|| panic!("expected a write to {expected_path}"));
            assert_eq!(entry.format, "val");
            assert_eq!(entry.value, json!(-6.0));
        }
    }

    /// Selecting a bus master latches sends mode, and `set_sends_mode` then mirrors each
    /// Bus/Main track's volume into its send slots by reading the *cache*. When one message
    /// carries both `volume` and `selected`, the new volume therefore has to be published
    /// before that call and re-read after it — otherwise the mirroring runs off the stale
    /// volume, or the dispatcher's final writeback discards the mirroring altogether.
    #[test]
    fn selecting_a_bus_mirrors_the_same_messages_new_volume_into_its_send_slots() {
        let mut state = running_state_without_ws_engine();
        state.sends_mode_ms_send_index = None;
        state.sends_mode_subscribed_ms_send_index = None;
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        let slot = state
            .layout
            .iter()
            .find(|s| s.kind == LayoutSlotKind::Bus && s.ms_primary.is_some())
            .cloned()
            .expect("default layout has bus slots");
        let track_id = clone_or_create_track(&mut state, slot.object_id, &slot).track_id;

        let frame =
            build_sysex_frame(&json!({"trackId": track_id, "volume": -3.0, "selected": true}));
        handle_inbound_midi_message(&frame, &mut state, &midi_tx, &event_tx);

        assert_eq!(
            state.sends_mode_ms_send_index,
            Some(slot.ms_primary.unwrap() - state.bus_channel_start)
        );
        let cached = state.track_cache.get(slot.object_id).unwrap();
        assert_eq!(cached.volume, json!(-3.0));
        // The mirroring must reflect this message's own new volume, not the pre-move one.
        assert!(cached.send_levels.iter().all(|l| l == &json!(-3.0)));
        assert!(cached.send_on.iter().all(|on| *on));
    }

    /// `ws_engine` drops outbound sends during its initial connect and every reconnect wait,
    /// while `ws_handle` stays `Some` throughout. Flushing then must not record an echo
    /// suppression entry, or a real Mixing Station update carrying a coincidentally matching
    /// value could be swallowed as a false echo within the 150ms window.
    #[tokio::test]
    async fn ms_write_flush_while_disconnected_records_no_phantom_echo_suppression() {
        let mut state = running_state_without_ws_engine();
        // A real engine, attached but never connected (the port refuses), so this reproduces
        // the actual hazard: `ws_handle` is `Some` while the connection is not open. Gating on
        // the handle alone would wrongly treat this as sendable.
        state.ws_handle = Some(spawn_ws_engine(
            "ws://127.0.0.1:1".to_string(),
            std::time::Duration::from_secs(3600),
        ));
        state.ws_connected = false;
        state
            .ms_write_queue
            .queue("ch.4.mix.lvl".to_string(), "val".to_string(), json!(-6.0));

        flush_ms_write_queue(&mut state);

        // Drained regardless, matching JS's unconditional `msWriteQueue.clear()`.
        assert!(state.ms_write_queue.is_empty());
        assert!(!state.echo_tracker.should_suppress(
            "ch.4.mix.lvl|val",
            &json!(-6.0),
            std::time::Instant::now()
        ));
    }

    /// The connected counterpart: the same flush *does* arm suppression, so Mixing Station's
    /// echo of the bridge's own write gets ignored rather than bounced back to Console 1.
    #[test]
    fn ms_write_flush_while_connected_arms_echo_suppression() {
        let mut state = running_state_without_ws_engine();
        state.ws_connected = true;
        state
            .ms_write_queue
            .queue("ch.4.mix.lvl".to_string(), "val".to_string(), json!(-6.0));

        flush_ms_write_queue(&mut state);

        assert!(state.echo_tracker.should_suppress(
            "ch.4.mix.lvl|val",
            &json!(-6.0),
            std::time::Instant::now()
        ));
    }

    /// Status-bank slots other than Start carry no meaning in this direction — and crucially
    /// must not fall through into the real-track dispatch, which assumes an `ms_primary`.
    #[test]
    fn non_start_status_slot_midi_message_is_ignored() {
        let mut state = running_state_without_ws_engine();
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();
        send_status_bank_tracks(&mut state, &midi_tx, &event_tx);

        let status_track_id = state.track_cache.get(0).unwrap().track_id.clone();
        let frame = build_sysex_frame(&json!({"trackId": status_track_id, "selected": true}));
        handle_inbound_midi_message(&frame, &mut state, &midi_tx, &event_tx);

        assert_eq!(state.lifecycle, Lifecycle::Running);
        assert!(state.update_queue.is_empty());
        assert!(state.ms_write_queue.is_empty());
    }

    #[test]
    fn ws_connect_drops_cached_real_channel_tracks_but_keeps_the_status_bank() {
        let mut state = running_state_without_ws_engine();
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        let status_slot = state.layout[0].clone();
        let real_slot = state
            .layout
            .iter()
            .find(|s| s.object_id > START_SLOT_OBJECT_ID)
            .cloned()
            .expect("default layout has real channel slots");
        let colors = default_colors(&state.config);
        let bus_channel_start = state.bus_channel_start;
        for slot in [&status_slot, &real_slot] {
            state.track_cache.get_or_create(
                slot.object_id,
                slot,
                state.lifecycle,
                &colors,
                bus_channel_start,
                &mut state.rng,
            );
        }

        // handle_ws_connected only starts the /console/information race now; the actual cache
        // drop happens once finish_ws_connect runs (immediately here, simulating the timeout
        // branch resolving with no reply ever having arrived).
        handle_ws_connected(&mut state, &event_tx);
        finish_ws_connect(&mut state, &midi_tx, &event_tx);

        assert!(state.track_cache.get(real_slot.object_id).is_none());
        assert!(state.track_cache.get(status_slot.object_id).is_some());
        assert_eq!(state.sends_mode_ms_send_index, None);
    }

    /// Queues a `Shutdown` behind whatever the caller already sent (one FIFO channel, so
    /// everything queued earlier is processed first) and collects every event the runtime
    /// emitted before its task exited.
    async fn drain_runtime_events(mut handle: BridgeRuntimeHandle) -> Vec<BridgeEvent> {
        handle.command_tx.send(BridgeCommand::Shutdown).unwrap();
        let collect = async {
            let mut events = Vec::new();
            while let Some(event) = handle.events.recv().await {
                events.push(event);
            }
            events
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), collect)
            .await
            .expect("runtime should shut down promptly")
    }

    /// Standby-leak guard, matching JS's `config:apply` stdin handler (`index.js:526-545`).
    /// Without it, the resync path arms the init-flush timer, and `finalize_initialization`
    /// has no lifecycle check of its own — so the timer would push a full real-channel
    /// `trackBatch` dump to hardware that should only be showing the status/Start bank.
    #[tokio::test]
    async fn config_apply_while_standby_is_ignored() {
        let handle = spawn_bridge_runtime(RuntimeConfig::default());
        handle
            .command_tx
            .send(BridgeCommand::ConfigApply(BridgeConfigPatch {
                console1_main_color: Some(0x123456),
                ..Default::default()
            }))
            .unwrap();

        let events = drain_runtime_events(handle).await;

        assert!(events.contains(&BridgeEvent::Log(
            "[Lifecycle] Ignoring config:apply -- not running.".to_string()
        )));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, BridgeEvent::ConfigApplied { .. })),
            "the resync path must not run in standby: {events:?}"
        );
    }

    /// JS's `lifecycle:start` calls `enterRunningState(config)`, which applies the patch with
    /// the PLAIN `applyRuntimeConfig` — no resync, and no config-related output. Only the
    /// lifecycle log/event `enter_running_state` already emits should appear.
    #[tokio::test]
    async fn start_with_a_config_patch_emits_no_config_apply_events() {
        let handle = spawn_bridge_runtime(RuntimeConfig::default());
        handle
            .command_tx
            .send(BridgeCommand::Start(Some(BridgeConfigPatch {
                console1_main_color: Some(0x123456),
                ..Default::default()
            })))
            .unwrap();

        let events = drain_runtime_events(handle).await;

        assert!(events.contains(&BridgeEvent::LifecycleChanged(Lifecycle::Running)));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, BridgeEvent::ConfigApplied { .. })),
            "Start must not emit ConfigApplied: {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, BridgeEvent::Log(line) if line.starts_with("[config]"))),
            "Start must not emit config logs: {events:?}"
        );
    }

    /// The plain `applyRuntimeConfig` port used by the Start path: updates the live config and
    /// rebuilds the send mapping, with none of `apply_config_patch`'s resync side effects.
    #[test]
    fn apply_runtime_config_updates_config_and_send_mapping() {
        let mut state = running_state_without_ws_engine();
        let patch = BridgeConfigPatch {
            console1_main_color: Some(0x123456),
            c1_send_to_ms_bus_number: Some(vec![2, 1, 3, 4, 5, 6]),
            ..Default::default()
        };

        apply_runtime_config(&mut state, &patch);

        assert_eq!(state.config.console1_main_color, 0x123456);
        assert_eq!(
            state.config.c1_send_to_ms_bus_number,
            vec![2, 1, 3, 4, 5, 6]
        );
        assert_eq!(
            state.send_mapping,
            build_send_mapping(&[2, 1, 3, 4, 5, 6], 16)
        );
        // None of the resync branch's state was touched.
        assert!(!state.is_initializing);
    }

    #[test]
    fn apply_runtime_config_rebuilds_send_mapping_using_the_live_bus_channel_count() {
        let mut state = running_state_without_ws_engine();
        state.console_architecture.bus_channel_count = 4;
        let patch = BridgeConfigPatch {
            c1_send_to_ms_bus_number: Some(vec![10]), // bus 10 doesn't exist on a 4-bus console
            ..Default::default()
        };

        apply_runtime_config(&mut state, &patch);

        // Old hardcoded bound (16) would have accepted bus 10 (index 9) as in-range. With only
        // 4 live buses, index 9 is out of range and must fall back to the identity mapping (C1
        // send slot 0 maps to MS send index 0, matching its own slot position).
        assert_eq!(state.send_mapping.c1_to_ms_send_index[0], 0);
    }

    #[tokio::test]
    async fn config_apply_with_no_changes_only_logs() {
        let mut state = running_state_without_ws_engine();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let sleep = tokio::time::sleep(std::time::Duration::from_secs(3600));
        tokio::pin!(sleep);
        let mut armed = false;

        apply_config_patch(
            &mut state,
            &BridgeConfigPatch::default(),
            &event_tx,
            &midi_tx,
            &mut sleep.as_mut(),
            &mut armed,
        )
        .await;

        let event = event_rx.try_recv().unwrap();
        assert_eq!(
            event,
            BridgeEvent::Log("[config] No changes to apply".to_string())
        );
        assert!(event_rx.try_recv().is_err()); // nothing else emitted
        assert!(!armed);
    }

    #[tokio::test]
    async fn config_apply_url_change_while_standby_does_not_reconnect() {
        let mut state = running_state_without_ws_engine();
        state.lifecycle = Lifecycle::Standby;
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let sleep = tokio::time::sleep(std::time::Duration::from_secs(3600));
        tokio::pin!(sleep);
        let mut armed = false;

        let patch = BridgeConfigPatch {
            mixing_station_ws_url: Some("ws://127.0.0.1:9999".to_string()),
            ..Default::default()
        };
        apply_config_patch(
            &mut state,
            &patch,
            &event_tx,
            &midi_tx,
            &mut sleep.as_mut(),
            &mut armed,
        )
        .await;

        assert!(state.ws_handle.is_none()); // standby -> no reconnect attempted
        assert!(!armed); // url_changed branch returns early, never arms the init-flush timer
        let _ = event_rx.try_recv(); // "[config] Applied updates..." log
        let event = event_rx.try_recv().unwrap();
        assert_eq!(
            event,
            BridgeEvent::ConfigApplied {
                url_changed: true,
                anything_changed: true
            }
        );
    }

    #[tokio::test]
    async fn config_apply_non_url_change_while_disconnected_arms_a_500ms_resync() {
        let mut state = running_state_without_ws_engine();
        state.lifecycle = Lifecycle::Running;
        state.ws_connected = false;
        state.has_sent_initial_track_dump = true; // sentinel: must flip back to false
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let sleep = tokio::time::sleep(std::time::Duration::from_secs(3600));
        tokio::pin!(sleep);
        let mut armed = false;

        let patch = BridgeConfigPatch {
            console1_main_color: Some(0x123456),
            ..Default::default()
        };
        apply_config_patch(
            &mut state,
            &patch,
            &event_tx,
            &midi_tx,
            &mut sleep.as_mut(),
            &mut armed,
        )
        .await;

        assert!(armed);
        assert!(state.is_initializing);
        assert!(!state.has_sent_initial_track_dump);
    }

    #[tokio::test]
    async fn config_apply_rebuilds_send_mapping_when_anything_changed() {
        let mut state = running_state_without_ws_engine();
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let sleep = tokio::time::sleep(std::time::Duration::from_secs(3600));
        tokio::pin!(sleep);
        let mut armed = false;

        let patch = BridgeConfigPatch {
            c1_send_to_ms_bus_number: Some(vec![2, 1, 3, 4, 5, 6]),
            ..Default::default()
        };
        apply_config_patch(
            &mut state,
            &patch,
            &event_tx,
            &midi_tx,
            &mut sleep.as_mut(),
            &mut armed,
        )
        .await;

        let expected = build_send_mapping(&[2, 1, 3, 4, 5, 6], 16);
        assert_eq!(state.send_mapping, expected);
    }

    /// Regression: `enter_running_state` used to spawn the WS engine without porting the rest of
    /// JS's `enterRunningState`, so a `Start` patch that reordered tracks updated `config` but
    /// never `layout`, and nothing forced a track dump when Mixing Station was slow to connect.
    #[tokio::test]
    async fn entering_running_rebuilds_the_layout_and_requests_the_forced_resync() {
        let mut state = running_state_without_ws_engine();
        state.lifecycle = Lifecycle::Standby;
        state.has_sent_initial_track_dump = true; // sentinel: must flip back to false
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        // The Start path applies its patch before entering Running, as the command loop does.
        let patch = BridgeConfigPatch {
            input_track_order: Some(vec![TrackOrderEntry::Single(5), TrackOrderEntry::Single(3)]),
            ..Default::default()
        };
        apply_runtime_config(&mut state, &patch);

        let arm_init_flush = enter_running_state(&mut state, &midi_tx, &event_tx);

        let input_channels: Vec<Option<usize>> = state
            .layout
            .iter()
            .filter(|s| s.kind == LayoutSlotKind::Input)
            .map(|s| s.ms_primary)
            .collect();
        // Channels the order doesn't mention still follow, so only the head reflects the patch.
        assert_eq!(input_channels[..2], [Some(5), Some(3)]);
        assert_eq!(
            state.object_ids_by_ms_channel,
            object_ids_by_ms_channel(&state.layout),
            "the channel->object index must be rebuilt from the new layout"
        );
        assert!(arm_init_flush, "caller must arm the 500ms forced resync");
        assert!(state.is_initializing);
        assert!(!state.has_sent_initial_track_dump);
    }

    /// The hardware Start button reaches `enter_running_state` three levels down, so its arming
    /// signal has to survive the whole dispatch chain — a Stop must not arm anything.
    #[tokio::test]
    async fn hardware_start_bubbles_the_forced_resync_request_up_to_the_event_loop() {
        let mut state = running_state_without_ws_engine();
        state.lifecycle = Lifecycle::Standby;
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let event_tx = discarding_event_tx();

        assert!(handle_hardware_trigger(
            HardwareTrigger::Start,
            &mut state,
            &midi_tx,
            &event_tx
        ));
        assert!(!handle_hardware_trigger(
            HardwareTrigger::Stop,
            &mut state,
            &midi_tx,
            &event_tx
        ));
    }

    /// The lifecycle transitions emit both a human-readable log line and the structured
    /// `LifecycleChanged` event, so a GUI can drive state off the latter without parsing logs.
    #[test]
    fn lifecycle_transitions_emit_a_structured_event_alongside_the_log_line() {
        let mut state = running_state_without_ws_engine();
        let (midi_tx, _midi_rx) = std::sync::mpsc::channel();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();

        enter_standby_state(&mut state, &midi_tx, &event_tx);

        assert_eq!(
            event_rx.try_recv(),
            Ok(BridgeEvent::Log("[Lifecycle] standby".to_string()))
        );
        assert_eq!(
            event_rx.try_recv(),
            Ok(BridgeEvent::LifecycleChanged(Lifecycle::Standby))
        );
    }
}
