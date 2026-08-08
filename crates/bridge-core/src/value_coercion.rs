//! Pure value-coercion/normalization helpers shared across the bridge's config loading,
//! Mixing Station <-> Console 1 value translation, and WebSocket payload handling.
//!
//! Ported from `valueCoercion.js`. Kept separate from the rest of the bridge core so this
//! logic can be unit-tested in isolation, same rationale as the JS original.

use serde_json::{json, Value};

pub const MIN_DB_AS_NEGATIVE_INFINITY: f64 = -90.0;
pub const DB_NEGATIVE_INFINITY_EPS: f64 = 1e-3;

pub fn is_non_empty_string(v: &Value) -> bool {
    matches!(v, Value::String(s) if !s.trim().is_empty())
}

/// Console 1 sometimes sends momentary button events (often always `true`) instead of the
/// final state.
///
/// Heuristic: if `incoming` equals `current`, treat it as a toggle rather than a real state
/// report (e.g. a momentary "press" while already muted toggles to unmuted). If `incoming`
/// differs from `current`, it's trusted as-is.
pub fn resolve_next_boolean_from_momentary(current: bool, incoming: bool) -> bool {
    if incoming == current {
        !current
    } else {
        incoming
    }
}

pub fn is_negative_infinity_db(value: &Value) -> bool {
    match value.as_f64() {
        Some(n) if n.is_finite() => n <= MIN_DB_AS_NEGATIVE_INFINITY + DB_NEGATIVE_INFINITY_EPS,
        _ => false,
    }
}

pub fn normalize_console1_level_for_ms(value: &Value) -> Option<f64> {
    if value.as_str() == Some("-Infinity") {
        return Some(MIN_DB_AS_NEGATIVE_INFINITY);
    }
    if let Some(n) = value.as_f64() {
        return Some(if n.is_finite() {
            n
        } else {
            MIN_DB_AS_NEGATIVE_INFINITY
        });
    }
    if let Some(s) = value.as_str() {
        if let Ok(n) = s.parse::<f64>() {
            if n.is_finite() {
                return Some(n);
            }
        }
    }
    None
}

/// For stereo-linked channel names coming from Mixing Station, strip trailing side markers
/// (`L`/`R`/`P` for pan) so a stereo pair collapses to one shared display name on Console 1.
///
/// Handles two shapes:
/// - Marker at the very end: `"Keys L"` -> `"Keys"`, `"Vox R"` -> `"Vox"`, `"GTR P"` -> `"GTR"`.
/// - Marker before a trailing bus label: `"Dr L MixBus"` -> `"Dr MixBus"`,
///   `"Kuba Vox P MB"` -> `"Kuba Vox MB"` (tolerates extra whitespace before the bus label,
///   matching the JS original's `\s+` regex).
///
/// Names with no recognizable marker pass through unchanged.
pub fn trim_stereo_suffix_from_name(name: &str) -> String {
    if name.len() < 2 {
        return name.to_string();
    }
    let n = name.trim_end();

    if let Some(stripped) = n
        .strip_suffix(" L")
        .or_else(|| n.strip_suffix(" R"))
        .or_else(|| n.strip_suffix(" P"))
    {
        return stripped.to_string();
    }

    for tail_word in ["MixBus", "MB"] {
        if let Some(before_tail) = n.strip_suffix(tail_word) {
            // JS regex requires `\s+` (one or more whitespace) between the marker and
            // the tail word — trim it all off here rather than assuming exactly one space.
            let trimmed = before_tail.trim_end();
            if trimmed.len() == before_tail.len() {
                // No whitespace at all before the tail word: not a match.
                continue;
            }
            for marker in ["L", "R", "P"] {
                if let Some(base) = trimmed.strip_suffix(&format!(" {marker}")) {
                    let base = base.trim_end();
                    return if base.is_empty() {
                        tail_word.to_string()
                    } else {
                        format!("{base} {tail_word}")
                    };
                }
            }
        }
    }

    n.to_string()
}

pub fn coerce_console1_numeric_string(value: Value) -> Value {
    if let Value::String(s) = &value {
        if s != "-Infinity" {
            if let Ok(n) = s.parse::<f64>() {
                if n.is_finite() {
                    return json!(n);
                }
            }
        }
    }
    value
}

/// Console 1's DSP-section fields (Filter/EQ/Compressor) arrive `{value: ...}`-wrapped,
/// unlike the bare mixer primitives (`volume`, `mute`, `pan`). Unwraps either shape.
///
/// `field` is `Option<Value>` rather than a bare `Value` specifically to distinguish JS's
/// `undefined` (field absent from the SysEx payload) from `null` (field present, explicitly
/// null) — callers pass `None` for a missing field and `Some(Value::Null)` for a present-but-
/// null one, and those two cases are *not* the same: `None` returns `None` (the standard
/// "field not present" signal handlers guard on), while `Some(Value::Null)` returns
/// `Some(Value::Null)` unchanged.
pub fn read_c1_dsp_value(field: Option<Value>) -> Option<Value> {
    match field {
        None => None,
        Some(Value::Object(mut map)) => map.remove("value").or(Some(Value::Object(map))),
        Some(v) => Some(v),
    }
}

pub fn coerce_ws_payload_to_text(data: &[u8]) -> String {
    String::from_utf8_lossy(data).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_non_empty_string_rejects_non_strings_empty_and_whitespace_only() {
        assert!(is_non_empty_string(&json!("hello")));
        assert!(!is_non_empty_string(&json!("")));
        assert!(!is_non_empty_string(&json!("   ")));
        assert!(!is_non_empty_string(&json!(42)));
        assert!(!is_non_empty_string(&Value::Null));
    }

    #[test]
    fn resolve_next_boolean_from_momentary_equal_current_toggles() {
        assert!(!resolve_next_boolean_from_momentary(true, true));
        assert!(resolve_next_boolean_from_momentary(false, false));
    }

    #[test]
    fn resolve_next_boolean_from_momentary_different_used_as_is() {
        assert!(!resolve_next_boolean_from_momentary(true, false));
        assert!(resolve_next_boolean_from_momentary(false, true));
    }

    #[test]
    fn is_negative_infinity_db_at_or_below_floor_plus_eps_is_true() {
        assert!(is_negative_infinity_db(&json!(MIN_DB_AS_NEGATIVE_INFINITY)));
        assert!(is_negative_infinity_db(&json!(-89.999)));
    }

    #[test]
    fn is_negative_infinity_db_above_floor_is_false() {
        assert!(!is_negative_infinity_db(&json!(-40)));
    }

    #[test]
    fn is_negative_infinity_db_non_numbers_are_false() {
        assert!(!is_negative_infinity_db(&json!("-90")));
        assert!(!is_negative_infinity_db(&Value::Null));
    }

    #[test]
    fn normalize_console1_level_for_ms_infinity_string_maps_to_floor() {
        assert_eq!(
            normalize_console1_level_for_ms(&json!("-Infinity")),
            Some(MIN_DB_AS_NEGATIVE_INFINITY)
        );
    }

    #[test]
    fn normalize_console1_level_for_ms_finite_numbers_pass_through() {
        assert_eq!(normalize_console1_level_for_ms(&json!(-6)), Some(-6.0));
    }

    #[test]
    fn normalize_console1_level_for_ms_numeric_strings_are_parsed() {
        assert_eq!(normalize_console1_level_for_ms(&json!("-6.5")), Some(-6.5));
    }

    #[test]
    fn normalize_console1_level_for_ms_unparseable_returns_none() {
        assert_eq!(
            normalize_console1_level_for_ms(&json!("not a number")),
            None
        );
        assert_eq!(normalize_console1_level_for_ms(&Value::Null), None);
    }

    #[test]
    fn trim_stereo_suffix_from_name_strips_trailing_side_markers() {
        assert_eq!(trim_stereo_suffix_from_name("Keys L"), "Keys");
        assert_eq!(trim_stereo_suffix_from_name("Vox R"), "Vox");
        assert_eq!(trim_stereo_suffix_from_name("GTR P"), "GTR");
    }

    #[test]
    fn trim_stereo_suffix_from_name_strips_marker_before_trailing_bus_label() {
        assert_eq!(trim_stereo_suffix_from_name("Dr L MixBus"), "Dr MixBus");
        assert_eq!(trim_stereo_suffix_from_name("Kuba Vox P MB"), "Kuba Vox MB");
    }

    #[test]
    fn trim_stereo_suffix_from_name_tolerates_multiple_spaces_before_bus_label() {
        assert_eq!(trim_stereo_suffix_from_name("Dr L  MixBus"), "Dr MixBus");
    }

    #[test]
    fn trim_stereo_suffix_from_name_no_marker_passes_through_unchanged() {
        assert_eq!(trim_stereo_suffix_from_name("Lead Vocal"), "Lead Vocal");
    }

    #[test]
    fn coerce_console1_numeric_string_numeric_strings_become_numbers() {
        assert_eq!(coerce_console1_numeric_string(json!("-6.5")), json!(-6.5));
    }

    #[test]
    fn coerce_console1_numeric_string_infinity_string_preserved() {
        assert_eq!(
            coerce_console1_numeric_string(json!("-Infinity")),
            json!("-Infinity")
        );
    }

    #[test]
    fn coerce_console1_numeric_string_non_numeric_passes_through() {
        assert_eq!(coerce_console1_numeric_string(json!("abc")), json!("abc"));
        assert_eq!(coerce_console1_numeric_string(json!(5)), json!(5));
        assert_eq!(coerce_console1_numeric_string(json!(true)), json!(true));
    }

    #[test]
    fn read_c1_dsp_value_unwraps_value_wrapped_field() {
        assert_eq!(
            read_c1_dsp_value(Some(json!({"value": 0.5}))),
            Some(json!(0.5))
        );
    }

    #[test]
    fn read_c1_dsp_value_unwraps_value_wrapped_boolean() {
        assert_eq!(
            read_c1_dsp_value(Some(json!({"value": true}))),
            Some(json!(true))
        );
    }

    #[test]
    fn read_c1_dsp_value_passes_through_bare_value() {
        assert_eq!(read_c1_dsp_value(Some(json!(0.5))), Some(json!(0.5)));
        assert_eq!(read_c1_dsp_value(Some(json!(true))), Some(json!(true)));
    }

    #[test]
    fn read_c1_dsp_value_none_field_returns_none() {
        assert_eq!(read_c1_dsp_value(None), None);
    }

    #[test]
    fn read_c1_dsp_value_null_field_returns_null_not_none() {
        assert_eq!(read_c1_dsp_value(Some(Value::Null)), Some(Value::Null));
    }

    #[test]
    fn coerce_ws_payload_to_text_decodes_utf8_bytes() {
        assert_eq!(coerce_ws_payload_to_text(b"hi"), "hi");
    }
}
