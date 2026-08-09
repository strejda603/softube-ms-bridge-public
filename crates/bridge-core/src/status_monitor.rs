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
/// names. Both needles below only appear in the `.app` bundle path component of the
/// invocation -- the actual executable basenames don't contain them at all (verified against
/// real installed app bundles: e.g. Mixing Station's real executable is just named
/// "mixing-station"). A gatherer that passes basenames instead of full command lines will
/// silently produce `false` for both dots with no test failure to catch it.
pub fn compute_status(process_command_lines: &[String]) -> StatusSnapshot {
    let has_process = |needle: &str| process_command_lines.iter().any(|n| n.contains(needle));

    StatusSnapshot {
        mixing_station: has_process("Mixing Station"),
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
        let processes = names(&["/applications/mixing station.app/contents/macos/mixing-station"]); // lowercase, doesn't match "Mixing Station"
        let snap = compute_status(&processes);
        assert!(!snap.mixing_station);
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
