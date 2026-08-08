//! Applies a single Mixing Station parameter update onto a cached Console 1 track.
//!
//! Port of `index.js`'s `applyMsParamToTrack`. Unlike the JS original -- which enqueues
//! Filter/EQ/Compressor ("bare field") updates directly via a module-level queue -- this
//! stays pure: bare-field updates are returned as data (`bare_field_updates`) for the
//! caller to route into `bare_update_queue::BareUpdateQueue`. `changed` fields are meant
//! for `update_queue::UpdateQueue`, matching Plan 2b's existing queue.

use crate::send_mapping::SendMapping;
use crate::track_cache::{default_name_for_slot, TrackInfo};
use crate::track_layout::LayoutSlot;
use crate::value_coercion::is_negative_infinity_db;
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsValueFormat {
    Val,
    Norm,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct MsParamApplyResult {
    /// Fields eligible for `trackBatch` (route through `update_queue::UpdateQueue`).
    pub changed: HashMap<String, Value>,
    /// Filter/EQ/Compressor cache-sync fields (route through `bare_update_queue::BareUpdateQueue`).
    /// Never overlaps with `changed` -- these field names are never trackBatch-eligible.
    pub bare_field_updates: Vec<(String, Value)>,
}

// `on` is added separately in JS (`DYN_TO_C1.on = "compOn"`, index.js:91) rather than
// living in `COMP_CONTINUOUS_FIELD_MAP` (whose own doc comment says: "`compOn` is
// handled separately (needs boolean 0/1 val, not norm)") -- keep it here too, or
// `apply_dyn_update`'s "on" branch silently resolves to no field at all.
pub(crate) const DYN_TO_C1: &[(&str, &str)] = &[
    ("on", "compOn"),
    ("ratio", "compRatio"),
    ("attack", "compAttack"),
    ("release", "compRelease"),
    ("gain", "compMakeup"),
    ("thr", "compComp"),
    ("knee", "compKnee"),
    ("mix", "compWetdry"),
];

fn dyn_to_c1(field: &str) -> Option<&'static str> {
    DYN_TO_C1
        .iter()
        .find(|(ms, _)| *ms == field)
        .map(|(_, c1)| *c1)
}

/// Convenience wrapper for callers that don't need send-slot remapping (i.e. any
/// `param_path` that isn't `mix.sends.*`). Send updates fall through to a no-op for
/// `mix.sends.*` paths here -- use `apply_ms_param_to_track_with_send_mapping` when the
/// param path may be a send.
pub fn apply_ms_param_to_track(
    track: &mut TrackInfo,
    slot: &LayoutSlot,
    bus_channel_start: usize,
    param_path: &str,
    value: &Value,
    format: Option<MsValueFormat>,
) -> MsParamApplyResult {
    apply_ms_param_to_track_inner(
        track,
        slot,
        bus_channel_start,
        param_path,
        value,
        format,
        None,
    )
}

pub fn apply_ms_param_to_track_with_send_mapping(
    track: &mut TrackInfo,
    slot: &LayoutSlot,
    bus_channel_start: usize,
    param_path: &str,
    value: &Value,
    format: Option<MsValueFormat>,
    send_mapping: &SendMapping,
) -> MsParamApplyResult {
    apply_ms_param_to_track_inner(
        track,
        slot,
        bus_channel_start,
        param_path,
        value,
        format,
        Some(send_mapping),
    )
}

fn apply_ms_param_to_track_inner(
    track: &mut TrackInfo,
    slot: &LayoutSlot,
    bus_channel_start: usize,
    param_path: &str,
    value: &Value,
    format: Option<MsValueFormat>,
    send_mapping: Option<&SendMapping>,
) -> MsParamApplyResult {
    let mut result = MsParamApplyResult::default();

    match param_path {
        "cfg.name" => {
            if let Some(s) = value.as_str() {
                let next = if s.trim().is_empty() {
                    default_name_for_slot(slot, bus_channel_start)
                } else {
                    s.to_string()
                };
                if track.name != next {
                    track.name = next.clone();
                    result.changed.insert("name".to_string(), json!(next));
                }
            }
        }
        "cfg.color" => {
            if let Some(color_int) = crate::midi_color_utils::ms_color_to_softube_color_int(value) {
                if track.color != color_int {
                    track.color = color_int;
                    result.changed.insert("color".to_string(), json!(color_int));
                }
            }
        }
        "mix.lvl" => {
            let next = if is_negative_infinity_db(value) {
                json!("-Infinity")
            } else {
                value.clone()
            };
            if track.volume != next {
                track.volume = next.clone();
                result.changed.insert("volume".to_string(), next);
            }
        }
        "mix.on" => {
            let next = !value.as_bool().unwrap_or(false);
            if track.mute != next {
                track.mute = next;
                result.changed.insert("mute".to_string(), json!(next));
            }
        }
        "solo" => {
            let next = value.as_bool().unwrap_or(false);
            if track.solo != next {
                track.solo = next;
                result.changed.insert("solo".to_string(), json!(next));
            }
        }
        "mix.pan" => {
            if let Some(next) = value.as_f64() {
                if track.pan != next {
                    track.pan = next;
                    result.changed.insert("pan".to_string(), json!(next));
                }
            }
        }
        "info.isActive" => {
            let next = value.as_bool().unwrap_or(false);
            if track.is_active != next {
                track.is_active = next;
                result.changed.insert("isActive".to_string(), json!(next));
            }
        }
        "selected" => {
            let next = value.as_bool().unwrap_or(false);
            if track.selected != next {
                track.selected = next;
                result.changed.insert("selected".to_string(), json!(next));
            }
        }
        "preamp.filter.0.on" => {
            set_cache_only(
                track,
                &mut result,
                "filterLcOn",
                json!(value.as_bool().unwrap_or(false)),
            );
        }
        "preamp.filter.0.freq" => {
            if format == Some(MsValueFormat::Val) {
                set_real_value_only(track, "filterLcFreq", value);
            } else {
                set_cache_only(track, &mut result, "filterLcFreq", value.clone());
            }
        }
        "headamp.gain" => {
            set_real_value_only(track, "filterPreGain", value);
            let db = value.as_f64().unwrap_or(0.0);
            let normalized = crate::dsp_field_metadata::headamp_gain_db_to_normalized(db);
            set_cache_only(track, &mut result, "filterPreGain", json!(normalized));
        }
        "preamp.inv" => {
            set_cache_only(
                track,
                &mut result,
                "filterPhaseInvert",
                json!(value.as_bool().unwrap_or(false)),
            );
        }
        "peq.on" => {
            let on = value.as_bool().unwrap_or(false);
            for n in 1..=4 {
                set_cache_only(track, &mut result, &format!("eq{n}On"), json!(on));
            }
        }
        other => {
            if let Some(rest) = other.strip_prefix("mix.sends.") {
                apply_send_update(track, &mut result, rest, value, send_mapping);
            } else if let Some(rest) = other.strip_prefix("peq.bands.") {
                apply_eq_band_update(track, &mut result, rest, value, format);
            } else if let Some(field) = dyn_field_name(other) {
                apply_dyn_update(track, &mut result, field, value, format);
            }
        }
    }

    result
}

fn set_cache_only(
    track: &mut TrackInfo,
    result: &mut MsParamApplyResult,
    field: &str,
    value: Value,
) {
    track.dsp_fields.insert(field.to_string(), value.clone());
    result.bare_field_updates.push((field.to_string(), value));
}

fn set_real_value_only(track: &mut TrackInfo, field: &str, value: &Value) {
    if let Some(n) = value.as_f64() {
        track.dsp_real_values.insert(field.to_string(), n);
    }
}

fn dyn_field_name(param_path: &str) -> Option<&str> {
    param_path.strip_prefix("dyn.").filter(|f| {
        matches!(
            *f,
            "on" | "ratio" | "attack" | "release" | "gain" | "thr" | "knee" | "mix"
        )
    })
}

fn apply_send_update(
    track: &mut TrackInfo,
    result: &mut MsParamApplyResult,
    rest: &str, // "<msSendIndex>.lvl" or "<msSendIndex>.on"
    value: &Value,
    send_mapping: Option<&SendMapping>,
) {
    let Some(mapping) = send_mapping else { return };
    let mut parts = rest.splitn(2, '.');
    let Some(idx_str) = parts.next() else { return };
    let Some(kind) = parts.next() else { return };
    let Ok(ms_send_index) = idx_str.parse::<usize>() else {
        return;
    };
    let Some(&c1_slot) = mapping.ms_send_index_to_c1_slot.get(&ms_send_index) else {
        return;
    };
    let send_number = c1_slot + 1; // 1-based

    match kind {
        "lvl" => {
            let next = if is_negative_infinity_db(value) {
                json!("-Infinity")
            } else {
                value.clone()
            };
            if track.send_levels[c1_slot] != next {
                track.send_levels[c1_slot] = next.clone();
                result.changed.insert(format!("send{send_number}"), next);
            }
        }
        "on" => {
            let next = value.as_bool().unwrap_or(false);
            if track.send_on[c1_slot] != next {
                track.send_on[c1_slot] = next;
                result
                    .changed
                    .insert(format!("send{send_number}On"), json!(next));
            }
        }
        _ => {}
    }
}

fn apply_eq_band_update(
    track: &mut TrackInfo,
    result: &mut MsParamApplyResult,
    rest: &str, // "<bandIndex>.freq|gain|q|type"
    value: &Value,
    format: Option<MsValueFormat>,
) {
    let mut parts = rest.splitn(2, '.');
    let Some(idx_str) = parts.next() else { return };
    let Some(field) = parts.next() else { return };
    let Ok(band_index) = idx_str.parse::<usize>() else {
        return;
    };
    // Bound before incrementing: `band_index` comes straight off the wire, so
    // `band_index + 1` on a `usize::MAX`-magnitude index would overflow and panic.
    if band_index >= 4 {
        return;
    }
    let band_number = band_index + 1;
    let suffix = match field {
        "q" => "Q".to_string(),
        "freq" | "gain" | "type" => {
            let mut c = field.chars();
            match c.next() {
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                None => return,
            }
        }
        _ => return,
    };
    let key = format!("eq{band_number}{suffix}");

    if field != "type" && format == Some(MsValueFormat::Val) {
        set_real_value_only(track, &key, value);
    } else {
        set_cache_only(track, result, &key, value.clone());
    }
}

fn apply_dyn_update(
    track: &mut TrackInfo,
    result: &mut MsParamApplyResult,
    field: &str,
    value: &Value,
    format: Option<MsValueFormat>,
) {
    let Some(key) = dyn_to_c1(field) else { return };
    if field == "on" {
        set_cache_only(track, result, key, json!(value.as_bool().unwrap_or(false)));
    } else if field != "mix" && format == Some(MsValueFormat::Val) {
        set_real_value_only(track, key, value);
    } else {
        set_cache_only(track, result, key, value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console1_status_bank::Lifecycle;
    use crate::send_mapping::build_send_mapping;
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

    fn default_track() -> crate::track_cache::TrackInfo {
        create_default_track_for_slot(
            &input_slot(),
            "trk-1".to_string(),
            Lifecycle::Running,
            &default_colors(),
            48,
        )
    }

    #[test]
    fn cfg_name_applies_directly_when_non_empty() {
        let mut track = default_track();
        let result = apply_ms_param_to_track(
            &mut track,
            &input_slot(),
            48,
            "cfg.name",
            &json!("Vocal"),
            None,
        );
        assert_eq!(track.name, "Vocal");
        assert_eq!(result.changed.get("name"), Some(&json!("Vocal")));
    }

    #[test]
    fn cfg_name_falls_back_to_default_when_empty_or_whitespace() {
        let mut track = default_track();
        // Force the name away from its slot-derived default first, so applying an empty
        // MS name is a real, reportable change back to the default -- not a no-op.
        track.name = "Custom Name".to_string();
        let default_name = crate::track_cache::default_name_for_slot(&input_slot(), 48);

        let result = apply_ms_param_to_track(
            &mut track,
            &input_slot(),
            48,
            "cfg.name",
            &json!("   "),
            None,
        );
        assert_eq!(track.name, default_name);
        assert_eq!(result.changed.get("name"), Some(&json!(default_name)));
    }

    #[test]
    fn mix_lvl_negative_infinity_db_maps_to_json_negative_infinity_string() {
        let mut track = default_track();
        let result = apply_ms_param_to_track(
            &mut track,
            &input_slot(),
            48,
            "mix.lvl",
            &json!(-90.5),
            None,
        );
        assert_eq!(track.volume, json!("-Infinity"));
        assert_eq!(result.changed.get("volume"), Some(&json!("-Infinity")));
    }

    #[test]
    fn mix_on_inverts_into_mute() {
        let mut track = default_track();
        let result =
            apply_ms_param_to_track(&mut track, &input_slot(), 48, "mix.on", &json!(false), None);
        assert!(track.mute);
        assert_eq!(result.changed.get("mute"), Some(&json!(true)));
    }

    #[test]
    fn preamp_filter_freq_val_format_goes_to_real_value_only() {
        let mut track = default_track();
        let result = apply_ms_param_to_track(
            &mut track,
            &input_slot(),
            48,
            "preamp.filter.0.freq",
            &json!(120.0),
            Some(MsValueFormat::Val),
        );
        assert_eq!(track.dsp_real_values.get("filterLcFreq"), Some(&120.0));
        assert!(result.changed.is_empty());
        assert!(result.bare_field_updates.is_empty());
    }

    #[test]
    fn preamp_filter_freq_norm_format_goes_to_cache_and_bare_update() {
        let mut track = default_track();
        let result = apply_ms_param_to_track(
            &mut track,
            &input_slot(),
            48,
            "preamp.filter.0.freq",
            &json!(0.4),
            Some(MsValueFormat::Norm),
        );
        assert_eq!(track.dsp_fields.get("filterLcFreq"), Some(&json!(0.4)));
        assert!(result.changed.is_empty()); // never trackBatch-eligible
        assert_eq!(
            result.bare_field_updates,
            vec![("filterLcFreq".to_string(), json!(0.4))]
        );
    }

    #[test]
    fn headamp_gain_writes_both_real_value_and_derived_normalized_cache() {
        let mut track = default_track();
        let result = apply_ms_param_to_track(
            &mut track,
            &input_slot(),
            48,
            "headamp.gain",
            &json!(0.0),
            None,
        );
        assert_eq!(track.dsp_real_values.get("filterPreGain"), Some(&0.0));
        let expected_norm = crate::dsp_field_metadata::headamp_gain_db_to_normalized(0.0);
        assert_eq!(
            track.dsp_fields.get("filterPreGain"),
            Some(&json!(expected_norm))
        );
        assert_eq!(
            result.bare_field_updates,
            vec![("filterPreGain".to_string(), json!(expected_norm))]
        );
    }

    #[test]
    fn peq_on_fans_out_to_all_four_eq_band_on_fields() {
        let mut track = default_track();
        let result =
            apply_ms_param_to_track(&mut track, &input_slot(), 48, "peq.on", &json!(true), None);
        for n in 1..=4 {
            assert_eq!(
                track.dsp_fields.get(&format!("eq{n}On")),
                Some(&json!(true))
            );
        }
        assert_eq!(result.bare_field_updates.len(), 4);
    }

    #[test]
    fn send_lvl_maps_ms_send_index_to_c1_slot_via_send_mapping() {
        let mapping = build_send_mapping(&[], 16); // identity mapping
        let mut track = default_track();
        let result = apply_ms_param_to_track_with_send_mapping(
            &mut track,
            &input_slot(),
            48,
            "mix.sends.2.lvl",
            &json!(-4.0),
            None,
            &mapping,
        );
        assert_eq!(track.send_levels[2], json!(-4.0)); // ms index 2 -> identity -> c1 slot 2 -> send3
        assert_eq!(result.changed.get("send3"), Some(&json!(-4.0)));
    }

    #[test]
    fn send_on_maps_to_boolean_send_on_field() {
        let mapping = build_send_mapping(&[], 16);
        let mut track = default_track();
        let result = apply_ms_param_to_track_with_send_mapping(
            &mut track,
            &input_slot(),
            48,
            "mix.sends.0.on",
            &json!(true),
            None,
            &mapping,
        );
        assert!(track.send_on[0]);
        assert_eq!(result.changed.get("send1On"), Some(&json!(true)));
    }

    #[test]
    fn peq_band_freq_norm_goes_to_cache_val_goes_to_real_value() {
        let mut track = default_track();
        apply_ms_param_to_track(
            &mut track,
            &input_slot(),
            48,
            "peq.bands.0.freq",
            &json!(0.6),
            Some(MsValueFormat::Norm),
        );
        assert_eq!(track.dsp_fields.get("eq1Freq"), Some(&json!(0.6)));
        apply_ms_param_to_track(
            &mut track,
            &input_slot(),
            48,
            "peq.bands.0.freq",
            &json!(1000.0),
            Some(MsValueFormat::Val),
        );
        assert_eq!(track.dsp_real_values.get("eq1Freq"), Some(&1000.0));
    }

    #[test]
    fn peq_band_type_always_goes_to_cache_regardless_of_format() {
        let mut track = default_track();
        let result = apply_ms_param_to_track(
            &mut track,
            &input_slot(),
            48,
            "peq.bands.1.type",
            &json!(3.0),
            Some(MsValueFormat::Val),
        );
        assert_eq!(track.dsp_fields.get("eq2Type"), Some(&json!(3.0)));
        assert_eq!(
            result.bare_field_updates,
            vec![("eq2Type".to_string(), json!(3.0))]
        );
    }

    #[test]
    fn dyn_on_goes_to_cache_only() {
        let mut track = default_track();
        let result =
            apply_ms_param_to_track(&mut track, &input_slot(), 48, "dyn.on", &json!(true), None);
        assert_eq!(track.dsp_fields.get("compOn"), Some(&json!(true)));
        assert_eq!(
            result.bare_field_updates,
            vec![("compOn".to_string(), json!(true))]
        );
    }

    #[test]
    fn dyn_mix_never_dual_subscribed_always_syncs_via_cache() {
        let mut track = default_track();
        let result = apply_ms_param_to_track(
            &mut track,
            &input_slot(),
            48,
            "dyn.mix",
            &json!(0.3),
            Some(MsValueFormat::Norm),
        );
        assert_eq!(track.dsp_fields.get("compWetdry"), Some(&json!(0.3)));
        assert_eq!(
            result.bare_field_updates,
            vec![("compWetdry".to_string(), json!(0.3))]
        );
    }

    #[test]
    fn dyn_ratio_val_format_goes_to_real_value_only() {
        let mut track = default_track();
        let result = apply_ms_param_to_track(
            &mut track,
            &input_slot(),
            48,
            "dyn.ratio",
            &json!(4.0),
            Some(MsValueFormat::Val),
        );
        assert_eq!(track.dsp_real_values.get("compRatio"), Some(&4.0));
        assert!(result.bare_field_updates.is_empty());
    }

    #[test]
    fn peq_band_index_at_usize_max_is_a_no_op_and_does_not_overflow() {
        let mut track = default_track();
        let before = track.clone();
        let result = apply_ms_param_to_track(
            &mut track,
            &input_slot(),
            48,
            &format!("peq.bands.{}.freq", usize::MAX),
            &json!(0.6),
            Some(MsValueFormat::Norm),
        );
        assert!(result.changed.is_empty());
        assert!(result.bare_field_updates.is_empty());
        assert_eq!(track, before);
    }

    #[test]
    fn unrecognized_param_path_is_a_no_op() {
        let mut track = default_track();
        let result = apply_ms_param_to_track(
            &mut track,
            &input_slot(),
            48,
            "some.unknown.path",
            &json!(1),
            None,
        );
        assert!(result.changed.is_empty());
        assert!(result.bare_field_updates.is_empty());
    }

    #[test]
    fn setting_same_value_twice_reports_no_change_the_second_time() {
        let mut track = default_track();
        apply_ms_param_to_track(&mut track, &input_slot(), 48, "solo", &json!(true), None);
        let result =
            apply_ms_param_to_track(&mut track, &input_slot(), 48, "solo", &json!(true), None);
        assert!(result.changed.is_empty());
    }
}
