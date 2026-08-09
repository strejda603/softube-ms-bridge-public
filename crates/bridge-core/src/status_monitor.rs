//! Pure status-detection logic for the 2 topbar indicators (Mixing Station, Console 1
//! On-Screen Display), ported from `app/statusMonitor.js`'s `computeStatus()`. Takes
//! already-gathered process command lines as a plain string slice -- the impure gathering
//! (real OS process listing) lives in `bridge-tauri`, kept separate so this can be
//! unit-tested without touching real OS state.
//!
//! Note: since matching happens against full command lines (see `compute_status`'s
//! `process_command_lines` parameter), a process whose invocation merely *mentions* a needle
//! (e.g. in a config-file argument) would also trip that indicator -- this is parity with the
//! JS original's identical `ps -Ao args=` substring-matching behavior, not a new regression.

use serde::Serialize;

/// One snapshot of both status indicators. Field names match `app/statusMonitor.js`'s
/// `computeStatus()` return shape for the 2 indicators this edition keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusSnapshot {
    pub mixing_station: bool,
    pub console1_osd: bool,
}

/// Computes both indicators from an already-gathered process command-line list. Matching is
/// case-sensitive substring, identical to `app/statusMonitor.js:42-56`.
///
/// `process_command_lines` must be FULL command lines (`ps -Ao args=`-style: the full
/// executable path plus any arguments, one string per process), not bare executable/process
/// names -- some needles below only appear in a path component of the invocation, not in the
/// bare executable name.
///
/// `mixing_station`'s needle is the executable basename itself ("mixing-station"), not a
/// display/bundle name -- confirmed via real installs on both platforms: macOS's `.app` bundle
/// wraps an executable also literally named "mixing-station"
/// (`.../Mixing Station.app/Contents/MacOS/mixing-station`), and Windows' real executable is
/// `mixing-station-pc.exe` (confirmed via `Get-CimInstance Win32_Process` against a real
/// Windows 11 install: `C:\Users\<user>\AppData\Local\mixing-station-pc\mixing-station-pc.exe`).
/// An earlier version of this needle was "Mixing Station" (capitalized, with a space) -- that
/// only ever matched the macOS `.app` bundle's *display* name, which was verified against a real
/// macOS install but never against a real Windows one; on Windows it silently never matched
/// anything, leaving the topbar dot permanently red despite Mixing Station actually running.
///
/// `console1_osd`'s needle ("Softube On-Screen Display") is a display name too, but unlike
/// `mixing_station`'s old needle it genuinely does appear on Windows -- confirmed via
/// `Get-CimInstance Win32_Process` against a real Windows 11 install: the executable itself is
/// literally named `Softube On-Screen Display (x64).exe`, installed at
/// `C:\Program Files\Softube\Plug-Ins 64-bit\`. No fix needed here; this is documented so a
/// future reader doesn't assume it has the same bug `mixing_station` had just because both
/// needles started life as macOS-only-verified display names.
pub fn compute_status(process_command_lines: &[String]) -> StatusSnapshot {
    let has_process = |needle: &str| process_command_lines.iter().any(|n| n.contains(needle));

    StatusSnapshot {
        mixing_station: has_process("mixing-station"),
        console1_osd: has_process("Softube On-Screen Display"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn process_command_lines_match_by_substring() {
        let processes = names(&[
            "/Applications/Mixing Station.app/Contents/MacOS/mixing-station",
            "/Applications/Softube On-Screen Display.app/Contents/MacOS/Console1OSD_APP_Protect",
        ]);
        let snap = compute_status(&processes);
        assert!(snap.mixing_station);
        assert!(snap.console1_osd);
    }

    #[test]
    fn matching_is_case_sensitive() {
        let processes = names(&["/Applications/Mixing Station.app/Contents/MacOS/MIXING-STATION"]); // wrong case on the matched basename itself
        let snap = compute_status(&processes);
        assert!(!snap.mixing_station);
    }

    /// Regression test for a real bug: the needle used to be "Mixing Station" (the macOS `.app`
    /// bundle's display name), which was verified against a real macOS install but never a real
    /// Windows one -- it silently never matched on Windows, leaving the topbar dot permanently
    /// red despite Mixing Station actually running. Fixture is the exact real command line from
    /// `Get-CimInstance Win32_Process` against a real Windows 11 install.
    #[test]
    fn windows_mixing_station_process_is_detected() {
        let processes = names(&[
            r"C:\Users\danielpitra\AppData\Local\mixing-station-pc\mixing-station-pc.exe",
        ]);
        let snap = compute_status(&processes);
        assert!(snap.mixing_station);
    }

    /// Confirms `console1_osd`'s needle, unlike `mixing_station`'s old one, genuinely does work
    /// on Windows -- fixture is the exact real command line from `Get-CimInstance Win32_Process`
    /// against a real Windows 11 install.
    #[test]
    fn windows_console1_osd_process_is_detected() {
        let processes = names(&[
            r"C:\Program Files\Softube\Plug-Ins 64-bit\Softube On-Screen Display (x64).exe",
        ]);
        let snap = compute_status(&processes);
        assert!(snap.console1_osd);
    }

    #[test]
    fn empty_input_produces_all_false() {
        let snap = compute_status(&[]);
        assert_eq!(
            snap,
            StatusSnapshot {
                mixing_station: false,
                console1_osd: false,
            }
        );
    }

    #[test]
    fn status_snapshot_serializes_camel_case() {
        let snap = StatusSnapshot {
            mixing_station: false,
            console1_osd: true,
        };
        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains("\"mixingStation\":false"));
        assert!(json.contains("\"console1Osd\":true"));
    }
}
