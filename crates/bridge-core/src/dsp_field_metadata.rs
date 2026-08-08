//! Console 1 DSP-field (Filter/EQ/Compressor) metadata and OSD display-value formatting.
//!
//! See `index.js`'s `CONSOLE1_DSP_FIELD_METADATA`/`formatConsole1DspDisplayValue` for the
//! original this ports, including the EQ-type index<->normalized conversion (Cubase's
//! generic-remote convention represents every parameter, discrete or continuous, as a
//! normalized 0..1 float) and the headamp-gain dB<->normalized conversion (this MS parameter
//! only offers "val" format, no "norm" companion, so the bridge does its own linear mapping).

use crate::pan_utils::clamp01;

pub const MS_EQ_TYPE_NAMES: [&str; 6] = ["PEQ", "VEQ", "Hi-Cut", "Lo-Cut", "Lo-Shelf", "Hi-Shelf"];

/// Every bare Filter/EQ/Compressor field name known to [`dsp_field_metadata`] — the single
/// source of truth other modules should check their own field-name lists against. Any module
/// that independently enumerates this same field set (e.g. `track_cache::default_dsp_fields`)
/// should have a test asserting its key set matches this list exactly, so a future typo/rename
/// on either side is caught by a test instead of silently producing a `TrackInfo.dsp_fields`
/// entry that never gets read/written by the metadata-driven code path.
pub const DSP_FIELD_NAMES: [&str; 32] = [
    "filterLcOn",
    "filterLcFreq",
    "filterPreGain",
    "filterPhaseInvert",
    "eq1On",
    "eq1Freq",
    "eq1Gain",
    "eq1Q",
    "eq1Type",
    "eq2On",
    "eq2Freq",
    "eq2Gain",
    "eq2Q",
    "eq2Type",
    "eq3On",
    "eq3Freq",
    "eq3Gain",
    "eq3Q",
    "eq3Type",
    "eq4On",
    "eq4Freq",
    "eq4Gain",
    "eq4Q",
    "eq4Type",
    "compOn",
    "compRatio",
    "compAttack",
    "compRelease",
    "compMakeup",
    "compComp",
    "compKnee",
    "compWetdry",
];

/// Convert an MS `peq.bands.N.type` index into the normalized 0..1 float Console 1's EQ Type
/// knob expects (Cubase generic-remote convention: every parameter is 0..1, `quantisation`
/// only tells the receiving control how many detents to snap to within that range).
pub fn eq_type_index_to_normalized(index: usize) -> f64 {
    if MS_EQ_TYPE_NAMES.len() > 1 {
        index as f64 / (MS_EQ_TYPE_NAMES.len() - 1) as f64
    } else {
        0.0
    }
}

/// Inverse of [`eq_type_index_to_normalized`] — normalized 0..1 knob value -> MS type index.
pub fn eq_type_normalized_to_index(normalized: f64) -> usize {
    let max_index = (MS_EQ_TYPE_NAMES.len() - 1) as f64;
    let idx = (normalized * max_index).round().clamp(0.0, max_index);
    idx as usize
}

pub const HEADAMP_GAIN_MIN_DB: f64 = -12.0;
pub const HEADAMP_GAIN_MAX_DB: f64 = 60.0;

/// `ch.<n>.headamp.gain` only offers Mixing Station's "val" format (real dB) — no "norm"
/// companion — so the bridge does its own linear dB<->0..1 conversion for Console 1's knob.
pub fn headamp_gain_db_to_normalized(db: f64) -> f64 {
    clamp01((db - HEADAMP_GAIN_MIN_DB) / (HEADAMP_GAIN_MAX_DB - HEADAMP_GAIN_MIN_DB))
}

pub fn headamp_gain_normalized_to_db(normalized: f64) -> f64 {
    HEADAMP_GAIN_MIN_DB + clamp01(normalized) * (HEADAMP_GAIN_MAX_DB - HEADAMP_GAIN_MIN_DB)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DspFieldKind {
    Bool,
    Percent,
    /// Defined but currently unreachable from `dsp_field_metadata`'s real field table — mirrors
    /// the JS source's own JSDoc typedef listing `"index"` without any field actually using it.
    Index,
    Hz,
    Db,
    Ms,
    Ratio,
    Raw,
    EqType,
    /// Present in the JS source's actual code (`compKnee`'s metadata + `formatConsole1DspDisplayValue`'s
    /// `case "knee"`) despite being omitted from the JSDoc typedef comment there — not a typo here.
    Knee,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DspFieldMeta {
    pub name: &'static str,
    pub quantisation: u32,
    pub kind: DspFieldKind,
    pub default_value: f64,
}

/// Look up a Console 1 DSP field's OSD metadata by its bare field name (e.g. `"filterLcOn"`,
/// `"eq2Freq"`, `"compRatio"`). Returns `None` for anything not in the fixed DSP field set.
pub fn dsp_field_metadata(field: &str) -> Option<DspFieldMeta> {
    use DspFieldKind::*;

    match field {
        "filterLcOn" => Some(DspFieldMeta {
            name: "Low Cut On",
            quantisation: 2,
            kind: Bool,
            default_value: 0.0,
        }),
        "filterLcFreq" => Some(DspFieldMeta {
            name: "Low Cut Freq",
            quantisation: 0,
            kind: Hz,
            default_value: 0.0,
        }),
        "filterPreGain" => Some(DspFieldMeta {
            name: "Input Gain",
            quantisation: 0,
            kind: Db,
            default_value: headamp_gain_db_to_normalized(0.0),
        }),
        "filterPhaseInvert" => Some(DspFieldMeta {
            name: "Phase Invert",
            quantisation: 2,
            kind: Bool,
            default_value: 0.0,
        }),
        "compOn" => Some(DspFieldMeta {
            name: "Comp On",
            quantisation: 2,
            kind: Bool,
            default_value: 0.0,
        }),
        "compRatio" => Some(DspFieldMeta {
            name: "Ratio",
            quantisation: 12,
            kind: Ratio,
            default_value: 0.0,
        }),
        "compAttack" => Some(DspFieldMeta {
            name: "Attack",
            quantisation: 0,
            kind: Ms,
            default_value: 0.0,
        }),
        "compRelease" => Some(DspFieldMeta {
            name: "Release",
            quantisation: 0,
            kind: Ms,
            default_value: 0.0,
        }),
        "compMakeup" => Some(DspFieldMeta {
            name: "Gain",
            quantisation: 0,
            kind: Db,
            default_value: 0.0,
        }),
        "compComp" => Some(DspFieldMeta {
            name: "Threshold",
            quantisation: 0,
            kind: Db,
            default_value: 0.0,
        }),
        "compKnee" => Some(DspFieldMeta {
            name: "Knee",
            quantisation: 6,
            kind: Knee,
            default_value: 0.0,
        }),
        "compWetdry" => Some(DspFieldMeta {
            name: "Wet/Dry",
            quantisation: 0,
            kind: Percent,
            default_value: 0.0,
        }),
        "eq1On" => Some(DspFieldMeta {
            name: "EQ 1 On",
            quantisation: 2,
            kind: Bool,
            default_value: 0.0,
        }),
        "eq1Freq" => Some(DspFieldMeta {
            name: "EQ 1 Freq",
            quantisation: 0,
            kind: Hz,
            default_value: 0.5,
        }),
        "eq1Gain" => Some(DspFieldMeta {
            name: "EQ 1 Gain",
            quantisation: 0,
            kind: Db,
            default_value: 0.5,
        }),
        "eq1Q" => Some(DspFieldMeta {
            name: "EQ 1 Q",
            quantisation: 0,
            kind: Raw,
            default_value: 0.5,
        }),
        "eq1Type" => Some(DspFieldMeta {
            name: "EQ 1 Type",
            quantisation: MS_EQ_TYPE_NAMES.len() as u32,
            kind: EqType,
            default_value: 0.0,
        }),
        "eq2On" => Some(DspFieldMeta {
            name: "EQ 2 On",
            quantisation: 2,
            kind: Bool,
            default_value: 0.0,
        }),
        "eq2Freq" => Some(DspFieldMeta {
            name: "EQ 2 Freq",
            quantisation: 0,
            kind: Hz,
            default_value: 0.5,
        }),
        "eq2Gain" => Some(DspFieldMeta {
            name: "EQ 2 Gain",
            quantisation: 0,
            kind: Db,
            default_value: 0.5,
        }),
        "eq2Q" => Some(DspFieldMeta {
            name: "EQ 2 Q",
            quantisation: 0,
            kind: Raw,
            default_value: 0.5,
        }),
        "eq2Type" => Some(DspFieldMeta {
            name: "EQ 2 Type",
            quantisation: MS_EQ_TYPE_NAMES.len() as u32,
            kind: EqType,
            default_value: 0.0,
        }),
        "eq3On" => Some(DspFieldMeta {
            name: "EQ 3 On",
            quantisation: 2,
            kind: Bool,
            default_value: 0.0,
        }),
        "eq3Freq" => Some(DspFieldMeta {
            name: "EQ 3 Freq",
            quantisation: 0,
            kind: Hz,
            default_value: 0.5,
        }),
        "eq3Gain" => Some(DspFieldMeta {
            name: "EQ 3 Gain",
            quantisation: 0,
            kind: Db,
            default_value: 0.5,
        }),
        "eq3Q" => Some(DspFieldMeta {
            name: "EQ 3 Q",
            quantisation: 0,
            kind: Raw,
            default_value: 0.5,
        }),
        "eq3Type" => Some(DspFieldMeta {
            name: "EQ 3 Type",
            quantisation: MS_EQ_TYPE_NAMES.len() as u32,
            kind: EqType,
            default_value: 0.0,
        }),
        "eq4On" => Some(DspFieldMeta {
            name: "EQ 4 On",
            quantisation: 2,
            kind: Bool,
            default_value: 0.0,
        }),
        "eq4Freq" => Some(DspFieldMeta {
            name: "EQ 4 Freq",
            quantisation: 0,
            kind: Hz,
            default_value: 0.5,
        }),
        "eq4Gain" => Some(DspFieldMeta {
            name: "EQ 4 Gain",
            quantisation: 0,
            kind: Db,
            default_value: 0.5,
        }),
        "eq4Q" => Some(DspFieldMeta {
            name: "EQ 4 Q",
            quantisation: 0,
            kind: Raw,
            default_value: 0.5,
        }),
        "eq4Type" => Some(DspFieldMeta {
            name: "EQ 4 Type",
            quantisation: MS_EQ_TYPE_NAMES.len() as u32,
            kind: EqType,
            default_value: 0.0,
        }),
        _ => None,
    }
}

/// Build the OSD app's `display_value` string for a bare field update, per its metadata kind.
/// `real_val` is the companion "val"-format (real-unit) value, if known yet — falls back to a
/// plain percentage of the 0..1 sync `value` if it hasn't arrived yet, so `display_value` is
/// never blank.
///
/// Note: on `NaN` input this diverges from the JS original (`NaN` is falsy in JS, so `bool`
/// stays `"Off"`-shaped and `Math.round(NaN)` stringifies as `"NaN"`; Rust's `!= 0.0` is `true`
/// for `NaN` and `f64::round` propagates `NaN` through `as i64` as `0`). Real MS traffic never
/// sends `NaN` for these fields, so this is unlikely to matter in practice.
pub fn format_console1_dsp_display_value(
    kind: DspFieldKind,
    value: f64,
    real_val: Option<f64>,
) -> String {
    match kind {
        DspFieldKind::Bool => {
            if value != 0.0 {
                "On".to_string()
            } else {
                "Off".to_string()
            }
        }
        DspFieldKind::Index => format!("{}", value.round() as i64),
        DspFieldKind::EqType => {
            let idx = value.round() as i64;
            MS_EQ_TYPE_NAMES
                .get(idx as usize)
                .map(|s| s.to_string())
                .unwrap_or_else(|| idx.to_string())
        }
        _ => match real_val {
            None => format!("{}%", (value * 100.0).round() as i64),
            Some(n) => match kind {
                DspFieldKind::Hz => format!("{n:.1} Hz"),
                DspFieldKind::Db => format!("{n:.1} dB"),
                DspFieldKind::Ms => format!("{n:.1} ms"),
                DspFieldKind::Ratio => format!("{n:.1}:1"),
                DspFieldKind::Knee => format!("{n:.0}"),
                DspFieldKind::Raw => format!("{n:.2}"),
                _ => format!("{}%", (value * 100.0).round() as i64),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eq_type_index_0_maps_to_normalized_0() {
        assert_eq!(eq_type_index_to_normalized(0), 0.0);
    }

    #[test]
    fn eq_type_last_index_maps_to_normalized_1() {
        assert_eq!(eq_type_index_to_normalized(MS_EQ_TYPE_NAMES.len() - 1), 1.0);
    }

    #[test]
    fn eq_type_index_to_normalized_and_back_round_trips_for_every_index() {
        for i in 0..MS_EQ_TYPE_NAMES.len() {
            let normalized = eq_type_index_to_normalized(i);
            assert_eq!(eq_type_normalized_to_index(normalized), i);
        }
    }

    #[test]
    fn eq_type_normalized_to_index_clamps_out_of_range_input() {
        assert_eq!(eq_type_normalized_to_index(-1.0), 0);
        assert_eq!(eq_type_normalized_to_index(2.0), MS_EQ_TYPE_NAMES.len() - 1);
    }

    #[test]
    fn headamp_gain_min_db_maps_to_normalized_0() {
        assert_eq!(headamp_gain_db_to_normalized(HEADAMP_GAIN_MIN_DB), 0.0);
    }

    #[test]
    fn headamp_gain_max_db_maps_to_normalized_1() {
        assert_eq!(headamp_gain_db_to_normalized(HEADAMP_GAIN_MAX_DB), 1.0);
    }

    #[test]
    fn headamp_gain_0db_round_trips() {
        let normalized = headamp_gain_db_to_normalized(0.0);
        assert!((headamp_gain_normalized_to_db(normalized) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn headamp_gain_out_of_range_db_clamps() {
        assert_eq!(
            headamp_gain_db_to_normalized(HEADAMP_GAIN_MIN_DB - 100.0),
            0.0
        );
        assert_eq!(
            headamp_gain_db_to_normalized(HEADAMP_GAIN_MAX_DB + 100.0),
            1.0
        );
    }

    #[test]
    fn dsp_field_metadata_known_bool_field() {
        let meta = dsp_field_metadata("filterLcOn").unwrap();
        assert_eq!(meta.name, "Low Cut On");
        assert_eq!(meta.quantisation, 2);
        assert_eq!(meta.kind, DspFieldKind::Bool);
    }

    #[test]
    fn dsp_field_metadata_eq_type_uses_ms_eq_type_names_length_as_quantisation() {
        let meta = dsp_field_metadata("eq1Type").unwrap();
        assert_eq!(meta.quantisation, MS_EQ_TYPE_NAMES.len() as u32);
        assert_eq!(meta.kind, DspFieldKind::EqType);
    }

    #[test]
    fn dsp_field_metadata_comp_knee_uses_knee_kind() {
        let meta = dsp_field_metadata("compKnee").unwrap();
        assert_eq!(meta.kind, DspFieldKind::Knee);
    }

    #[test]
    fn dsp_field_metadata_all_four_eq_bands_present() {
        for n in 1..=4 {
            assert!(dsp_field_metadata(&format!("eq{n}On")).is_some());
            assert!(dsp_field_metadata(&format!("eq{n}Freq")).is_some());
            assert!(dsp_field_metadata(&format!("eq{n}Gain")).is_some());
            assert!(dsp_field_metadata(&format!("eq{n}Q")).is_some());
            assert!(dsp_field_metadata(&format!("eq{n}Type")).is_some());
        }
    }

    #[test]
    fn dsp_field_metadata_unknown_field_returns_none() {
        assert_eq!(dsp_field_metadata("notARealField"), None);
    }

    #[test]
    fn format_bool_kind() {
        assert_eq!(
            format_console1_dsp_display_value(DspFieldKind::Bool, 1.0, None),
            "On"
        );
        assert_eq!(
            format_console1_dsp_display_value(DspFieldKind::Bool, 0.0, None),
            "Off"
        );
    }

    #[test]
    fn format_eqtype_kind_uses_names_table() {
        assert_eq!(
            format_console1_dsp_display_value(DspFieldKind::EqType, 2.0, None),
            "Hi-Cut"
        );
    }

    #[test]
    fn format_falls_back_to_percentage_when_real_val_not_yet_known() {
        assert_eq!(
            format_console1_dsp_display_value(DspFieldKind::Hz, 0.5, None),
            "50%"
        );
    }

    #[test]
    fn format_uses_real_val_unit_suffix_once_known() {
        assert_eq!(
            format_console1_dsp_display_value(DspFieldKind::Hz, 0.3, Some(120.4)),
            "120.4 Hz"
        );
        assert_eq!(
            format_console1_dsp_display_value(DspFieldKind::Db, 0.5, Some(-6.0)),
            "-6.0 dB"
        );
        assert_eq!(
            format_console1_dsp_display_value(DspFieldKind::Ratio, 0.5, Some(4.0)),
            "4.0:1"
        );
        assert_eq!(
            format_console1_dsp_display_value(DspFieldKind::Knee, 0.5, Some(3.0)),
            "3"
        );
    }

    #[test]
    fn format_index_kind_rounds_and_stringifies() {
        assert_eq!(
            format_console1_dsp_display_value(DspFieldKind::Index, 2.0, None),
            "2"
        );
    }
}
