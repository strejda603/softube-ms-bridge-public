//! Impure gatherer + the dedicated polling thread feeding `bridge_core::status_monitor::
//! compute_status` -- running-process command-line listing (via `sysinfo`), cross-platform
//! macOS+Windows, replacing the old Electron app's macOS-only `ps -Ao args=` which silently
//! degraded to always-off on Windows. Runs on its own dedicated `std::thread`, not a tokio
//! task, matching this codebase's existing `bridge_core::midi_io` precedent of a dedicated
//! thread for blocking OS work. A long-lived `sysinfo::System`, refreshed narrowly for just
//! `cmd` via `refresh_processes_specifics` each tick, is both cheaper and no less fresh than a
//! fresh `System::new_all()` per tick -- `sysinfo` already prunes dead processes and detects
//! PID reuse internally on every refresh of a reused `System`.

use bridge_core::status_monitor::{compute_status, StatusSnapshot};
use std::time::Duration;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};

/// Lists every currently-running process's full command line (joined argv, mirroring `ps -Ao
/// args=`'s one-line-per-process format). `sysinfo::Process::name()` (bare executable
/// basename) is deliberately NOT used: verified empirically that neither of
/// `compute_status`'s 2 process-detection needles match against executable basenames -- only
/// the full joined command line reliably contains the needle, via the `.app` bundle path
/// component of argv[0]. Refreshes only `cmd` on the given long-lived `System` (not a fresh
/// `new_all()` each call) -- see the module doc for why this is both cheaper and no less
/// fresh.
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

/// Runs the status-poll loop forever on the CALLING thread -- meant to be spawned via
/// `std::thread::spawn`, never a tokio task (see the module doc for why). Constructs
/// `System` exactly once and holds it for the loop's whole lifetime. Calls `on_change` with
/// each new snapshot only when it differs from the last one seen, so a stable session doesn't
/// produce IPC/webview traffic every 2s for nothing.
pub fn run_status_poll_loop(on_change: impl Fn(StatusSnapshot)) {
    let mut sys = sysinfo::System::new();

    let mut last: Option<StatusSnapshot> = None;
    loop {
        let process_command_lines = gather_process_command_lines(&mut sys);
        let snapshot = compute_status(&process_command_lines);
        if last != Some(snapshot) {
            on_change(snapshot);
            last = Some(snapshot);
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
