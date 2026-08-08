//! Coalescing queue for Filter/EQ/Compressor "bare field" updates (as opposed to
//! `update_queue::UpdateQueue`'s whole-track `trackBatch` partials). Keyed by
//! `(track_id, field)` since bare updates are field-level. No timer here -- that's an
//! event-loop concern for Plan 2d's caller, matching `update_queue::UpdateQueue`'s split.
//!
//! Port of the coalescing half of `index.js`'s `queueConsole1BareFieldUpdate`, plus the
//! pure payload-building half of the same function and `formatConsole1DspDisplayValue`.

use crate::dsp_field_metadata::{
    dsp_field_metadata, eq_type_index_to_normalized, format_console1_dsp_display_value,
    DspFieldKind,
};
use serde_json::{json, Value};
use std::collections::HashMap;

/// `TrackInfo.dsp_fields` stores bool-kind fields as JSON `Bool`, not `Number` (see
/// `ms_param_apply.rs`'s `set_cache_only` call sites) -- this reads either shape as an
/// `f64`, matching `format_console1_dsp_display_value`'s untyped-`f64` expectation.
fn value_as_f64_loose(v: &Value) -> f64 {
    v.as_f64()
        .or_else(|| v.as_bool().map(|b| if b { 1.0 } else { 0.0 }))
        .unwrap_or(0.0)
}

#[derive(Debug, Clone, PartialEq)]
pub struct BareUpdateEntry {
    pub track_id: String,
    pub field: String,
    pub value: Value,
}

#[derive(Default)]
pub struct BareUpdateQueue {
    entries: HashMap<(String, String), Value>,
}

impl BareUpdateQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn queue(&mut self, track_id: String, field: String, value: Value) {
        self.entries.insert((track_id, field), value);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn take_all(&mut self) -> Vec<BareUpdateEntry> {
        std::mem::take(&mut self.entries)
            .into_iter()
            .map(|((track_id, field), value)| BareUpdateEntry {
                track_id,
                field,
                value,
            })
            .collect()
    }
}

/// Builds the SysEx payload for a single bare field update, e.g. for embedding as
/// `{trackId, [field]: <this payload>}`. `real_val`, if present, is the field's
/// companion `"val"`-format value (from `TrackInfo.dsp_real_values`), used for
/// real-unit `display_value` text.
pub fn build_bare_field_sysex_payload(
    field: &str,
    value: &Value,
    real_val: Option<&Value>,
) -> Value {
    let Some(meta) = dsp_field_metadata(field) else {
        return json!({ "value": value });
    };

    let value_f64 = value_as_f64_loose(value);

    let outbound_value = match meta.kind {
        DspFieldKind::Bool => json!(if value_f64 != 0.0 { 1 } else { 0 }),
        DspFieldKind::EqType => {
            let idx = value_f64.round() as usize;
            json!(eq_type_index_to_normalized(idx))
        }
        _ => value.clone(),
    };

    let real_val_f64 = real_val.map(value_as_f64_loose);
    let display_value = format_console1_dsp_display_value(meta.kind, value_f64, real_val_f64);

    json!({
        "name": meta.name,
        "quantisation": meta.quantisation,
        "value": outbound_value,
        "display_value": display_value,
        "default_value": meta.default_value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn queue_then_take_all_returns_the_entry() {
        let mut q = BareUpdateQueue::new();
        q.queue("trk-1".to_string(), "filterLcOn".to_string(), json!(true));
        let entries = q.take_all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].track_id, "trk-1");
        assert_eq!(entries[0].field, "filterLcOn");
        assert_eq!(entries[0].value, json!(true));
        assert!(q.is_empty());
    }

    #[test]
    fn later_queue_for_same_track_and_field_overwrites_earlier_value() {
        let mut q = BareUpdateQueue::new();
        q.queue("trk-1".to_string(), "eq1Freq".to_string(), json!(0.2));
        q.queue("trk-1".to_string(), "eq1Freq".to_string(), json!(0.7));
        let entries = q.take_all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].value, json!(0.7));
    }

    #[test]
    fn different_fields_on_same_track_stay_separate() {
        let mut q = BareUpdateQueue::new();
        q.queue("trk-1".to_string(), "eq1Freq".to_string(), json!(0.2));
        q.queue("trk-1".to_string(), "eq1Gain".to_string(), json!(0.5));
        let entries = q.take_all();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn take_all_on_empty_queue_returns_empty_vec() {
        let mut q = BareUpdateQueue::new();
        assert!(q.take_all().is_empty());
    }

    #[test]
    fn bool_kind_field_sends_value_as_zero_or_one_not_json_bool() {
        let payload = build_bare_field_sysex_payload("filterLcOn", &json!(true), None);
        assert_eq!(payload["value"], json!(1));
    }

    #[test]
    fn bool_kind_field_false_sends_zero() {
        let payload = build_bare_field_sysex_payload("compOn", &json!(false), None);
        assert_eq!(payload["value"], json!(0));
    }

    #[test]
    fn eqtype_kind_field_normalizes_the_index() {
        let payload = build_bare_field_sysex_payload("eq1Type", &json!(2.0), None);
        let expected = crate::dsp_field_metadata::eq_type_index_to_normalized(2);
        assert_eq!(payload["value"], json!(expected));
    }

    #[test]
    fn continuous_field_sends_value_as_is_with_full_metadata() {
        let payload = build_bare_field_sysex_payload("filterLcFreq", &json!(0.4), None);
        assert_eq!(payload["value"], json!(0.4));
        assert!(payload.get("name").is_some());
        assert!(payload.get("quantisation").is_some());
        assert!(payload.get("display_value").is_some());
        assert!(payload.get("default_value").is_some());
    }

    #[test]
    fn display_value_uses_real_val_when_provided() {
        let payload =
            build_bare_field_sysex_payload("filterLcFreq", &json!(0.4), Some(&json!(120.0)));
        assert_eq!(payload["display_value"], json!("120.0 Hz"));
    }

    #[test]
    fn unknown_field_falls_back_to_bare_value_payload() {
        let payload = build_bare_field_sysex_payload("totallyUnknownField", &json!(5), None);
        assert_eq!(payload, json!({ "value": 5 }));
    }
}
