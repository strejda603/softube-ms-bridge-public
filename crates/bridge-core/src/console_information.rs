//! Best-effort parsing of Mixing Station's `/console/information` response into updated
//! channel-architecture fields. Falls back to whatever was already known on any
//! unmatched/invalid field — never errors.
//!
//! Port of `index.js`'s `applyConsoleInformation`.

use serde_json::Value;

/// The channel-layout facts the bridge derives from Mixing Station, either discovered from a
/// `/console/information` response or left at whatever was previously known.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsoleArchitecture {
    /// Total channel count reported by the console.
    pub total_channels: i64,
    /// Number of input channels. The bridge assumes inputs are indexed from 0.
    pub input_channel_count: i64,
    /// Channel index at which the bus range begins.
    pub bus_channel_start: i64,
    /// Number of bus channels starting at [`Self::bus_channel_start`].
    pub bus_channel_count: i64,
    /// Channel indices making up the main output — a stereo pair, or one index if mono.
    pub main_stereo_channels: Vec<i64>,
}

#[derive(Debug)]
struct ChannelType {
    name: String,
    short_name: String,
    offset: i64,
    count: i64,
}

fn parse_channel_types(info: &Value) -> Vec<ChannelType> {
    let Some(types) = info.get("channelTypes").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    types
        .iter()
        .filter_map(|t| {
            let offset = t.get("offset").and_then(as_finite_i64)?;
            let count = t.get("count").and_then(as_finite_i64)?;
            if count <= 0 {
                return None;
            }
            Some(ChannelType {
                name: t
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                short_name: t
                    .get("shortName")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                offset,
                count,
            })
        })
        .collect()
}

/// Mirrors JS `Number(v)` followed by `Number.isFinite`: accepts JSON numbers and numeric
/// strings alike, rejecting anything non-finite or unparseable.
fn as_finite_i64(v: &Value) -> Option<i64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
        .filter(|n| n.is_finite())
        .map(|n| n as i64)
}

fn name_matches(ct: &ChannelType, is_match: impl Fn(&str) -> bool) -> bool {
    is_match(&ct.name.to_lowercase()) || is_match(&ct.short_name.to_lowercase())
}

/// Whole-word match equivalent to a JS `\bword\b` regex. `_` counts as a word character in
/// JS, so it does not act as a boundary here either — `"Bus_2"` does not contain the word
/// `"bus"`.
fn contains_word(haystack: &str, word: &str) -> bool {
    haystack
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|token| token == word)
}

/// Derives an updated [`ConsoleArchitecture`] from a `/console/information` response.
///
/// Best-effort and infallible: any field that is absent, malformed, or fails the heuristic
/// keeps its value from `current`.
pub fn apply_console_information(
    info: &Value,
    current: ConsoleArchitecture,
) -> ConsoleArchitecture {
    let mut result = current;
    if !info.is_object() {
        return result;
    }

    if let Some(total) = info.get("totalChannels").and_then(as_finite_i64) {
        if total > 0 {
            result.total_channels = total;
        }
    }

    let types = parse_channel_types(info);

    // Inputs: candidates matching "input" or "ch" as a whole word, sorted by offset ascending;
    // prefer offset==0, else the lowest-offset candidate. Only applied if the chosen
    // candidate's offset is exactly 0.
    let mut input_candidates: Vec<&ChannelType> = types
        .iter()
        .filter(|ct| name_matches(ct, |s| contains_word(s, "input") || contains_word(s, "ch")))
        .collect();
    input_candidates.sort_by_key(|ct| ct.offset);
    let input_type = input_candidates
        .iter()
        .find(|ct| ct.offset == 0)
        .or_else(|| input_candidates.first());
    if let Some(ct) = input_type {
        if ct.offset == 0 {
            result.input_channel_count = ct.count;
        }
    }

    // Bus: first matching entry in original order; offset and count apply independently.
    if let Some(ct) = types
        .iter()
        .find(|ct| name_matches(ct, |s| contains_word(s, "bus")))
    {
        if ct.offset >= 0 {
            result.bus_channel_start = ct.offset;
        }
        if ct.count > 0 {
            result.bus_channel_count = ct.count;
        }
    }

    // Main: first matching entry; offset and count must BOTH be valid to apply either.
    if let Some(ct) = types
        .iter()
        .find(|ct| name_matches(ct, |s| contains_word(s, "main")))
    {
        if ct.offset >= 0 && ct.count > 0 {
            result.main_stereo_channels = if ct.count >= 2 {
                vec![ct.offset, ct.offset + 1]
            } else {
                vec![ct.offset]
            };
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn defaults() -> ConsoleArchitecture {
        ConsoleArchitecture {
            total_channels: 80,
            input_channel_count: 32,
            bus_channel_start: 48,
            bus_channel_count: 16,
            main_stereo_channels: vec![70, 71],
        }
    }

    #[test]
    fn null_info_is_a_no_op() {
        let result = apply_console_information(&Value::Null, defaults());
        assert_eq!(result, defaults());
    }

    #[test]
    fn non_object_info_is_a_no_op() {
        let result = apply_console_information(&json!("not an object"), defaults());
        assert_eq!(result, defaults());
    }

    #[test]
    fn applies_total_channels_when_positive_finite() {
        let info = json!({ "totalChannels": 64 });
        let result = apply_console_information(&info, defaults());
        assert_eq!(result.total_channels, 64);
    }

    #[test]
    fn ignores_non_positive_total_channels() {
        let info = json!({ "totalChannels": 0 });
        let result = apply_console_information(&info, defaults());
        assert_eq!(result.total_channels, 80);
    }

    #[test]
    fn applies_input_type_with_zero_offset() {
        let info = json!({
            "channelTypes": [
                { "name": "Input", "offset": 0, "count": 24 },
                { "name": "Bus", "offset": 24, "count": 8 }
            ]
        });
        let result = apply_console_information(&info, defaults());
        assert_eq!(result.input_channel_count, 24);
    }

    #[test]
    fn discards_input_type_with_nonzero_offset() {
        // Matches the "input"/"ch" pattern but offset != 0 -> must NOT apply, even though
        // it's the only/lowest-offset candidate.
        let info = json!({
            "channelTypes": [
                { "name": "Channel", "offset": 4, "count": 20 }
            ]
        });
        let result = apply_console_information(&info, defaults());
        assert_eq!(result.input_channel_count, 32);
    }

    #[test]
    fn prefers_zero_offset_input_candidate_over_lower_sorted_alternative() {
        let info = json!({
            "channelTypes": [
                { "name": "Ch Group B", "offset": 40, "count": 4 },
                { "name": "Ch Group A", "offset": 0, "count": 16 }
            ]
        });
        let result = apply_console_information(&info, defaults());
        assert_eq!(result.input_channel_count, 16);
    }

    #[test]
    fn bus_offset_and_count_apply_independently() {
        // Invalid (negative) offset, valid count: count still applies, offset doesn't.
        let info = json!({
            "channelTypes": [
                { "name": "Bus", "offset": -1, "count": 12 }
            ]
        });
        let result = apply_console_information(&info, defaults());
        assert_eq!(result.bus_channel_start, 48); // unchanged
        assert_eq!(result.bus_channel_count, 12);
    }

    #[test]
    fn bus_with_non_positive_count_is_not_selected_at_all() {
        // JS `findType` requires count > 0 to select a candidate, so a zero count discards
        // the whole entry — the otherwise-valid offset is not applied either.
        let info = json!({
            "channelTypes": [
                { "name": "Bus", "offset": 50, "count": 0 }
            ]
        });
        let result = apply_console_information(&info, defaults());
        assert_eq!(result, defaults());
    }

    #[test]
    fn main_requires_both_offset_and_count_valid_to_apply_either() {
        let info = json!({
            "channelTypes": [
                { "name": "Main", "offset": 72, "count": 0 }
            ]
        });
        let result = apply_console_information(&info, defaults());
        // count invalid -> neither offset nor count-derived field applied.
        assert_eq!(result.main_stereo_channels, vec![70, 71]);
    }

    #[test]
    fn main_with_count_two_or_more_becomes_stereo_pair() {
        let info = json!({
            "channelTypes": [
                { "name": "Main", "offset": 72, "count": 2 }
            ]
        });
        let result = apply_console_information(&info, defaults());
        assert_eq!(result.main_stereo_channels, vec![72, 73]);
    }

    #[test]
    fn main_with_count_one_becomes_single_channel() {
        let info = json!({
            "channelTypes": [
                { "name": "Main", "offset": 72, "count": 1 }
            ]
        });
        let result = apply_console_information(&info, defaults());
        assert_eq!(result.main_stereo_channels, vec![72]);
    }

    #[test]
    fn shortname_is_also_matched() {
        let info = json!({
            "channelTypes": [
                { "shortName": "BUS", "offset": 48, "count": 12 }
            ]
        });
        let result = apply_console_information(&info, defaults());
        assert_eq!(result.bus_channel_count, 12);
    }

    #[test]
    fn underscore_is_a_word_character_so_bus_2_does_not_match() {
        // JS `\bbus\b` does not match "Bus_2" — `_` is a word character, so there is no
        // word boundary after "Bus".
        let info = json!({
            "channelTypes": [
                { "name": "Bus_2", "offset": 50, "count": 12 }
            ]
        });
        let result = apply_console_information(&info, defaults());
        assert_eq!(result, defaults());
    }

    #[test]
    fn numeric_strings_are_coerced_like_js_number() {
        // JS `Number("64")` === 64, so a stringified number is accepted, not dropped.
        let info = json!({
            "totalChannels": "64",
            "channelTypes": [
                { "name": "Bus", "offset": "50", "count": "12" }
            ]
        });
        let result = apply_console_information(&info, defaults());
        assert_eq!(result.total_channels, 64);
        assert_eq!(result.bus_channel_start, 50);
        assert_eq!(result.bus_channel_count, 12);
    }

    #[test]
    fn non_matching_types_are_ignored() {
        let info = json!({
            "channelTypes": [
                { "name": "Aux", "offset": 100, "count": 5 }
            ]
        });
        let result = apply_console_information(&info, defaults());
        assert_eq!(result, defaults());
    }
}
