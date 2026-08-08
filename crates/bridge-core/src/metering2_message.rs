//! Normalizes Mixing Station metering2 push payloads (which arrive in several observed
//! shapes -- see `normalize_metering2_values`'s doc comment) into a per-param dB list,
//! and tracks per-channel dB state to detect meaningful changes (0.05dB threshold).
//!
//! Port of `index.js`'s `handleMeteringMessage` (shape-normalization half) and
//! `computeSlotMeterNorm`/`msChannelMeterDb`.

use crate::metering_utils::decode_metering2_binary;
use crate::track_layout::LayoutSlot;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const METER_CHANGE_THRESHOLD_DB: f64 = 0.05;

/// Fixed metering2 subscription ID this bridge always uses — matches `index.js`'s
/// `METERING2_SUBSCRIPTION_ID`.
pub const METERING2_SUBSCRIPTION_ID: i64 = 0;
/// This bridge always requests JSON (not binary) metering2 payloads — matches `index.js`'s
/// `METERING2_BINARY`. (Binary payloads ARE still handled on the inbound side, in case Mixing
/// Station sends one anyway — see `normalize_metering2_values`'s `b` branch.)
const METERING2_BINARY: bool = false;

/// Builds the Mixing Station metering2 subscribe request body from the currently metered
/// object IDs and the live track layout. Returns the sorted, deduplicated MS channel list
/// alongside the JSON request -- the caller must store the channel list as
/// `metering2_param_ms_channels` on `BridgeState`, since inbound metering2 pushes are
/// positional against it (index 0 of an inbound `v` list corresponds to this list's first
/// channel, and so on).
///
/// Matches `updateMetering2Subscription` (`index.js:954-983`): a metered object ID with no
/// matching layout slot is silently skipped (not an error), and an empty `metered_object_ids`
/// produces an empty `params` list -- this is how a subscription gets cleared, not a special
/// no-op case the caller needs to detect separately.
pub fn build_metering2_subscribe_body(
    metered_object_ids: &HashSet<usize>,
    layout: &[LayoutSlot],
    interval_ms: u32,
) -> (Vec<i64>, Value) {
    let mut channels: Vec<i64> = metered_object_ids
        .iter()
        .filter_map(|&object_id| layout.get(object_id))
        .flat_map(|slot| slot.ms_channels.iter().map(|&ch| ch as i64))
        .collect();
    channels.sort_unstable();
    channels.dedup();

    let params: Vec<Value> = channels
        .iter()
        .map(|&ch| json!({"type": 0, "index": ch}))
        .collect();

    let body = json!({
        "path": "/console/metering2/subscribe",
        "method": "POST",
        "body": {
            "id": METERING2_SUBSCRIPTION_ID,
            "interval": interval_ms,
            "binary": METERING2_BINARY,
            "params": params,
        }
    });

    (channels, body)
}

/// Given a set of MS channels whose dB value just changed, returns the object IDs that are
/// both mapped to one of those channels (via `object_ids_by_ms_channel`, `BridgeState`'s
/// existing reverse map) AND currently in the metered set. Matches
/// `applyMeterUpdatesForMsChannels`'s (`index.js:989-1011`) affected-object-ID computation.
///
/// Negative channel numbers (which can't be real MS channel indices) are silently ignored --
/// without this guard, `as usize` would silently wrap a negative value into a huge, wrong
/// index rather than panicking, so the check is about correctness, not crash-safety.
pub fn affected_metered_object_ids(
    changed_ms_channels: &[i64],
    object_ids_by_ms_channel: &HashMap<usize, Vec<usize>>,
    metered_object_ids: &HashSet<usize>,
) -> HashSet<usize> {
    let mut affected = HashSet::new();
    for &ch in changed_ms_channels {
        if ch < 0 {
            continue;
        }
        if let Some(object_ids) = object_ids_by_ms_channel.get(&(ch as usize)) {
            for &object_id in object_ids {
                if metered_object_ids.contains(&object_id) {
                    affected.insert(object_id);
                }
            }
        }
    }
    affected
}

/// Extracts a per-param max-dB list from a metering2 push message body.
///
/// Handles all shapes documented in the original JS (`index.js`'s `handleMeteringMessage`
/// doc comment): nested per-param arrays, per-param scalar lists, flat multi-value-per-param
/// lists, single scalar, and base64-encoded binary (`body.b`, decoded via the existing
/// `decode_metering2_binary`). Returns `None` if neither `v` nor `b` is present.
///
/// **The returned length is not guaranteed to equal `param_count.`** It reflects however many
/// per-param groups the payload actually contained: the nested-array branch returns one entry
/// per inner array (which a malformed or over-long payload can make longer than `param_count`),
/// and the single-list fallback and scalar branches always return exactly one entry regardless
/// of `param_count`. Only the exact-match, chunked, and binary branches return `param_count`
/// entries. Callers pairing this against a channel list must therefore use `.zip()` (which
/// stops at the shorter side) or an explicit bounds check -- never index the channel list by
/// this `Vec`'s indices assuming equal lengths.
///
/// This mirrors the JS end-to-end: `index.js:1078` clamps with
/// `Math.min(vNormalized.length, metering2ParamMsChannels.length)`, but its change-detection
/// loop (`index.js:1105`) re-applies the same bound, so a `.zip()` at the call site reproduces
/// JS's observable behavior exactly.
///
/// Entries may be `f64::NEG_INFINITY` when a param's value list held nothing finite (JS's
/// `let maxDb = -Infinity` never updated); callers must skip non-finite entries, which
/// [`MeterDbCache::apply_updates`] already does.
///
/// # Preconditions
///
/// `param_count` must be `> 0`; `Some` is only meaningful for a non-empty subscription. A
/// `param_count` of `0` returns `None`, consistent with `decode_metering2_binary`'s own
/// `param_count <= 0 -> None` contract in `metering_utils.rs` (which this function's `b`
/// branch already delegated that case to). The real caller guards on an empty channel list
/// before calling, exactly as JS does at `index.js:1033`.
pub fn normalize_metering2_values(body: &Value, param_count: usize) -> Option<Vec<f64>> {
    if param_count == 0 {
        return None;
    }
    if let Some(raw_v) = body.get("v") {
        let v_normalized: Vec<Vec<Value>> = if let Some(arr) = raw_v.as_array() {
            if arr.first().map(|x| x.is_array()).unwrap_or(false) {
                arr.iter()
                    .map(|x| x.as_array().cloned().unwrap_or_default())
                    .collect()
            } else if arr.len() == param_count {
                arr.iter().map(|x| vec![x.clone()]).collect()
            } else if param_count > 0 && arr.len() > param_count && arr.len() % param_count == 0 {
                let values_per_param = arr.len() / param_count;
                (0..param_count)
                    .map(|i| arr[i * values_per_param..(i + 1) * values_per_param].to_vec())
                    .collect()
            } else {
                vec![arr.clone()]
            }
        } else {
            vec![vec![raw_v.clone()]]
        };

        Some(
            v_normalized
                .iter()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|x| x.as_f64().filter(|n| n.is_finite()))
                        .fold(f64::NEG_INFINITY, f64::max)
                })
                .collect(),
        )
    } else if let Some(b) = body.get("b").and_then(|v| v.as_str()) {
        let decoded = decode_metering2_binary(b, param_count as i64)?;
        Some(decoded.max_db_by_param)
    } else {
        None
    }
}

#[derive(Default)]
pub struct MeterDbCache {
    values: HashMap<i64, f64>,
}

impl MeterDbCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, ms_channel: i64) -> Option<f64> {
        self.values.get(&ms_channel).copied()
    }

    /// Applies `(ms_channel, max_db)` updates, returning the channels whose recorded dB
    /// changed by more than the threshold (or were previously unrecorded). Non-finite
    /// updates are skipped entirely -- never recorded, never reported as changed.
    pub fn apply_updates(&mut self, updates: &[(i64, f64)]) -> Vec<i64> {
        let mut changed = Vec::new();
        for &(ms_channel, max_db) in updates {
            if !max_db.is_finite() {
                continue;
            }
            let prev = self.values.get(&ms_channel).copied();
            if prev.is_none_or(|p| (p - max_db).abs() > METER_CHANGE_THRESHOLD_DB) {
                self.values.insert(ms_channel, max_db);
                changed.push(ms_channel);
            }
        }
        changed
    }
}

/// Console 1 meter-norm for a layout slot spanning one or more MS channels: the max dB
/// across all of the slot's `ms_channels`, converted via `db_to_console1_meter_norm`.
/// Returns 0.0 if the slot has no channels or none have a recorded dB yet.
pub fn compute_slot_meter_norm(ms_channels: &[i64], cache: &MeterDbCache) -> f64 {
    let max_db = ms_channels
        .iter()
        .filter_map(|ch| cache.get(*ch))
        .fold(f64::NEG_INFINITY, f64::max);
    if !max_db.is_finite() {
        return 0.0;
    }
    crate::metering_utils::db_to_console1_meter_norm(max_db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nested_array_shape_used_as_is() {
        let body = json!({ "v": [[-20.0], [-18.0, -19.0]] });
        let result = normalize_metering2_values(&body, 2).unwrap();
        assert_eq!(result, vec![-20.0, -18.0]); // max of each inner list
    }

    #[test]
    fn per_param_scalar_list_is_wrapped() {
        let body = json!({ "v": [-20.0, -25.0, -30.0] });
        let result = normalize_metering2_values(&body, 3).unwrap();
        assert_eq!(result, vec![-20.0, -25.0, -30.0]);
    }

    #[test]
    fn flat_multi_value_per_param_is_chunked() {
        // 3 params, 2 values each -> length 6.
        let body = json!({ "v": [-20.0, -21.0, -25.0, -26.0, -30.0, -31.0] });
        let result = normalize_metering2_values(&body, 3).unwrap();
        assert_eq!(result, vec![-20.0, -25.0, -30.0]); // max of each pair
    }

    #[test]
    fn single_param_multi_value_falls_back_to_one_list() {
        // length doesn't match param_count and isn't a clean multiple -> whole array is
        // treated as one param's values.
        let body = json!({ "v": [-20.0, -20.0] });
        let result = normalize_metering2_values(&body, 3).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], -20.0);
    }

    #[test]
    fn scalar_v_becomes_single_param_single_value() {
        let body = json!({ "v": -90.0 });
        let result = normalize_metering2_values(&body, 1).unwrap();
        assert_eq!(result, vec![-90.0]);
    }

    #[test]
    fn non_finite_values_in_a_params_list_are_skipped_from_the_max() {
        let body = json!({ "v": [["not a number", -20.0]] });
        let result = normalize_metering2_values(&body, 1).unwrap();
        assert_eq!(result, vec![-20.0]);
    }

    #[test]
    fn param_with_no_finite_values_yields_negative_infinity() {
        let body = json!({ "v": [["not a number"]] });
        let result = normalize_metering2_values(&body, 1).unwrap();
        assert_eq!(result[0], f64::NEG_INFINITY);
    }

    #[test]
    fn missing_v_and_b_yields_none() {
        let body = json!({});
        assert!(normalize_metering2_values(&body, 1).is_none());
    }

    #[test]
    fn binary_payload_decodes_via_existing_decoder() {
        // decode_metering2_binary (metering_utils.rs) packs big-endian int16s scaled by
        // 100 (a reading of -18.00dB is the wire integer -1800), base64-encoded with
        // padding stripped -- confirmed against metering_utils.rs's own
        // `single_param_single_value_1_02db_encoded_as_102` test fixture convention.
        // Two values for one param -> decode_metering2_binary keeps the max (-18.0).
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let mut buf = Vec::new();
        buf.extend_from_slice(&(-2000i16).to_be_bytes()); // -20.00dB
        buf.extend_from_slice(&(-1800i16).to_be_bytes()); // -18.00dB (max)
        let encoded = STANDARD.encode(&buf).trim_end_matches('=').to_string();
        let body = json!({ "b": encoded });
        let result = normalize_metering2_values(&body, 1).unwrap();
        assert_eq!(result, vec![-18.0]);
    }

    #[test]
    fn zero_param_count_yields_none_for_every_payload_shape() {
        // Matches decode_metering2_binary's `param_count <= 0 -> None` contract, and makes the
        // `v` and `b` branches agree -- the `b` branch already returned None via delegation.
        assert!(normalize_metering2_values(&json!({ "v": [-20.0, -20.0] }), 0).is_none());
        assert!(normalize_metering2_values(&json!({ "v": [[-20.0]] }), 0).is_none());
        assert!(normalize_metering2_values(&json!({ "v": -90.0 }), 0).is_none());
        assert!(normalize_metering2_values(&json!({ "b": "AAA" }), 0).is_none());
    }

    #[test]
    fn meter_db_cache_reports_first_write_as_changed() {
        let mut cache = MeterDbCache::new();
        let changed = cache.apply_updates(&[(5, -20.0)]);
        assert_eq!(changed, vec![5]);
        assert_eq!(cache.get(5), Some(-20.0));
    }

    #[test]
    fn meter_db_cache_ignores_small_deltas() {
        let mut cache = MeterDbCache::new();
        cache.apply_updates(&[(5, -20.0)]);
        let changed = cache.apply_updates(&[(5, -20.03)]);
        assert!(changed.is_empty());
    }

    #[test]
    fn meter_db_cache_reports_deltas_over_threshold() {
        let mut cache = MeterDbCache::new();
        cache.apply_updates(&[(5, -20.0)]);
        let changed = cache.apply_updates(&[(5, -19.9)]);
        assert_eq!(changed, vec![5]);
    }

    #[test]
    fn meter_db_cache_skips_non_finite_updates() {
        let mut cache = MeterDbCache::new();
        let changed = cache.apply_updates(&[(5, f64::NEG_INFINITY)]);
        assert!(changed.is_empty());
        assert_eq!(cache.get(5), None);
    }

    #[test]
    fn compute_slot_meter_norm_takes_max_across_slot_ms_channels() {
        let mut cache = MeterDbCache::new();
        cache.apply_updates(&[(10, -20.0), (11, -6.0)]);
        let norm = compute_slot_meter_norm(&[10, 11], &cache);
        assert_eq!(norm, crate::metering_utils::db_to_console1_meter_norm(-6.0));
    }

    #[test]
    fn compute_slot_meter_norm_is_zero_for_empty_or_unknown_channels() {
        let cache = MeterDbCache::new();
        assert_eq!(compute_slot_meter_norm(&[], &cache), 0.0);
        assert_eq!(compute_slot_meter_norm(&[99], &cache), 0.0);
    }

    fn test_slot(object_id: usize, ms_channels: Vec<usize>) -> LayoutSlot {
        LayoutSlot {
            object_id,
            kind: crate::track_layout::LayoutSlotKind::Input,
            ms_channels,
            ms_primary: None,
            pan_locked: false,
        }
    }

    #[test]
    fn build_metering2_subscribe_body_empty_set_yields_empty_params() {
        let (channels, body) = build_metering2_subscribe_body(&HashSet::new(), &[], 100);
        assert!(channels.is_empty());
        assert_eq!(body["body"]["params"], json!([]));
    }

    #[test]
    fn build_metering2_subscribe_body_maps_object_ids_to_sorted_deduped_channels() {
        let layout = vec![
            test_slot(0, vec![5]),
            test_slot(1, vec![2, 3]),
            test_slot(2, vec![3]), // duplicate channel 3, must be deduped
        ];
        let metered: HashSet<usize> = [0, 1, 2].into_iter().collect();
        let (channels, body) = build_metering2_subscribe_body(&metered, &layout, 250);
        assert_eq!(channels, vec![2, 3, 5]);
        assert_eq!(body["body"]["interval"], json!(250));
        assert_eq!(body["body"]["id"], json!(0));
        assert_eq!(body["body"]["binary"], json!(false));
        assert_eq!(
            body["body"]["params"],
            json!([{"type": 0, "index": 2}, {"type": 0, "index": 3}, {"type": 0, "index": 5}])
        );
    }

    #[test]
    fn build_metering2_subscribe_body_skips_object_ids_with_no_layout_slot() {
        let layout = vec![test_slot(0, vec![1])];
        let metered: HashSet<usize> = [0, 99].into_iter().collect(); // 99 has no slot
        let (channels, _body) = build_metering2_subscribe_body(&metered, &layout, 100);
        assert_eq!(channels, vec![1]);
    }

    #[test]
    fn build_metering2_subscribe_body_has_correct_path_and_method() {
        let (_channels, body) = build_metering2_subscribe_body(&HashSet::new(), &[], 100);
        assert_eq!(body["path"], json!("/console/metering2/subscribe"));
        assert_eq!(body["method"], json!("POST"));
    }

    #[test]
    fn affected_metered_object_ids_intersects_channel_map_with_metered_set() {
        let mut map: HashMap<usize, Vec<usize>> = HashMap::new();
        map.insert(5, vec![10, 11]);
        map.insert(6, vec![12]);
        let metered: HashSet<usize> = [10, 12].into_iter().collect(); // 11 is NOT metered
        let affected = affected_metered_object_ids(&[5, 6], &map, &metered);
        assert_eq!(affected, [10, 12].into_iter().collect::<HashSet<usize>>());
    }

    #[test]
    fn affected_metered_object_ids_ignores_channels_with_no_mapped_objects() {
        let map: HashMap<usize, Vec<usize>> = HashMap::new();
        let metered: HashSet<usize> = [10].into_iter().collect();
        let affected = affected_metered_object_ids(&[5], &map, &metered);
        assert!(affected.is_empty());
    }

    #[test]
    fn affected_metered_object_ids_ignores_negative_channels() {
        let mut map: HashMap<usize, Vec<usize>> = HashMap::new();
        map.insert(5, vec![10]);
        let metered: HashSet<usize> = [10].into_iter().collect();
        let affected = affected_metered_object_ids(&[-1, 5], &map, &metered);
        assert_eq!(affected, [10].into_iter().collect::<HashSet<usize>>());
    }
}
