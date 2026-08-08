//! Pure status-detection logic for the 7 topbar indicators, ported from
//! `app/statusMonitor.js`'s `computeStatus()`. Takes already-gathered MIDI port names and
//! process command lines as plain string slices -- the impure gathering (real MIDI enumeration,
//! real OS process listing) lives in `bridge-tauri`, kept separate so this can be unit-tested
//! without touching real hardware/OS state, same rationale as `console1_status_bank.rs`.
//!
//! Note: since matching happens against full command lines (see `compute_status`'s
//! `process_command_lines` parameter), a process whose invocation merely *mentions* a needle
//! (e.g. in a config-file argument) would also trip that indicator -- this is parity with the
//! JS original's identical `ps -Ao args=` substring-matching behavior, not a new regression.

use serde::Serialize;

/// One snapshot of all 7 status indicators. Field names match
/// `console1_status_bank::STATUS_BANK_INDICATORS`'s `key`s and `app/statusMonitor.js`'s
/// `computeStatus()` return shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusSnapshot {
    pub ipad: bool,
    pub spd_sx_pro: bool,
    pub midi_maestro: bool,
    pub bome_mtp: bool,
    pub mixing_station: bool,
    pub console1_osd: bool,
    pub ableton_live: bool,
}

/// Computes all 7 indicators from already-gathered name lists. Matching is case-sensitive
/// substring, identical to `app/statusMonitor.js:42-56`.
///
/// `process_command_lines` must be FULL command lines (`ps -Ao args=`-style: the full
/// executable path plus any arguments, one string per process), not bare executable/process
/// names. All 4 process-detection needles below only appear in the `.app` bundle path
/// component of the invocation -- the actual executable basenames don't contain them at all
/// (verified against real installed app bundles: e.g. Ableton Live 12's real executable is
/// just named "Live", Mixing Station's is "mixing-station", etc.). A gatherer that passes
/// basenames instead of full command lines will silently produce `false` for all 4 process
/// dots with no test failure to catch it.
pub fn compute_status(
    midi_input_names: &[String],
    midi_output_names: &[String],
    process_command_lines: &[String],
) -> StatusSnapshot {
    let has_input = |needle: &str| midi_input_names.iter().any(|n| n.contains(needle));
    let has_output = |needle: &str| midi_output_names.iter().any(|n| n.contains(needle));
    let has_process = |needle: &str| process_command_lines.iter().any(|n| n.contains(needle));

    StatusSnapshot {
        ipad: has_input("iPad") && has_output("iPad"),
        spd_sx_pro: has_output("SPD-SX PRO"),
        // "MIDI Maestro" deliberately also matches the Bluetooth variant's port name -- no
        // separate check needed, mirrors the JS original's reasoning.
        midi_maestro: has_input("MIDI Maestro"),
        bome_mtp: has_process("Bome MIDI Translator Pro"),
        mixing_station: has_process("Mixing Station"),
        console1_osd: has_process("Softube On-Screen Display"),
        ableton_live: has_process("Ableton Live 12 Suite"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn ipad_true_only_when_both_input_and_output_match() {
        let inputs = names(&["Some iPad Input"]);
        let outputs = names(&["Some iPad Output"]);
        let snap = compute_status(&inputs, &outputs, &[]);
        assert!(snap.ipad);
    }

    #[test]
    fn ipad_false_when_only_input_matches() {
        let inputs = names(&["Some iPad Input"]);
        let outputs = names(&["Unrelated Output"]);
        let snap = compute_status(&inputs, &outputs, &[]);
        assert!(!snap.ipad);
    }

    #[test]
    fn ipad_false_when_only_output_matches() {
        let inputs = names(&["Unrelated Input"]);
        let outputs = names(&["Some iPad Output"]);
        let snap = compute_status(&inputs, &outputs, &[]);
        assert!(!snap.ipad);
    }

    #[test]
    fn spd_sx_pro_matches_output_substring() {
        let outputs = names(&["Roland SPD-SX PRO MIDI Out"]);
        let snap = compute_status(&[], &outputs, &[]);
        assert!(snap.spd_sx_pro);
    }

    #[test]
    fn midi_maestro_matches_input_substring() {
        let inputs = names(&["MIDI Maestro Bluetooth"]);
        let snap = compute_status(&inputs, &[], &[]);
        assert!(snap.midi_maestro);
    }

    #[test]
    fn process_command_lines_match_by_substring() {
        let processes = names(&[
            "/Applications/Bome MIDI Translator Pro.app/Contents/MacOS/MIDITranslatorPro",
            "/Applications/Mixing Station.app/Contents/MacOS/mixing-station",
            "/Applications/Softube On-Screen Display.app/Contents/MacOS/Console1OSD_APP_Protect",
            "/Applications/Ableton Live 12 Suite.app/Contents/MacOS/Live",
        ]);
        let snap = compute_status(&[], &[], &processes);
        assert!(snap.bome_mtp);
        assert!(snap.mixing_station);
        assert!(snap.console1_osd);
        assert!(snap.ableton_live);
    }

    #[test]
    fn matching_is_case_sensitive() {
        let inputs = names(&["some ipad input"]); // lowercase, doesn't match "iPad"
        let outputs = names(&["some ipad output"]);
        let processes = names(&["/applications/mixing station.app/contents/macos/mixing-station"]); // lowercase, doesn't match "Mixing Station"
        let snap = compute_status(&inputs, &outputs, &processes);
        assert!(!snap.ipad);
        assert!(!snap.mixing_station);
    }

    #[test]
    fn empty_inputs_produce_all_false() {
        let snap = compute_status(&[], &[], &[]);
        assert_eq!(
            snap,
            StatusSnapshot {
                ipad: false,
                spd_sx_pro: false,
                midi_maestro: false,
                bome_mtp: false,
                mixing_station: false,
                console1_osd: false,
                ableton_live: false,
            }
        );
    }

    #[test]
    fn status_snapshot_serializes_camel_case() {
        let snap = StatusSnapshot {
            ipad: true,
            spd_sx_pro: false,
            midi_maestro: false,
            bome_mtp: false,
            mixing_station: false,
            console1_osd: true,
            ableton_live: false,
        };
        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains("\"ipad\":true"));
        assert!(json.contains("\"spdSxPro\":false"));
        assert!(json.contains("\"console1Osd\":true"));
    }
}
