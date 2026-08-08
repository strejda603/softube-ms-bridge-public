//! Impure gatherers + the dedicated polling thread feeding `bridge_core::status_monitor::
//! compute_status` -- MIDI port name enumeration (via `midir`) and running-process
//! command-line listing (via `sysinfo`), cross-platform macOS+Windows, replacing the old
//! Electron app's macOS-only `ps -Ao args=` which silently degraded to always-off on Windows.
//! Runs on its own dedicated `std::thread`, not a tokio task, for two reasons: (1) all three
//! gatherers are blocking OS calls, matching this codebase's existing `bridge_core::midi_io`
//! precedent of a dedicated thread for blocking MIDI work rather than parking a tokio worker;
//! (2) `midir::MidiInput`/`MidiOutput` must be constructed ONCE and held for the thread's
//! whole lifetime, not per-tick -- on macOS, each construction creates a CoreMIDI client that
//! is NEVER disposed (Apple explicitly warns against disposing the last/only client, so the
//! `coremidi` crate's `Client` has no `Drop` impl at all), so constructing one per 2s poll
//! tick for the app's entire lifetime would leak thousands of CoreMIDI clients per session
//! with zero benefit (port enumeration doesn't even use the client handle internally). A
//! long-lived `sysinfo::System`, refreshed narrowly for just `cmd` via
//! `refresh_processes_specifics` each tick, is both cheaper and no less fresh than a fresh
//! `System::new_all()` per tick -- `sysinfo` already prunes dead processes and detects PID
//! reuse internally on every refresh of a reused `System`.

use bridge_core::status_monitor::{compute_status, StatusSnapshot};
use std::time::Duration;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};

fn gather_midi_input_names(midi_in: &midir::MidiInput) -> Vec<String> {
    midi_in
        .ports()
        .iter()
        .filter_map(|p| midi_in.port_name(p).ok())
        .collect()
}

fn gather_midi_output_names(midi_out: &midir::MidiOutput) -> Vec<String> {
    midi_out
        .ports()
        .iter()
        .filter_map(|p| midi_out.port_name(p).ok())
        .collect()
}

/// Lists every currently-running process's full command line (joined argv, mirroring `ps -Ao
/// args=`'s one-line-per-process format). `sysinfo::Process::name()` (bare executable
/// basename) is deliberately NOT used: verified empirically that none of `compute_status`'s 4
/// process-detection needles match against executable basenames -- only the full joined
/// command line reliably contains the needle, via the `.app` bundle path component of
/// argv[0]. Refreshes only `cmd` on the given long-lived `System` (not a fresh `new_all()`
/// each call) -- see the module doc for why this is both cheaper and no less fresh.
fn gather_process_command_lines(sys: &mut sysinfo::System) -> Vec<String> {
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    sys.processes()
        .values()
        .map(|p| {
            p.cmd()
                .iter()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// A CoreMIDI client's own view of available sources/destinations is a local mirror
/// synchronized via notification IPC messages delivered on the run loop active when the
/// client was created -- without an active run loop pumping on this thread, `ports()` never
/// picks up devices that connect/disconnect after this client's initial snapshot (confirmed
/// on real hardware: process-based dots update live since OS process enumeration has no such
/// dependency, but MIDI-based dots only refreshed after a full app relaunch, which re-snapshots
/// at construction time). Registering one notification-capable client (a no-op callback is
/// enough -- we don't need to react to the notification ourselves, only need CoreMIDI's
/// registry mirror kept current as a side effect of the run loop processing it) and pumping
/// the run loop each tick keeps the registry state current for the whole thread, including for
/// `midir`'s own separately-constructed `MidiInput`/`MidiOutput` clients used for enumeration.
#[cfg(target_os = "macos")]
fn register_coremidi_notification_client() -> Option<coremidi::Client> {
    coremidi::Client::new_with_notifications(
        "softube-ms-bridge-status-probe-notify",
        |_notification: &coremidi::Notification| {},
    )
    .ok()
}

/// Runs the status-poll loop forever on the CALLING thread -- meant to be spawned via
/// `std::thread::spawn`, never a tokio task (see the module doc for why). Constructs
/// `MidiInput`/`MidiOutput`/`System` exactly once and holds them for the loop's whole
/// lifetime. Calls `on_change` with each new snapshot only when it differs from the last one
/// seen, so a stable session doesn't produce IPC/webview traffic every 2s for nothing.
pub fn run_status_poll_loop(on_change: impl Fn(StatusSnapshot)) {
    let midi_in = midir::MidiInput::new("softube-ms-bridge-status-probe").ok();
    let midi_out = midir::MidiOutput::new("softube-ms-bridge-status-probe").ok();
    if midi_in.is_none() || midi_out.is_none() {
        eprintln!(
            "[status] Failed to initialize MIDI client for status polling -- iPad/SPD-SX/MIDI Maestro dots will always read off this session"
        );
    }
    // Held for the whole function -- never dropped, since this loop never returns. Its only
    // job is to keep CoreMIDI's notification IPC flowing on this thread; see its own doc
    // comment for why that's needed even though we never touch the notification payload.
    #[cfg(target_os = "macos")]
    let _notify_client = register_coremidi_notification_client();
    let mut sys = sysinfo::System::new();

    let mut last: Option<StatusSnapshot> = None;
    loop {
        let midi_inputs = midi_in.as_ref().map(gather_midi_input_names).unwrap_or_default();
        let midi_outputs = midi_out.as_ref().map(gather_midi_output_names).unwrap_or_default();
        let process_command_lines = gather_process_command_lines(&mut sys);
        let snapshot = compute_status(&midi_inputs, &midi_outputs, &process_command_lines);
        if last != Some(snapshot) {
            on_change(snapshot);
            last = Some(snapshot);
        }
        #[cfg(target_os = "macos")]
        {
            // Pumping the run loop here (instead of a plain sleep) both provides the 2s
            // interval AND lets any pending CoreMIDI notification IPC get processed, keeping
            // ports()'s view of connected devices current for the NEXT iteration -- this is
            // what actually fixes the live-hotplug-detection gap, not just the notify client's
            // existence alone.
            unsafe {
                core_foundation::runloop::CFRunLoop::run_in_mode(
                    core_foundation::runloop::kCFRunLoopDefaultMode,
                    Duration::from_secs(2),
                    false,
                );
            }
        }
        #[cfg(not(target_os = "macos"))]
        std::thread::sleep(Duration::from_secs(2));
    }
}
