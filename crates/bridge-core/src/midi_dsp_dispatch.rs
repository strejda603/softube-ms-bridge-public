//! Console1 -> Mixing Station handlers for the Filter/EQ/Compressor DSP sections. All
//! three share a shape: unwrap a `{value: ...}`-wrapped Console1 field via
//! `read_c1_dsp_value`, clamp/convert, write into `track.dsp_fields`, return a bare-field
//! update entry (never trackBatch-eligible -- see this repo's own SysEx-allowlist
//! constraint), and push `MsWrite`s.
//!
//! Port of `index.js`'s `handleMidiFilterUpdate`, `handleMidiEqUpdate`, `handleMidiCompUpdate`.

use crate::channel_data_message::MsFormat;
use crate::dsp_field_metadata::{eq_type_normalized_to_index, headamp_gain_normalized_to_db};
use crate::midi_mixer_dispatch::{push_writes_for_channels, MsWrite};
use crate::pan_utils::clamp01;
use crate::track_cache::TrackInfo;
use crate::track_layout::{LayoutSlot, LayoutSlotKind};
use crate::value_coercion::read_c1_dsp_value;
use serde_json::{json, Value};

fn is_input_or_bus_or_main(kind: LayoutSlotKind) -> bool {
    matches!(
        kind,
        LayoutSlotKind::Input | LayoutSlotKind::Bus | LayoutSlotKind::Main
    )
}

/// `compOn` is deliberately absent: it needs a boolean 0/1 `val` write, not a clamped
/// `norm` one, so it's handled separately above this table. `ms_param_apply::DYN_TO_C1`
/// is the MS->C1 mirror of this table and DOES carry an `("on", "compOn")` entry; the
/// two are kept in sync by `comp_continuous_field_map_is_the_inverse_of_dyn_to_c1`.
const COMP_CONTINUOUS_FIELD_MAP: &[(&str, &str)] = &[
    ("compRatio", "ratio"),
    ("compAttack", "attack"),
    ("compRelease", "release"),
    ("compMakeup", "gain"),
    ("compComp", "thr"),
    ("compKnee", "knee"),
    ("compWetdry", "mix"),
];

/// Handle Filter (low-cut, input gain, phase-invert) updates from Console 1.
///
/// Console 1's Filter section also has a high-cut stage (`filterHcOn`/`filterHcFreq`) and a
/// slope field -- none of those have a Mixing Station destination (`preamp.filter.0` is a
/// single filter stage, not a pair; `preamp` has no slope concept), so only low-cut is
/// mapped. Input gain (`filterPreGain` -> `headamp.gain`, the real mic preamp gain in dB) and
/// phase invert (`filterPhaseInvert` -> `preamp.inv`) do have clean 1:1 destinations.
///
/// `headamp.gain` is the one write in this whole module that goes out as `Val` rather than
/// `Norm` -- no normalized form of that path exists, so the 0..1 knob value is converted to
/// real dB via `headamp_gain_normalized_to_db` first.
pub fn handle_midi_filter_update(
    parsed: &Value,
    slot: &LayoutSlot,
    track: &mut TrackInfo,
    writes: &mut Vec<MsWrite>,
) -> Vec<(String, Value)> {
    if !is_input_or_bus_or_main(slot.kind) {
        return Vec::new();
    }
    let mut bare = Vec::new();

    if let Some(lc_on) = read_c1_dsp_value(parsed.get("filterLcOn").cloned()) {
        let next = lc_on.as_bool().unwrap_or(false);
        track
            .dsp_fields
            .insert("filterLcOn".to_string(), json!(next));
        bare.push(("filterLcOn".to_string(), json!(next)));
        push_writes_for_channels(
            writes,
            slot,
            "preamp.filter.0.on",
            json!(if next { 1 } else { 0 }),
            MsFormat::Val,
        );
    }

    if let Some(lc_freq) = read_c1_dsp_value(parsed.get("filterLcFreq").cloned()) {
        let next = clamp01(lc_freq.as_f64().unwrap_or(0.0));
        track
            .dsp_fields
            .insert("filterLcFreq".to_string(), json!(next));
        bare.push(("filterLcFreq".to_string(), json!(next)));
        push_writes_for_channels(
            writes,
            slot,
            "preamp.filter.0.freq",
            json!(next),
            MsFormat::Norm,
        );
    }

    if let Some(input_gain) = read_c1_dsp_value(parsed.get("filterPreGain").cloned()) {
        let next = clamp01(input_gain.as_f64().unwrap_or(0.0));
        track
            .dsp_fields
            .insert("filterPreGain".to_string(), json!(next));
        bare.push(("filterPreGain".to_string(), json!(next)));
        push_writes_for_channels(
            writes,
            slot,
            "headamp.gain",
            json!(headamp_gain_normalized_to_db(next)),
            MsFormat::Val,
        );
    }

    if let Some(phase_invert) = read_c1_dsp_value(parsed.get("filterPhaseInvert").cloned()) {
        let next = phase_invert.as_bool().unwrap_or(false);
        track
            .dsp_fields
            .insert("filterPhaseInvert".to_string(), json!(next));
        bare.push(("filterPhaseInvert".to_string(), json!(next)));
        push_writes_for_channels(
            writes,
            slot,
            "preamp.inv",
            json!(if next { 1 } else { 0 }),
            MsFormat::Val,
        );
    }

    bare
}

/// Handle 4-band parametric EQ updates from Console 1 (`eq1..eq4` x `On/Freq/Gain/Q/Type`).
///
/// `eq{N}On` has no per-band Mixing Station destination -- MS exposes a single global
/// `peq.on`, not one switch per band -- so toggling ANY band's On button drives that one
/// shared path. The inbound side (`ms_param_apply`'s `peq.on` case) then fans the resulting
/// state back out to all 4 bands, keeping them in lockstep with each other and with MS.
///
/// `eq{N}Type` is a discrete step index, not a continuous 0..1 value, so it is converted via
/// `eq_type_normalized_to_index` and written as a raw integer rather than clamped. Console 1
/// still sends it normalized (the Cubase generic-remote convention applies to every
/// parameter, discrete or not).
pub fn handle_midi_eq_update(
    parsed: &Value,
    slot: &LayoutSlot,
    track: &mut TrackInfo,
    writes: &mut Vec<MsWrite>,
) -> Vec<(String, Value)> {
    if !is_input_or_bus_or_main(slot.kind) {
        return Vec::new();
    }
    let mut bare = Vec::new();

    for n in 1..=4usize {
        let band_index = n - 1;

        if let Some(on) = read_c1_dsp_value(parsed.get(format!("eq{n}On")).cloned()) {
            let next = on.as_bool().unwrap_or(false);
            let key = format!("eq{n}On");
            track.dsp_fields.insert(key.clone(), json!(next));
            bare.push((key, json!(next)));
            push_writes_for_channels(
                writes,
                slot,
                "peq.on",
                json!(if next { 1 } else { 0 }),
                MsFormat::Val,
            );
        }

        if let Some(freq) = read_c1_dsp_value(parsed.get(format!("eq{n}Freq")).cloned()) {
            let next = clamp01(freq.as_f64().unwrap_or(0.0));
            let key = format!("eq{n}Freq");
            track.dsp_fields.insert(key.clone(), json!(next));
            bare.push((key, json!(next)));
            push_writes_for_channels(
                writes,
                slot,
                &format!("peq.bands.{band_index}.freq"),
                json!(next),
                MsFormat::Norm,
            );
        }

        if let Some(gain) = read_c1_dsp_value(parsed.get(format!("eq{n}Gain")).cloned()) {
            let next = clamp01(gain.as_f64().unwrap_or(0.0));
            let key = format!("eq{n}Gain");
            track.dsp_fields.insert(key.clone(), json!(next));
            bare.push((key, json!(next)));
            push_writes_for_channels(
                writes,
                slot,
                &format!("peq.bands.{band_index}.gain"),
                json!(next),
                MsFormat::Norm,
            );
        }

        if let Some(q) = read_c1_dsp_value(parsed.get(format!("eq{n}Q")).cloned()) {
            let next = clamp01(q.as_f64().unwrap_or(0.0));
            let key = format!("eq{n}Q");
            track.dsp_fields.insert(key.clone(), json!(next));
            bare.push((key, json!(next)));
            push_writes_for_channels(
                writes,
                slot,
                &format!("peq.bands.{band_index}.q"),
                json!(next),
                MsFormat::Norm,
            );
        }

        if let Some(eq_type) = read_c1_dsp_value(parsed.get(format!("eq{n}Type")).cloned()) {
            let next = eq_type_normalized_to_index(eq_type.as_f64().unwrap_or(0.0));
            let key = format!("eq{n}Type");
            track.dsp_fields.insert(key.clone(), json!(next));
            bare.push((key, json!(next)));
            push_writes_for_channels(
                writes,
                slot,
                &format!("peq.bands.{band_index}.type"),
                json!(next),
                MsFormat::Val,
            );
        }
    }

    bare
}

/// Handle Compressor updates from Console 1 (`compOn`, plus the continuous fields in
/// `COMP_CONTINUOUS_FIELD_MAP`). `compAttackShift` has no Mixing Station analog and is not
/// handled.
///
/// **None of this handler's fields have been independently hardware-verified** (only
/// `filterLcFreq`, in `handle_midi_filter_update`, has), so their exact wire encoding is
/// inferred from Softube's Cubase driver script rather than confirmed against real Console 1
/// hardware. Treat a mismatch here as unverified-mapping-until-proven, not as a port bug.
pub fn handle_midi_comp_update(
    parsed: &Value,
    slot: &LayoutSlot,
    track: &mut TrackInfo,
    writes: &mut Vec<MsWrite>,
) -> Vec<(String, Value)> {
    if !is_input_or_bus_or_main(slot.kind) {
        return Vec::new();
    }
    let mut bare = Vec::new();

    if let Some(on) = read_c1_dsp_value(parsed.get("compOn").cloned()) {
        let next = on.as_bool().unwrap_or(false);
        track.dsp_fields.insert("compOn".to_string(), json!(next));
        bare.push(("compOn".to_string(), json!(next)));
        push_writes_for_channels(
            writes,
            slot,
            "dyn.on",
            json!(if next { 1 } else { 0 }),
            MsFormat::Val,
        );
    }

    for &(c1, ms) in COMP_CONTINUOUS_FIELD_MAP {
        if let Some(raw) = read_c1_dsp_value(parsed.get(c1).cloned()) {
            let next = clamp01(raw.as_f64().unwrap_or(0.0));
            track.dsp_fields.insert(c1.to_string(), json!(next));
            bare.push((c1.to_string(), json!(next)));
            push_writes_for_channels(
                writes,
                slot,
                &format!("dyn.{ms}"),
                json!(next),
                MsFormat::Norm,
            );
        }
    }

    bare
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel_data_message::MsFormat;
    use crate::console1_status_bank::Lifecycle;
    use crate::track_cache::{create_default_track_for_slot, DefaultTrackColors};
    use crate::track_layout::{LayoutSlot, LayoutSlotKind};
    use serde_json::json;

    fn input_slot() -> LayoutSlot {
        LayoutSlot {
            object_id: 10,
            kind: LayoutSlotKind::Input,
            ms_channels: vec![0],
            ms_primary: Some(0),
            pan_locked: false,
        }
    }

    fn main_slot() -> LayoutSlot {
        LayoutSlot {
            object_id: 69,
            kind: LayoutSlotKind::Main,
            ms_channels: vec![70, 71],
            ms_primary: Some(70),
            pan_locked: true,
        }
    }

    fn default_colors() -> DefaultTrackColors {
        DefaultTrackColors {
            bus_color: 0x00a5ff,
            main_color: 0x00a5ff,
            status_off_color: 0x333333,
            status_on_color: 0x00ff00,
            start_color: 0x00a5ff,
            stop_color: 0x999999,
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
    fn filter_update_absent_fields_is_a_no_op() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let bare = handle_midi_filter_update(&json!({}), &input_slot(), &mut track, &mut writes);
        assert!(bare.is_empty());
        assert!(writes.is_empty());
    }

    #[test]
    fn filter_update_lc_on_unwraps_value_wrapper_and_writes() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let bare = handle_midi_filter_update(
            &json!({"filterLcOn": {"value": true}}),
            &input_slot(),
            &mut track,
            &mut writes,
        );
        assert_eq!(track.dsp_fields.get("filterLcOn"), Some(&json!(true)));
        assert_eq!(bare, vec![("filterLcOn".to_string(), json!(true))]);
        assert_eq!(writes[0].path, "ch.0.preamp.filter.0.on");
        assert_eq!(writes[0].value, json!(1));
    }

    #[test]
    fn filter_update_lc_freq_clamps_and_writes_norm() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        handle_midi_filter_update(
            &json!({"filterLcFreq": {"value": 1.5}}),
            &input_slot(),
            &mut track,
            &mut writes,
        );
        assert_eq!(track.dsp_fields.get("filterLcFreq"), Some(&json!(1.0))); // clamped
        assert_eq!(writes[0].path, "ch.0.preamp.filter.0.freq");
        assert_eq!(writes[0].format, MsFormat::Norm);
    }

    #[test]
    fn filter_update_pre_gain_converts_to_real_db_for_headamp_gain() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        handle_midi_filter_update(
            &json!({"filterPreGain": {"value": 1.0}}),
            &input_slot(),
            &mut track,
            &mut writes,
        );
        assert_eq!(writes[0].path, "ch.0.headamp.gain");
        assert_eq!(
            writes[0].value,
            json!(crate::dsp_field_metadata::headamp_gain_normalized_to_db(
                1.0
            ))
        );
        assert_eq!(writes[0].format, MsFormat::Val);
    }

    #[test]
    fn filter_update_phase_invert_writes_preamp_inv() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        handle_midi_filter_update(
            &json!({"filterPhaseInvert": {"value": true}}),
            &input_slot(),
            &mut track,
            &mut writes,
        );
        assert_eq!(writes[0].path, "ch.0.preamp.inv");
        assert_eq!(writes[0].value, json!(1));
    }

    #[test]
    fn filter_update_works_on_main_slot() {
        let mut track = default_track(&main_slot());
        let mut writes = Vec::new();
        let bare = handle_midi_filter_update(
            &json!({"filterLcOn": {"value": true}}),
            &main_slot(),
            &mut track,
            &mut writes,
        );
        assert!(!bare.is_empty());
        assert_eq!(writes.len(), 2); // both L and R channels of the stereo Main
    }

    #[test]
    fn eq_update_absent_fields_is_a_no_op() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let bare = handle_midi_eq_update(&json!({}), &input_slot(), &mut track, &mut writes);
        assert!(bare.is_empty());
    }

    #[test]
    fn eq_update_band_2_on_writes_shared_peq_on_path() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let bare = handle_midi_eq_update(
            &json!({"eq2On": {"value": true}}),
            &input_slot(),
            &mut track,
            &mut writes,
        );
        assert_eq!(track.dsp_fields.get("eq2On"), Some(&json!(true)));
        assert_eq!(bare, vec![("eq2On".to_string(), json!(true))]);
        assert_eq!(writes[0].path, "ch.0.peq.on");
        assert_eq!(writes[0].value, json!(1));
    }

    #[test]
    fn eq_update_band_3_freq_writes_zero_based_band_index() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        handle_midi_eq_update(
            &json!({"eq3Freq": {"value": 0.5}}),
            &input_slot(),
            &mut track,
            &mut writes,
        );
        assert_eq!(writes[0].path, "ch.0.peq.bands.2.freq");
        assert_eq!(writes[0].format, MsFormat::Norm);
    }

    #[test]
    fn eq_update_type_converts_normalized_to_index_and_writes_raw() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        // eq_type_normalized_to_index(1.0) should be the last MS_EQ_TYPE_NAMES index (5).
        handle_midi_eq_update(
            &json!({"eq1Type": {"value": 1.0}}),
            &input_slot(),
            &mut track,
            &mut writes,
        );
        let expected = crate::dsp_field_metadata::eq_type_normalized_to_index(1.0);
        assert_eq!(track.dsp_fields.get("eq1Type"), Some(&json!(expected)));
        assert_eq!(writes[0].path, "ch.0.peq.bands.0.type");
        assert_eq!(writes[0].value, json!(expected));
        // JS omits `format` entirely on this one write, which `queueMsWrite`'s
        // `w.format || "val"` call site resolves to "val" (index.js:4184).
        assert_eq!(writes[0].format, MsFormat::Val);
    }

    #[test]
    fn comp_update_absent_fields_is_a_no_op() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let bare = handle_midi_comp_update(&json!({}), &input_slot(), &mut track, &mut writes);
        assert!(bare.is_empty());
    }

    #[test]
    fn comp_update_on_writes_dyn_on() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let bare = handle_midi_comp_update(
            &json!({"compOn": {"value": true}}),
            &input_slot(),
            &mut track,
            &mut writes,
        );
        assert_eq!(bare, vec![("compOn".to_string(), json!(true))]);
        assert_eq!(writes[0].path, "ch.0.dyn.on");
        assert_eq!(writes[0].value, json!(1));
    }

    #[test]
    fn comp_update_ratio_maps_to_dyn_ratio() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        handle_midi_comp_update(
            &json!({"compRatio": {"value": 0.5}}),
            &input_slot(),
            &mut track,
            &mut writes,
        );
        assert_eq!(writes[0].path, "ch.0.dyn.ratio");
        assert_eq!(writes[0].format, MsFormat::Norm);
    }

    #[test]
    fn comp_update_wetdry_maps_to_dyn_mix() {
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        handle_midi_comp_update(
            &json!({"compWetdry": {"value": 0.3}}),
            &input_slot(),
            &mut track,
            &mut writes,
        );
        assert_eq!(writes[0].path, "ch.0.dyn.mix");
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

    #[test]
    fn filter_update_is_a_no_op_for_empty_slot() {
        let empty_slot = LayoutSlot {
            object_id: 5,
            kind: LayoutSlotKind::Empty,
            ms_channels: vec![],
            ms_primary: None,
            pan_locked: false,
        };
        let mut track = default_track(&input_slot());
        let mut writes = Vec::new();
        let bare = handle_midi_filter_update(
            &json!({"filterLcOn": {"value": true}}),
            &empty_slot,
            &mut track,
            &mut writes,
        );
        assert!(bare.is_empty());
        assert!(writes.is_empty());
    }

    #[test]
    fn filter_update_is_accepted_for_bus_slot() {
        let mut track = default_track(&bus_slot());
        let mut writes = Vec::new();
        let bare = handle_midi_filter_update(
            &json!({"filterLcOn": {"value": true}}),
            &bus_slot(),
            &mut track,
            &mut writes,
        );
        assert_eq!(bare, vec![("filterLcOn".to_string(), json!(true))]);
        assert_eq!(writes[0].path, "ch.50.preamp.filter.0.on");
    }

    /// `COMP_CONTINUOUS_FIELD_MAP` (C1->MS, here) and `ms_param_apply::DYN_TO_C1` (MS->C1)
    /// are two independently hand-maintained tables of the same field pairs. They are NOT
    /// literal inverses: `DYN_TO_C1` additionally carries `("on", "compOn")`, because its
    /// consumer resolves every `dyn.*` path through that one table, whereas `compOn` is
    /// handled outside this table here (it needs a boolean 0/1 `val` write, not a clamped
    /// `norm` one). Dropping that entry from `DYN_TO_C1` was a real bug once, so this test
    /// pins both halves of the invariant: same continuous pairs in both directions, plus the
    /// `on`/`compOn` asymmetry present on exactly the side that needs it.
    #[test]
    fn comp_continuous_field_map_is_the_inverse_of_dyn_to_c1() {
        use std::collections::HashSet;

        let ours: HashSet<(&str, &str)> = COMP_CONTINUOUS_FIELD_MAP.iter().copied().collect();
        let theirs_flipped: HashSet<(&str, &str)> = crate::ms_param_apply::DYN_TO_C1
            .iter()
            .filter(|(ms, _)| *ms != "on")
            .map(|(ms, c1)| (*c1, *ms))
            .collect();

        assert_eq!(
            ours, theirs_flipped,
            "COMP_CONTINUOUS_FIELD_MAP's (c1, ms) pairs must be the exact direction-flipped \
             inverse of ms_param_apply::DYN_TO_C1's (ms, c1) pairs (excluding DYN_TO_C1's \
             extra `on` entry) — a mismatch means the two tables have drifted apart"
        );

        assert!(
            crate::ms_param_apply::DYN_TO_C1.contains(&("on", "compOn")),
            "DYN_TO_C1 must keep its ('on', 'compOn') entry — without it the MS->C1 direction \
             silently resolves `dyn.on` to no field at all"
        );
        assert!(
            !COMP_CONTINUOUS_FIELD_MAP
                .iter()
                .any(|(c1, _)| *c1 == "compOn"),
            "compOn must stay OUT of COMP_CONTINUOUS_FIELD_MAP — it needs a boolean 0/1 `val` \
             write, not the clamped `norm` write every entry in this table gets"
        );
    }
}
