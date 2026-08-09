//! Sends-mode state and pure decision logic: latching sends mode from a Bus/Main
//! selection, and how input-track sends-mode updates map onto Console 1 fields.
//!
//! Orchestration (WS subscribe/unsubscribe, one-shot refresh GETs, iterating the full
//! layout to sync Bus/Main send-slot displays) stays in `bridge-cli` -- this module only
//! covers the parts that are pure given already-available state.
//!
//! Port of `index.js`'s `maybeLatchSendsModeFromSelection`, `handleSendsModeInputUpdate`,
//! `mirrorConsole1SendSlotsFromVolume`, `clearConsole1SendSlots`, `isSendsModeActive`.

use crate::send_mapping::{SendMapping, NUMBER_OF_SENDS};
use crate::track_cache::TrackInfo;
use crate::track_layout::{LayoutSlot, LayoutSlotKind};
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn is_sends_mode_active(sends_mode_ms_send_index: Option<usize>) -> bool {
    sends_mode_ms_send_index.is_some()
}

/// Given a `"selected"` update on a Bus/Main slot, decides the new sends-mode state.
/// Returns `None` if this update doesn't affect sends mode at all (wrong param path,
/// non-Bus/Main slot, or a deselection). Returns `Some(new_state)` when it does --
/// `Some(Some(ms_send_index))` to enter sends mode, `Some(None)` to leave it.
pub fn maybe_latch_sends_mode_from_selection(
    slot: &LayoutSlot,
    param_path: &str,
    value: &Value,
    bus_channel_start: usize,
) -> Option<Option<usize>> {
    if param_path != "selected" || !value.as_bool().unwrap_or(false) {
        return None;
    }
    match slot.kind {
        LayoutSlotKind::Bus => {
            let ms_primary = slot.ms_primary?;
            Some(Some(ms_primary.saturating_sub(bus_channel_start)))
        }
        LayoutSlotKind::Main => Some(None),
        _ => None,
    }
}

#[derive(Debug, Default, Clone)]
pub struct InputSendState {
    /// MS channel index -> last known (on, pan) for the active send, keyed per-channel so
    /// state from one send index doesn't bleed into another when the active send changes.
    state: HashMap<usize, (Option<bool>, Option<f64>)>,
}

impl InputSendState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.state.clear();
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SendsModeInputUpdateOutcome {
    /// Not an input slot, sends mode inactive, or an unrelated param path -- caller should
    /// fall through to standard `ms_param_apply` routing.
    NotHandled,
    /// Standard `mix.on`/`mix.pan` update while sends mode is active -- consumed, no
    /// Console 1 change (standard mute/pan are hidden while sends mode is active).
    Suppressed,
    /// Applied to `track` in place; `changed` holds the trackBatch-eligible fields that
    /// changed (route through `update_queue::UpdateQueue`, same as `ms_param_apply`'s output).
    Applied { changed: HashMap<String, Value> },
}

pub struct SendsModeInputUpdateArgs<'a> {
    pub slot: &'a LayoutSlot,
    pub channel_index: usize,
    pub param_path: &'a str,
    pub value: &'a Value,
    pub sends_mode_ms_send_index: Option<usize>,
    pub mapping: &'a SendMapping,
    pub track: &'a mut TrackInfo,
    pub input_send_state: &'a mut InputSendState,
}

pub fn handle_sends_mode_input_update(
    args: SendsModeInputUpdateArgs,
) -> SendsModeInputUpdateOutcome {
    let SendsModeInputUpdateArgs {
        slot,
        channel_index,
        param_path,
        value,
        sends_mode_ms_send_index,
        mapping,
        track,
        input_send_state,
    } = args;

    if slot.kind != LayoutSlotKind::Input {
        return SendsModeInputUpdateOutcome::NotHandled;
    }
    let Some(ms_send_index) = sends_mode_ms_send_index else {
        return SendsModeInputUpdateOutcome::NotHandled;
    };

    if param_path == "mix.on" || param_path == "mix.pan" {
        return SendsModeInputUpdateOutcome::Suppressed;
    }

    let send_on_path = format!("mix.sends.{ms_send_index}.on");
    let send_pan_path = format!("mix.sends.{ms_send_index}.pan");

    if param_path == send_on_path {
        let on = value.as_bool().unwrap_or(false);
        let entry = input_send_state.state.entry(channel_index).or_default();
        entry.0 = Some(on);

        let mut changed = HashMap::new();
        let Some(&c1_slot) = mapping.ms_send_index_to_c1_slot.get(&ms_send_index) else {
            return SendsModeInputUpdateOutcome::Applied { changed };
        };
        let field = format!("send{}On", c1_slot + 1);
        if track.send_on[c1_slot] != on {
            track.send_on[c1_slot] = on;
            changed.insert(field, json!(on));
        }
        let next_muted = !on;
        if track.mute != next_muted {
            track.mute = next_muted;
            changed.insert("mute".to_string(), json!(next_muted));
        }
        return SendsModeInputUpdateOutcome::Applied { changed };
    }

    if param_path == send_pan_path {
        let pan = value.as_f64().unwrap_or(0.5);
        let entry = input_send_state.state.entry(channel_index).or_default();
        entry.1 = Some(pan);

        let mut changed = HashMap::new();
        // Stereo-linked pairs drive pan via the Console 1 hybrid knob -- ignore MS echoes.
        if slot.pan_locked && slot.ms_channels.len() == 2 {
            return SendsModeInputUpdateOutcome::Applied { changed };
        }
        if track.pan != pan {
            track.pan = pan;
            changed.insert("pan".to_string(), json!(pan));
        }
        return SendsModeInputUpdateOutcome::Applied { changed };
    }

    SendsModeInputUpdateOutcome::NotHandled
}

/// Mirror `volume` into all 6 Console 1 send slots (forcing each `sendNOn` true) -- proxies
/// the fader display for Bus/Main tracks, which must stay "standard" even in sends mode
/// (Console 1 reads fader state from send slots while sends mode is active). Returns only
/// the fields that changed.
pub fn mirror_console1_send_slots_from_volume(
    track: &mut TrackInfo,
    volume: &Value,
) -> HashMap<String, Value> {
    let mut changed = HashMap::new();
    for i in 0..NUMBER_OF_SENDS {
        if !track.send_on[i] {
            track.send_on[i] = true;
            changed.insert(format!("send{}On", i + 1), json!(true));
        }
        if &track.send_levels[i] != volume {
            track.send_levels[i] = volume.clone();
            changed.insert(format!("send{}", i + 1), volume.clone());
        }
    }
    changed
}

/// Clear all 6 Console 1 send slots (used when leaving sends mode for Bus/Main). Returns
/// only the fields that changed.
pub fn clear_console1_send_slots(track: &mut TrackInfo) -> HashMap<String, Value> {
    let mut changed = HashMap::new();
    for i in 0..NUMBER_OF_SENDS {
        if track.send_on[i] {
            track.send_on[i] = false;
            changed.insert(format!("send{}On", i + 1), json!(false));
        }
        if track.send_levels[i] != json!(0) {
            track.send_levels[i] = json!(0);
            changed.insert(format!("send{}", i + 1), json!(0));
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::Lifecycle;
    use crate::send_mapping::build_send_mapping;
    use crate::track_cache::{create_default_track_for_slot, DefaultTrackColors};
    use crate::track_layout::{LayoutSlot, LayoutSlotKind};
    use serde_json::json;

    fn bus_slot(object_id: usize, ms_primary: usize) -> LayoutSlot {
        LayoutSlot {
            object_id,
            kind: LayoutSlotKind::Bus,
            ms_channels: vec![ms_primary],
            ms_primary: Some(ms_primary),
            pan_locked: false,
        }
    }

    fn main_slot(object_id: usize) -> LayoutSlot {
        LayoutSlot {
            object_id,
            kind: LayoutSlotKind::Main,
            ms_channels: vec![70, 71],
            ms_primary: Some(70),
            pan_locked: true,
        }
    }

    fn input_slot(object_id: usize, ms_primary: usize) -> LayoutSlot {
        LayoutSlot {
            object_id,
            kind: LayoutSlotKind::Input,
            ms_channels: vec![ms_primary],
            ms_primary: Some(ms_primary),
            pan_locked: false,
        }
    }

    fn default_colors() -> DefaultTrackColors {
        DefaultTrackColors {
            bus_color: 0x00a5ff,
            main_color: 0x00a5ff,
        }
    }

    fn default_track(slot: &LayoutSlot) -> crate::track_cache::TrackInfo {
        create_default_track_for_slot(
            slot,
            "trk-1".to_string(),
            Lifecycle::Running,
            &default_colors(),
            48,
        )
    }

    #[test]
    fn selecting_a_bus_latches_sends_mode_to_its_ms_send_index() {
        let slot = bus_slot(20, 50); // ms_primary=50, bus_channel_start=48 -> index 2
        let result = maybe_latch_sends_mode_from_selection(&slot, "selected", &json!(true), 48);
        assert_eq!(result, Some(Some(2)));
    }

    #[test]
    fn selecting_main_clears_sends_mode() {
        let slot = main_slot(69);
        let result = maybe_latch_sends_mode_from_selection(&slot, "selected", &json!(true), 48);
        assert_eq!(result, Some(None));
    }

    #[test]
    fn selecting_an_input_is_a_no_op() {
        let slot = input_slot(0, 0);
        let result = maybe_latch_sends_mode_from_selection(&slot, "selected", &json!(true), 48);
        assert_eq!(result, None);
    }

    #[test]
    fn non_selected_param_path_is_a_no_op() {
        let slot = bus_slot(20, 50);
        let result = maybe_latch_sends_mode_from_selection(&slot, "mix.lvl", &json!(true), 48);
        assert_eq!(result, None);
    }

    #[test]
    fn deselecting_a_bus_is_a_no_op() {
        let slot = bus_slot(20, 50);
        let result = maybe_latch_sends_mode_from_selection(&slot, "selected", &json!(false), 48);
        assert_eq!(result, None);
    }

    #[test]
    fn sends_mode_input_update_suppresses_standard_mute_and_pan_while_active() {
        let mut track = default_track(&input_slot(0, 0));
        let slot = input_slot(0, 0);
        let mapping = build_send_mapping(&[], 16);
        let mut input_send_state = InputSendState::new();

        let result = handle_sends_mode_input_update(SendsModeInputUpdateArgs {
            slot: &slot,
            channel_index: 0,
            param_path: "mix.on",
            value: &json!(true),
            sends_mode_ms_send_index: Some(2),
            mapping: &mapping,
            track: &mut track,
            input_send_state: &mut input_send_state,
        });
        assert_eq!(result, SendsModeInputUpdateOutcome::Suppressed);
    }

    #[test]
    fn sends_mode_input_update_not_handled_when_sends_mode_inactive() {
        let mut track = default_track(&input_slot(0, 0));
        let slot = input_slot(0, 0);
        let mapping = build_send_mapping(&[], 16);
        let mut input_send_state = InputSendState::new();

        let result = handle_sends_mode_input_update(SendsModeInputUpdateArgs {
            slot: &slot,
            channel_index: 0,
            param_path: "mix.on",
            value: &json!(true),
            sends_mode_ms_send_index: None,
            mapping: &mapping,
            track: &mut track,
            input_send_state: &mut input_send_state,
        });
        assert_eq!(result, SendsModeInputUpdateOutcome::NotHandled);
    }

    #[test]
    fn sends_mode_input_update_applies_active_send_on_and_mirrors_to_mute() {
        let mut track = default_track(&input_slot(0, 0));
        // Starting state consistent with send3 being off: in sends mode `mute` is the
        // send-on/off LED (`!sendOn`), so an off send means a muted-looking track.
        track.mute = true;
        let slot = input_slot(0, 0);
        let mapping = build_send_mapping(&[], 16); // identity: ms index 2 -> c1 slot 2 -> send3On
        let mut input_send_state = InputSendState::new();

        let result = handle_sends_mode_input_update(SendsModeInputUpdateArgs {
            slot: &slot,
            channel_index: 0,
            param_path: "mix.sends.2.on",
            value: &json!(true),
            sends_mode_ms_send_index: Some(2),
            mapping: &mapping,
            track: &mut track,
            input_send_state: &mut input_send_state,
        });
        match result {
            SendsModeInputUpdateOutcome::Applied { changed } => {
                assert_eq!(changed.get("send3On"), Some(&json!(true)));
                assert_eq!(changed.get("mute"), Some(&json!(false))); // !sendOn
            }
            other => panic!("expected Applied, got {other:?}"),
        }
        assert!(track.send_on[2]);
        assert!(!track.mute);
    }

    #[test]
    fn sends_mode_input_update_applies_active_send_pan() {
        let mut track = default_track(&input_slot(0, 0));
        let slot = input_slot(0, 0);
        let mapping = build_send_mapping(&[], 16);
        let mut input_send_state = InputSendState::new();

        let result = handle_sends_mode_input_update(SendsModeInputUpdateArgs {
            slot: &slot,
            channel_index: 0,
            param_path: "mix.sends.2.pan",
            value: &json!(0.75),
            sends_mode_ms_send_index: Some(2),
            mapping: &mapping,
            track: &mut track,
            input_send_state: &mut input_send_state,
        });
        match result {
            SendsModeInputUpdateOutcome::Applied { changed } => {
                assert_eq!(changed.get("pan"), Some(&json!(0.75)));
            }
            other => panic!("expected Applied, got {other:?}"),
        }
        assert_eq!(track.pan, 0.75);
    }

    #[test]
    fn sends_mode_input_update_pan_skipped_for_stereo_linked_pair() {
        let mut track = default_track(&input_slot(0, 0));
        let mut slot = input_slot(0, 0);
        slot.pan_locked = true;
        slot.ms_channels = vec![0, 1]; // stereo-linked pair
        let mapping = build_send_mapping(&[], 16);
        let mut input_send_state = InputSendState::new();

        let result = handle_sends_mode_input_update(SendsModeInputUpdateArgs {
            slot: &slot,
            channel_index: 0,
            param_path: "mix.sends.2.pan",
            value: &json!(0.75),
            sends_mode_ms_send_index: Some(2),
            mapping: &mapping,
            track: &mut track,
            input_send_state: &mut input_send_state,
        });
        // Handled (echo consumed into input_send_state) but no Console1 pan change --
        // stereo-linked pairs drive pan via the Console 1 hybrid knob, not MS echoes.
        match result {
            SendsModeInputUpdateOutcome::Applied { changed } => {
                assert!(!changed.contains_key("pan"));
            }
            other => panic!("expected Applied with no pan change, got {other:?}"),
        }
    }

    #[test]
    fn sends_mode_input_update_other_param_path_not_handled() {
        let mut track = default_track(&input_slot(0, 0));
        let slot = input_slot(0, 0);
        let mapping = build_send_mapping(&[], 16);
        let mut input_send_state = InputSendState::new();

        let result = handle_sends_mode_input_update(SendsModeInputUpdateArgs {
            slot: &slot,
            channel_index: 0,
            param_path: "cfg.name",
            value: &json!("Vocal"),
            sends_mode_ms_send_index: Some(2),
            mapping: &mapping,
            track: &mut track,
            input_send_state: &mut input_send_state,
        });
        assert_eq!(result, SendsModeInputUpdateOutcome::NotHandled);
    }

    #[test]
    fn mirror_console1_send_slots_from_volume_sets_all_six_slots_and_forces_on() {
        let mut track = default_track(&bus_slot(20, 50));
        let changed = mirror_console1_send_slots_from_volume(&mut track, &json!(-6.0));
        for i in 1..=6 {
            assert_eq!(track.send_levels[i - 1], json!(-6.0));
            assert!(track.send_on[i - 1]);
            assert_eq!(changed.get(&format!("send{i}")), Some(&json!(-6.0)));
            assert_eq!(changed.get(&format!("send{i}On")), Some(&json!(true)));
        }
    }

    #[test]
    fn mirror_console1_send_slots_reports_no_change_when_already_mirrored() {
        let mut track = default_track(&bus_slot(20, 50));
        mirror_console1_send_slots_from_volume(&mut track, &json!(-6.0));
        let changed = mirror_console1_send_slots_from_volume(&mut track, &json!(-6.0));
        assert!(changed.is_empty());
    }

    #[test]
    fn clear_console1_send_slots_zeroes_and_turns_off_all_six() {
        let mut track = default_track(&bus_slot(20, 50));
        mirror_console1_send_slots_from_volume(&mut track, &json!(-6.0));
        let changed = clear_console1_send_slots(&mut track);
        for i in 1..=6 {
            assert_eq!(track.send_levels[i - 1], json!(0));
            assert!(!track.send_on[i - 1]);
            assert_eq!(changed.get(&format!("send{i}")), Some(&json!(0)));
            assert_eq!(changed.get(&format!("send{i}On")), Some(&json!(false)));
        }
    }

    #[test]
    fn is_sends_mode_active_reflects_option_state() {
        assert!(!is_sends_mode_active(None));
        assert!(is_sends_mode_active(Some(3)));
    }
}
