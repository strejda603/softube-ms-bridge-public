//! Console1 -> Mixing Station handlers for the "mixer" fields: volume, mute, solo, pan,
//! selected, send levels/on, and the Bus/Main sends-mode fader-display proxy. Each handler
//! mutates the given `TrackInfo` optimistically (avoiding Console 1 fader snap-back while
//! waiting for MS's own echo), returns the trackBatch-eligible fields that changed, and
//! appends zero or more `MsWrite`s to a shared output vector -- mirroring `index.js`'s
//! `writes` array being threaded through and appended to by every handler in sequence.
//!
//! Port of `index.js`'s `handleMidiVolumeUpdate`, `handleMidiMuteUpdate`,
//! `handleMidiSoloUpdate`, `handleMidiPanUpdate`, `handleMidiSelectedUpdate`,
//! `handleMidiSendSlotsUpdate`, `handleMidiBusMainSendsModeFaderProxy`.

use crate::channel_data_message::MsFormat;
use crate::pan_utils::{
    clamp01, hybrid_stereo_pan_to_dual_mono_pans, DualMonoPans, STEREO_HYBRID_NARROW_ZONE,
};
use crate::runtime::BridgeEvent;
use crate::send_mapping::SendMapping;
use crate::sends_mode::{
    maybe_latch_sends_mode_from_selection, mirror_console1_send_slots_from_volume,
};
use crate::track_cache::TrackInfo;
use crate::track_layout::{LayoutSlot, LayoutSlotKind};
use crate::value_coercion::{
    coerce_console1_numeric_string, normalize_console1_level_for_ms,
    resolve_next_boolean_from_momentary,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub struct MsWrite {
    pub path: String,
    pub value: Value,
    pub format: MsFormat,
}

fn is_bus_or_main(kind: LayoutSlotKind) -> bool {
    matches!(kind, LayoutSlotKind::Bus | LayoutSlotKind::Main)
}

pub(crate) fn push_writes_for_channels(
    writes: &mut Vec<MsWrite>,
    slot: &LayoutSlot,
    suffix: &str,
    value: Value,
    format: MsFormat,
) {
    for &ch in &slot.ms_channels {
        writes.push(MsWrite {
            path: format!("ch.{ch}.{suffix}"),
            value: value.clone(),
            format,
        });
    }
}

fn write_mix_lvl_for_slot(writes: &mut Vec<MsWrite>, slot: &LayoutSlot, console1_level: &Value) {
    let Some(v) = normalize_console1_level_for_ms(console1_level) else {
        return;
    };
    push_writes_for_channels(writes, slot, "mix.lvl", json!(v), MsFormat::Val);
}

/// Shared by `handle_midi_volume_update` and `handle_midi_bus_main_sends_mode_fader_proxy`:
/// sets `track.volume`, mirrors it into all 6 send slots, returns the merged changed-fields
/// map. Port of `buildBusMainSendsModeFaderDisplayPartial`.
fn build_bus_main_sends_mode_fader_display_partial(
    track: &mut TrackInfo,
    next_volume: Value,
) -> HashMap<String, Value> {
    track.volume = next_volume.clone();
    let mut partial = mirror_console1_send_slots_from_volume(track, &next_volume);
    partial.insert("volume".to_string(), next_volume);
    partial
}

/// Returns `{"volume": next}`, or -- for a Bus/Main slot while sends mode is active -- that
/// plus the mirrored `send1..6`/`send1..6On` fields (see
/// `build_bus_main_sends_mode_fader_display_partial`). Empty if `volume` is absent.
pub fn handle_midi_volume_update(
    parsed: &Value,
    slot: &LayoutSlot,
    track: &mut TrackInfo,
    sends_mode_ms_send_index: Option<usize>,
    writes: &mut Vec<MsWrite>,
) -> HashMap<String, Value> {
    let Some(raw_volume) = parsed.get("volume").cloned() else {
        return HashMap::new();
    };
    let next_volume = coerce_console1_numeric_string(raw_volume);

    let changed = if sends_mode_ms_send_index.is_some() && is_bus_or_main(slot.kind) {
        build_bus_main_sends_mode_fader_display_partial(track, next_volume)
    } else {
        track.volume = next_volume.clone();
        HashMap::from([("volume".to_string(), next_volume)])
    };

    write_mix_lvl_for_slot(writes, slot, &parsed["volume"]);
    changed
}

pub fn handle_midi_bus_main_sends_mode_fader_proxy(
    parsed: &Value,
    slot: &LayoutSlot,
    track: &mut TrackInfo,
    sends_mode_ms_send_index: Option<usize>,
    writes: &mut Vec<MsWrite>,
) -> HashMap<String, Value> {
    if sends_mode_ms_send_index.is_none() || !is_bus_or_main(slot.kind) {
        return HashMap::new();
    }

    let found_level_key = (1..=6usize).find_map(|i| {
        let key = format!("send{i}");
        parsed.get(&key).map(|_| key)
    });
    let saw_any_on_key = (1..=6usize).any(|i| parsed.get(format!("send{i}On")).is_some());
    if found_level_key.is_none() && !saw_any_on_key {
        return HashMap::new();
    }

    let next_volume = match &found_level_key {
        Some(key) => coerce_console1_numeric_string(parsed[key].clone()),
        None => track.volume.clone(),
    };

    let changed = build_bus_main_sends_mode_fader_display_partial(track, next_volume);

    if let Some(key) = &found_level_key {
        write_mix_lvl_for_slot(writes, slot, &parsed[key]);
    }

    changed
}

/// Returns `{"mute": next}` in standard mode, or `{"mute": led_state, "send<N>On": next_on}`
/// while sends mode is active on an Input slot -- see the sends-mode branch below for why a
/// `mute` field is still returned there. Empty if `mute` is absent, if the slot is neither
/// Input nor Bus/Main, or if the active send has no Console 1 slot mapped to it.
pub fn handle_midi_mute_update(
    parsed: &Value,
    slot: &LayoutSlot,
    track: &mut TrackInfo,
    sends_mode_ms_send_index: Option<usize>,
    mapping: &SendMapping,
    writes: &mut Vec<MsWrite>,
) -> HashMap<String, Value> {
    let Some(raw_mute) = parsed.get("mute") else {
        return HashMap::new();
    };

    if slot.kind != LayoutSlotKind::Input {
        if !is_bus_or_main(slot.kind) {
            return HashMap::new();
        }
        let next_mute =
            resolve_next_boolean_from_momentary(track.mute, raw_mute.as_bool().unwrap_or(false));
        track.mute = next_mute;
        push_writes_for_channels(
            writes,
            slot,
            "mix.on",
            json!(if next_mute { 0 } else { 1 }),
            MsFormat::Val,
        );
        return HashMap::from([("mute".to_string(), json!(next_mute))]);
    }

    // Sends mode: Console 1 keeps sending plain `mute` changes, but here they mean "toggle the
    // ACTIVE send's on/off", not "mute the channel". Console 1 mute semantics are inverted
    // relative to "send on": mute=true means send off. MS's own `mix.sends.<idx>.on` keeps
    // normal semantics (1 = on).
    if let Some(ms_send_index) = sends_mode_ms_send_index {
        if let Some(&c1_slot) = mapping.ms_send_index_to_c1_slot.get(&ms_send_index) {
            let current_on = track.send_on[c1_slot];
            let current_muted = !current_on;
            let incoming_muted = raw_mute.as_bool().unwrap_or(false);
            let next_muted = resolve_next_boolean_from_momentary(current_muted, incoming_muted);
            let next_on = !next_muted;

            // Optimistically update Console 1 immediately to avoid snap-back while waiting for MS.
            //
            // `track.mute` here is NOT real mute state: while sends mode is active it is
            // deliberately repurposed as the Mute *button LED* state, derived as `!send_on`, so
            // the Console 1 UI stays consistent with the send it is now controlling. Console 1
            // firmware needs that `mute` field present alongside `send<N>On` for the LED to
            // render at all, which is why both are returned as changed fields below.
            track.send_on[c1_slot] = next_on;
            track.mute = next_muted;

            push_writes_for_channels(
                writes,
                slot,
                &format!("mix.sends.{ms_send_index}.on"),
                json!(if next_on { 1 } else { 0 }),
                MsFormat::Val,
            );

            let field = format!("send{}On", c1_slot + 1);
            return HashMap::from([
                ("mute".to_string(), json!(next_muted)),
                (field, json!(next_on)),
            ]);
        }

        // The active send isn't one of Console 1's 6 physical slots (MS supports up to 16 bus
        // sends), so there's no valid `send<N>On` field to reflect on Console 1 and no Mute LED
        // to derive -- `track` is left untouched and no changed fields are returned. The MS-side
        // toggle still goes out, so the send itself responds.
        let incoming_mute_fallback = raw_mute.as_bool().unwrap_or(false);
        let next_mute_fallback =
            resolve_next_boolean_from_momentary(track.mute, incoming_mute_fallback);
        push_writes_for_channels(
            writes,
            slot,
            &format!("mix.sends.{ms_send_index}.on"),
            json!(if next_mute_fallback { 0 } else { 1 }),
            MsFormat::Val,
        );
        return HashMap::new();
    }

    // Standard mode.
    let next_mute =
        resolve_next_boolean_from_momentary(track.mute, raw_mute.as_bool().unwrap_or(false));
    track.mute = next_mute;
    push_writes_for_channels(
        writes,
        slot,
        "mix.on",
        json!(if next_mute { 0 } else { 1 }),
        MsFormat::Val,
    );
    HashMap::from([("mute".to_string(), json!(next_mute))])
}

/// Returns `{"solo": next}`. Empty if `solo` is absent, for non-Input/non-Bus/Main slots, or
/// for Bus slots specifically -- bus masters ignore solo from both directions.
pub fn handle_midi_solo_update(
    parsed: &Value,
    slot: &LayoutSlot,
    track: &mut TrackInfo,
    writes: &mut Vec<MsWrite>,
) -> HashMap<String, Value> {
    let Some(raw_solo) = parsed.get("solo") else {
        return HashMap::new();
    };
    if slot.kind != LayoutSlotKind::Input && !is_bus_or_main(slot.kind) {
        return HashMap::new();
    }
    if slot.kind == LayoutSlotKind::Bus {
        return HashMap::new();
    }

    let next_solo =
        resolve_next_boolean_from_momentary(track.solo, raw_solo.as_bool().unwrap_or(false));
    track.solo = next_solo;
    push_writes_for_channels(
        writes,
        slot,
        "solo",
        json!(if next_solo { 1 } else { 0 }),
        MsFormat::Val,
    );
    HashMap::from([("solo".to_string(), json!(next_solo))])
}

/// Returns the trackBatch-eligible fields that changed. Note the standard-mode mono-Input
/// branch deliberately returns an empty map and leaves `track.pan` untouched, matching
/// `index.js:3789-3792`: only the lossy hybrid stereo conversion (which can't round-trip
/// through Mixing Station's own echo) needs the optimistic local cache write.
pub fn handle_midi_pan_update(
    parsed: &Value,
    slot: &LayoutSlot,
    track: &mut TrackInfo,
    primary_channel: usize,
    sends_mode_ms_send_index: Option<usize>,
    // Unused: pan addresses the active send by its MS index directly and never needs to resolve
    // a Console 1 slot. Kept for call-shape symmetry with the sibling handlers.
    _mapping: &SendMapping,
    writes: &mut Vec<MsWrite>,
    log_json: bool,
    event_tx: &mpsc::UnboundedSender<BridgeEvent>,
) -> HashMap<String, Value> {
    let Some(raw_pan) = parsed.get("pan") else {
        return HashMap::new();
    };
    let knob_pan = clamp01(raw_pan.as_f64().unwrap_or(0.5));
    let is_stereo_linked_pair = slot.pan_locked && slot.ms_channels.len() == 2;

    if slot.kind != LayoutSlotKind::Input {
        if !is_bus_or_main(slot.kind) {
            return HashMap::new();
        }
        track.pan = knob_pan;
        let changed = HashMap::from([("pan".to_string(), json!(knob_pan))]);
        if is_stereo_linked_pair {
            let _ = push_dual_mono_pan_writes(writes, slot, knob_pan, "mix.pan");
        } else {
            writes.push(MsWrite {
                path: format!("ch.{primary_channel}.mix.pan"),
                value: json!(knob_pan),
                format: MsFormat::Norm,
            });
        }
        return changed;
    }

    if let Some(ms_send_index) = sends_mode_ms_send_index {
        track.pan = knob_pan;
        let changed = HashMap::from([("pan".to_string(), json!(knob_pan))]);
        let suffix = format!("mix.sends.{ms_send_index}.pan");
        if is_stereo_linked_pair {
            let dual = push_dual_mono_pan_writes(writes, slot, knob_pan, &suffix);
            if log_json {
                let (left_ch, right_ch) = (slot.ms_channels[0], slot.ms_channels[1]);
                let _ = event_tx.send(BridgeEvent::Log(format!(
                    "[hybrid-pan] sends trackId={} msChannels=[{left_ch},{right_ch}] msSendIndex={ms_send_index} knobPan={knob_pan} left={} right={} width={} mid={}",
                    track.track_id, dual.left, dual.right, dual.width, dual.mid
                )));
            }
        } else {
            push_writes_for_channels(writes, slot, &suffix, json!(knob_pan), MsFormat::Norm);
        }
        return changed;
    }

    // Standard mode, Input slot.
    if is_stereo_linked_pair {
        track.pan = knob_pan;
        let dual = push_dual_mono_pan_writes(writes, slot, knob_pan, "mix.pan");
        if log_json {
            let (left_ch, right_ch) = (slot.ms_channels[0], slot.ms_channels[1]);
            let _ = event_tx.send(BridgeEvent::Log(format!(
                "[hybrid-pan] standard trackId={} msChannels=[{left_ch},{right_ch}] knobPan={knob_pan} left={} right={} width={} mid={}",
                track.track_id, dual.left, dual.right, dual.width, dual.mid
            )));
        }
        HashMap::from([("pan".to_string(), json!(knob_pan))])
    } else {
        writes.push(MsWrite {
            path: format!("ch.{primary_channel}.mix.pan"),
            value: json!(knob_pan),
            format: MsFormat::Norm,
        });
        HashMap::new()
    }
}

/// Split one Console 1 pan knob position into the stereo pair's two dual-mono pan writes.
fn push_dual_mono_pan_writes(
    writes: &mut Vec<MsWrite>,
    slot: &LayoutSlot,
    knob_pan: f64,
    suffix: &str,
) -> DualMonoPans {
    let dual = hybrid_stereo_pan_to_dual_mono_pans(knob_pan, STEREO_HYBRID_NARROW_ZONE);
    let (left_ch, right_ch) = (slot.ms_channels[0], slot.ms_channels[1]);
    writes.push(MsWrite {
        path: format!("ch.{left_ch}.{suffix}"),
        value: json!(dual.left),
        format: MsFormat::Norm,
    });
    writes.push(MsWrite {
        path: format!("ch.{right_ch}.{suffix}"),
        value: json!(dual.right),
        format: MsFormat::Norm,
    });
    dual
}

/// Returns the sends-mode latch decision (see `sends_mode::maybe_latch_sends_mode_from_selection`)
/// for the caller to apply via `bridge-cli`'s `set_sends_mode`.
pub fn handle_midi_selected_update(
    parsed: &Value,
    slot: &LayoutSlot,
    primary_channel: usize,
    bus_channel_start: usize,
    writes: &mut Vec<MsWrite>,
) -> Option<Option<usize>> {
    let selected = parsed.get("selected")?.as_bool().unwrap_or(false);

    writes.push(MsWrite {
        path: format!("ch.{primary_channel}.selected"),
        value: json!(if selected { 1 } else { 0 }),
        format: MsFormat::Val,
    });

    if selected {
        maybe_latch_sends_mode_from_selection(slot, "selected", &json!(true), bus_channel_start)
    } else {
        None
    }
}

/// Returns the `send1..6` level fields that changed. Deliberately returns nothing for the
/// `send<N>On` keys and leaves `track.send_on` untouched: Console 1 already holds the
/// authoritative on/off state for its own button, so this handler only propagates it to MS.
pub fn handle_midi_send_slots_update(
    parsed: &Value,
    slot: &LayoutSlot,
    track: &mut TrackInfo,
    mapping: &SendMapping,
    writes: &mut Vec<MsWrite>,
) -> HashMap<String, Value> {
    if slot.kind != LayoutSlotKind::Input {
        return HashMap::new();
    }

    let mut changed = HashMap::new();
    for i in 1..=6usize {
        let ms_send_index = mapping.c1_to_ms_send_index[i - 1];
        let lvl_key = format!("send{i}");
        let on_key = format!("send{i}On");

        if let Some(raw_lvl) = parsed.get(&lvl_key).cloned() {
            let next = coerce_console1_numeric_string(raw_lvl);
            track.send_levels[i - 1] = next.clone();
            changed.insert(lvl_key.clone(), next);

            if let Some(v) = normalize_console1_level_for_ms(&parsed[&lvl_key]) {
                push_writes_for_channels(
                    writes,
                    slot,
                    &format!("mix.sends.{ms_send_index}.lvl"),
                    json!(v),
                    MsFormat::Val,
                );
            }
        }

        if let Some(raw_on) = parsed.get(&on_key) {
            let on = raw_on.as_bool().unwrap_or(false);
            push_writes_for_channels(
                writes,
                slot,
                &format!("mix.sends.{ms_send_index}.on"),
                json!(if on { 1 } else { 0 }),
                MsFormat::Val,
            );
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel_data_message::MsFormat;
    use crate::lifecycle::Lifecycle;
    use crate::send_mapping::build_send_mapping;
    use crate::track_cache::{create_default_track_for_slot, DefaultTrackColors};
    use crate::track_layout::{LayoutSlot, LayoutSlotKind};
    use serde_json::json;

    fn discarding_event_tx() -> mpsc::UnboundedSender<BridgeEvent> {
        mpsc::unbounded_channel().0
    }

    fn input_slot() -> LayoutSlot {
        LayoutSlot {
            object_id: 10,
            kind: LayoutSlotKind::Input,
            ms_channels: vec![0],
            ms_primary: Some(0),
            pan_locked: false,
        }
    }

    fn stereo_input_slot() -> LayoutSlot {
        LayoutSlot {
            object_id: 10,
            kind: LayoutSlotKind::Input,
            ms_channels: vec![0, 1],
            ms_primary: Some(0),
            pan_locked: true,
        }
    }

    fn bus_slot() -> LayoutSlot {
        LayoutSlot {
            object_id: 20,
            kind: LayoutSlotKind::Bus,
            ms_channels: vec![50],
            ms_primary: Some(50),
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
    fn volume_update_sets_track_and_writes_mix_lvl() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let changed = handle_midi_volume_update(
            &json!({"volume": -6.0}),
            &input_slot(),
            &mut track,
            None,
            &mut writes,
        );
        assert_eq!(track.volume, json!(-6.0));
        assert_eq!(changed.get("volume"), Some(&json!(-6.0)));
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].path, "ch.0.mix.lvl");
        assert_eq!(writes[0].value, json!(-6.0));
        assert_eq!(writes[0].format, MsFormat::Val);
    }

    #[test]
    fn volume_update_absent_field_is_a_no_op() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let changed =
            handle_midi_volume_update(&json!({}), &input_slot(), &mut track, None, &mut writes);
        assert!(changed.is_empty());
        assert!(writes.is_empty());
    }

    #[test]
    fn volume_update_stereo_pair_writes_both_channels() {
        let mut track = default_track(&stereo_input_slot());
        let mut writes = Vec::new();
        handle_midi_volume_update(
            &json!({"volume": -3.0}),
            &stereo_input_slot(),
            &mut track,
            None,
            &mut writes,
        );
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0].path, "ch.0.mix.lvl");
        assert_eq!(writes[1].path, "ch.1.mix.lvl");
    }

    #[test]
    fn volume_update_bus_main_in_sends_mode_mirrors_send_slots() {
        let mut track = default_track(&bus_slot());
        let mut writes = Vec::new();
        let changed = handle_midi_volume_update(
            &json!({"volume": -6.0}),
            &bus_slot(),
            &mut track,
            Some(2),
            &mut writes,
        );
        assert_eq!(track.volume, json!(-6.0));
        for i in 1..=6 {
            assert!(track.send_on[i - 1]);
            assert_eq!(track.send_levels[i - 1], json!(-6.0));
        }
        assert_eq!(changed.get("volume"), Some(&json!(-6.0)));
        assert_eq!(changed.get("send3"), Some(&json!(-6.0))); // arbitrary slot check
        assert_eq!(writes.len(), 1); // still writes mix.lvl normally
    }

    #[test]
    fn bus_main_sends_mode_fader_proxy_no_op_when_not_sends_mode() {
        let mut track = default_track(&bus_slot());
        let mut writes = Vec::new();
        let changed = handle_midi_bus_main_sends_mode_fader_proxy(
            &json!({"send3": -6.0}),
            &bus_slot(),
            &mut track,
            None,
            &mut writes,
        );
        assert!(changed.is_empty());
        assert!(writes.is_empty());
    }

    #[test]
    fn bus_main_sends_mode_fader_proxy_no_op_for_input_slot() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let changed = handle_midi_bus_main_sends_mode_fader_proxy(
            &json!({"send3": -6.0}),
            &input_slot(),
            &mut track,
            Some(2),
            &mut writes,
        );
        assert!(changed.is_empty());
        assert!(writes.is_empty());
    }

    #[test]
    fn bus_main_sends_mode_fader_proxy_reacts_to_a_send_level_key() {
        let mut track = default_track(&bus_slot());
        let mut writes = Vec::new();
        let changed = handle_midi_bus_main_sends_mode_fader_proxy(
            &json!({"send3": -6.0}),
            &bus_slot(),
            &mut track,
            Some(2),
            &mut writes,
        );
        assert_eq!(track.volume, json!(-6.0));
        assert_eq!(changed.get("volume"), Some(&json!(-6.0)));
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].path, "ch.50.mix.lvl");
    }

    #[test]
    fn bus_main_sends_mode_fader_proxy_reacts_to_an_on_key_with_no_level_write() {
        let mut track = default_track(&bus_slot());
        let mut writes = Vec::new();
        let changed = handle_midi_bus_main_sends_mode_fader_proxy(
            &json!({"send3On": true}),
            &bus_slot(),
            &mut track,
            Some(2),
            &mut writes,
        );
        // Still mirrors (using existing track.volume), but no mix.lvl write since no level key was seen.
        assert!(changed.contains_key("volume"));
        assert!(writes.is_empty());
    }

    #[test]
    fn mute_update_absent_field_is_a_no_op() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let mapping = build_send_mapping(&[], 16);
        let changed = handle_midi_mute_update(
            &json!({}),
            &input_slot(),
            &mut track,
            None,
            &mapping,
            &mut writes,
        );
        assert!(changed.is_empty());
        assert!(writes.is_empty());
    }

    #[test]
    fn mute_update_bus_toggles_and_writes_mix_on() {
        let mut track = default_track(&bus_slot());
        let mut writes = Vec::new();
        let mapping = build_send_mapping(&[], 16);
        let changed = handle_midi_mute_update(
            &json!({"mute": true}),
            &bus_slot(),
            &mut track,
            None,
            &mapping,
            &mut writes,
        );
        assert!(track.mute);
        assert_eq!(changed.get("mute"), Some(&json!(true)));
        assert_eq!(writes[0].path, "ch.50.mix.on");
        assert_eq!(writes[0].value, json!(0)); // muted -> mix.on = 0
    }

    #[test]
    fn mute_update_bus_momentary_toggle_when_same_as_current() {
        let mut track = default_track(&bus_slot());
        track.mute = false;
        let mut writes = Vec::new();
        let mapping = build_send_mapping(&[], 16);
        // incoming == current (both false) -> toggles to true.
        let changed = handle_midi_mute_update(
            &json!({"mute": false}),
            &bus_slot(),
            &mut track,
            None,
            &mapping,
            &mut writes,
        );
        assert!(track.mute);
        assert_eq!(changed.get("mute"), Some(&json!(true)));
    }

    #[test]
    fn mute_update_input_standard_mode_writes_mix_on() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let mapping = build_send_mapping(&[], 16);
        handle_midi_mute_update(
            &json!({"mute": true}),
            &input_slot(),
            &mut track,
            None,
            &mapping,
            &mut writes,
        );
        assert!(track.mute);
        assert_eq!(writes[0].path, "ch.0.mix.on");
        assert_eq!(writes[0].value, json!(0));
    }

    #[test]
    fn mute_update_input_sends_mode_toggles_send_on_and_mirrors_mute_led() {
        let mut track = default_track(&input_slot());
        track.send_on[2] = true; // send3On currently on -> not muted
        let mut writes = Vec::new();
        let mapping = build_send_mapping(&[], 16); // identity: ms index 2 -> c1 slot 2 -> send3On
        let changed = handle_midi_mute_update(
            &json!({"mute": true}),
            &input_slot(),
            &mut track,
            Some(2),
            &mapping,
            &mut writes,
        );
        // currentMuted = !true = false; incomingMuted = true; toggle -> nextMuted = true -> nextOn = false
        assert!(!track.send_on[2]);
        assert!(track.mute);
        assert_eq!(changed.get("send3On"), Some(&json!(false)));
        assert_eq!(changed.get("mute"), Some(&json!(true)));
        assert_eq!(writes[0].path, "ch.0.mix.sends.2.on");
        assert_eq!(writes[0].value, json!(0));
    }

    #[test]
    fn mute_update_input_sends_mode_unmapped_send_writes_ms_but_reflects_nothing() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        // Console 1's 6 slots cover MS send indices 0..=5; MS supports up to 16 bus sends, so
        // index 10 is reachable but has no Console 1 slot mapped to it.
        let mapping = build_send_mapping(&[1, 2, 3, 4, 5, 6], 16);
        assert!(!mapping.ms_send_index_to_c1_slot.contains_key(&10));

        let changed = handle_midi_mute_update(
            &json!({"mute": true}),
            &input_slot(),
            &mut track,
            Some(10),
            &mapping,
            &mut writes,
        );

        // The MS-side toggle still goes out, using the fallback momentary-toggle on track.mute:
        // current=false, incoming=true -> next_muted=true -> send off.
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].path, "ch.0.mix.sends.10.on");
        assert_eq!(writes[0].value, json!(0));
        assert_eq!(writes[0].format, MsFormat::Val);

        // Nothing to reflect on Console 1: no valid `send<N>On` field for this send index, so
        // the cache is left untouched and no changed fields are returned.
        assert!(changed.is_empty());
        assert!(!track.mute);
        assert_eq!(track.send_on, [false; 6]);
    }

    #[test]
    fn solo_update_absent_field_is_a_no_op() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let changed = handle_midi_solo_update(&json!({}), &input_slot(), &mut track, &mut writes);
        assert!(changed.is_empty());
    }

    #[test]
    fn solo_update_bus_slot_is_a_no_op() {
        let mut track = default_track(&bus_slot());
        let mut writes = Vec::new();
        let changed =
            handle_midi_solo_update(&json!({"solo": true}), &bus_slot(), &mut track, &mut writes);
        assert!(changed.is_empty());
        assert!(writes.is_empty());
    }

    #[test]
    fn solo_update_input_toggles_and_writes() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let changed = handle_midi_solo_update(
            &json!({"solo": true}),
            &input_slot(),
            &mut track,
            &mut writes,
        );
        assert!(track.solo);
        assert_eq!(changed.get("solo"), Some(&json!(true)));
        assert_eq!(writes[0].path, "ch.0.solo");
        assert_eq!(writes[0].value, json!(1));
    }

    #[test]
    fn pan_update_absent_field_is_a_no_op() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let mapping = build_send_mapping(&[], 16);
        let changed = handle_midi_pan_update(
            &json!({}),
            &input_slot(),
            &mut track,
            0,
            None,
            &mapping,
            &mut writes,
            false,
            &discarding_event_tx(),
        );
        assert!(changed.is_empty());
    }

    #[test]
    fn pan_update_mono_input_standard_mode_writes_single_channel_norm() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let mapping = build_send_mapping(&[], 16);
        let changed = handle_midi_pan_update(
            &json!({"pan": 0.75}),
            &input_slot(),
            &mut track,
            0,
            None,
            &mapping,
            &mut writes,
            false,
            &discarding_event_tx(),
        );
        // `index.js:3789-3792`: the standard-mode mono branch writes to MS only -- it neither
        // caches the knob position nor echoes it back to Console 1, because a mono `mix.pan`
        // round-trips through MS's own echo unchanged. Only the lossy hybrid stereo conversion
        // (below) needs the optimistic local write.
        assert_eq!(track.pan, 0.5);
        assert!(changed.is_empty());
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].path, "ch.0.mix.pan");
        assert_eq!(writes[0].value, json!(0.75));
        assert_eq!(writes[0].format, MsFormat::Norm);
    }

    #[test]
    fn pan_update_stereo_pair_writes_dual_mono() {
        let mut track = default_track(&stereo_input_slot());
        let mut writes = Vec::new();
        let mapping = build_send_mapping(&[], 16);
        handle_midi_pan_update(
            &json!({"pan": 0.5}),
            &stereo_input_slot(),
            &mut track,
            0,
            None,
            &mapping,
            &mut writes,
            false,
            &discarding_event_tx(),
        );
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0].path, "ch.0.mix.pan");
        assert_eq!(writes[1].path, "ch.1.mix.pan");
    }

    #[test]
    fn handle_midi_pan_update_logs_hybrid_pan_for_standard_mode_stereo_pair_when_log_json_set() {
        let mut track = default_track(&stereo_input_slot());
        let mut writes = Vec::new();
        let mapping = build_send_mapping(&[], 16);
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<BridgeEvent>();

        handle_midi_pan_update(
            &json!({"pan": 0.75}),
            &stereo_input_slot(),
            &mut track,
            0,
            None,
            &mapping,
            &mut writes,
            true,
            &event_tx,
        );

        let event = event_rx.try_recv().expect("expected a hybrid-pan log line");
        match event {
            BridgeEvent::Log(line) => assert!(line.starts_with("[hybrid-pan] standard")),
            other => panic!("expected BridgeEvent::Log, got {other:?}"),
        }
    }

    #[test]
    fn pan_update_input_sends_mode_writes_send_pan_instead_of_mix_pan() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let mapping = build_send_mapping(&[], 16);
        handle_midi_pan_update(
            &json!({"pan": 0.75}),
            &input_slot(),
            &mut track,
            0,
            Some(2),
            &mapping,
            &mut writes,
            false,
            &discarding_event_tx(),
        );
        assert_eq!(writes[0].path, "ch.0.mix.sends.2.pan");
    }

    #[test]
    fn selected_update_writes_selected_field() {
        let mut writes = Vec::new();
        let latch = handle_midi_selected_update(
            &json!({"selected": false}),
            &input_slot(),
            0,
            48,
            &mut writes,
        );
        assert_eq!(writes[0].path, "ch.0.selected");
        assert_eq!(writes[0].value, json!(0));
        assert_eq!(latch, None); // false selection never latches sends mode
    }

    #[test]
    fn selected_update_selecting_bus_latches_sends_mode() {
        let mut writes = Vec::new();
        let latch = handle_midi_selected_update(
            &json!({"selected": true}),
            &bus_slot(),
            50,
            48,
            &mut writes,
        );
        assert_eq!(writes[0].value, json!(1));
        assert_eq!(latch, Some(Some(2))); // ms_primary=50, bus_channel_start=48 -> index 2
    }

    #[test]
    fn selected_update_selecting_main_clears_sends_mode() {
        let main_slot = LayoutSlot {
            object_id: 69,
            kind: LayoutSlotKind::Main,
            ms_channels: vec![70, 71],
            ms_primary: Some(70),
            pan_locked: true,
        };
        let mut writes = Vec::new();
        let latch = handle_midi_selected_update(
            &json!({"selected": true}),
            &main_slot,
            70,
            48,
            &mut writes,
        );
        assert_eq!(latch, Some(None));
    }

    #[test]
    fn send_slots_update_non_input_is_a_no_op() {
        let mut track = default_track(&bus_slot());
        let mut writes = Vec::new();
        let mapping = build_send_mapping(&[], 16);
        let changed = handle_midi_send_slots_update(
            &json!({"send1": -6.0}),
            &bus_slot(),
            &mut track,
            &mapping,
            &mut writes,
        );
        assert!(changed.is_empty());
        assert!(writes.is_empty());
    }

    #[test]
    fn send_slots_update_level_key_updates_cache_and_writes() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let mapping = build_send_mapping(&[], 16);
        let changed = handle_midi_send_slots_update(
            &json!({"send3": -6.0}),
            &input_slot(),
            &mut track,
            &mapping,
            &mut writes,
        );
        assert_eq!(track.send_levels[2], json!(-6.0));
        assert_eq!(changed.get("send3"), Some(&json!(-6.0)));
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].path, "ch.0.mix.sends.2.lvl");
    }

    #[test]
    fn send_slots_update_on_key_writes_but_does_not_update_cache_or_changed() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let mapping = build_send_mapping(&[], 16);
        let changed = handle_midi_send_slots_update(
            &json!({"send3On": true}),
            &input_slot(),
            &mut track,
            &mapping,
            &mut writes,
        );
        assert!(changed.is_empty()); // matches JS: no track/queue update for the on-key
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].path, "ch.0.mix.sends.2.on");
        assert_eq!(writes[0].value, json!(1));
    }
}
