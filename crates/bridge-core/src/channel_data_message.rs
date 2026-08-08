//! Parses inbound `/console/data/get/ch.<n>.<param>/<format>` Mixing Station WebSocket
//! messages. Pure — takes an already-JSON-parsed `Value`, returns a structured result.
//!
//! Port of `index.js`'s `parseChannelDataGetMessage`.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsFormat {
    Val,
    Norm,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedChannelDataMessage {
    pub channel_index: i64,
    pub param_path: String,
    pub format: MsFormat,
    pub value: Value,
}

const PREFIX: &str = "/console/data/get/";

pub fn parse_channel_data_get_message(msg: &Value) -> Option<ParsedChannelDataMessage> {
    let path = msg.get("path")?.as_str()?;
    let rest = path.strip_prefix(PREFIX)?;

    let mut parts = rest.splitn(2, '/');
    let path_part = parts.next().unwrap_or("");
    let format_part = parts.next();
    let format = if format_part == Some("norm") {
        MsFormat::Norm
    } else {
        MsFormat::Val
    };

    let after_ch = path_part.strip_prefix("ch.")?;
    let dot_idx = after_ch.find('.')?;
    let (channel_str, param_path) = after_ch.split_at(dot_idx);
    let param_path = &param_path[1..]; // drop the leading '.'
    if param_path.is_empty() {
        return None;
    }
    // JS's `\d+` accepts ASCII digits only; `i64::parse` would also accept a leading +/-.
    if channel_str.is_empty() || !channel_str.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let channel_index: i64 = channel_str.parse().ok()?;

    let body_value = msg
        .get("body")
        .and_then(|b| b.get("value"))
        .filter(|v| !v.is_null());
    let top_level_value = msg.get("value").filter(|v| !v.is_null());
    let value = body_value
        .or(top_level_value)
        .cloned()
        .unwrap_or(Value::Null);

    Some(ParsedChannelDataMessage {
        channel_index,
        param_path: param_path.to_string(),
        format,
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_val_format_with_body_value() {
        let msg = json!({
            "path": "/console/data/get/ch.5.mix.lvl/val",
            "body": { "value": -6.0 }
        });
        let parsed = parse_channel_data_get_message(&msg).expect("should parse");
        assert_eq!(parsed.channel_index, 5);
        assert_eq!(parsed.param_path, "mix.lvl");
        assert_eq!(parsed.format, MsFormat::Val);
        assert_eq!(parsed.value, json!(-6.0));
    }

    #[test]
    fn parses_norm_format() {
        let msg = json!({
            "path": "/console/data/get/ch.0.mix.pan/norm",
            "body": { "value": 0.5 }
        });
        let parsed = parse_channel_data_get_message(&msg).expect("should parse");
        assert_eq!(parsed.format, MsFormat::Norm);
    }

    #[test]
    fn missing_or_unrecognized_format_segment_defaults_to_val() {
        let msg = json!({
            "path": "/console/data/get/ch.0.mix.on/garbage",
            "body": { "value": true }
        });
        let parsed = parse_channel_data_get_message(&msg).expect("should parse");
        assert_eq!(parsed.format, MsFormat::Val);
    }

    #[test]
    fn multi_segment_param_path_is_preserved_whole() {
        let msg = json!({
            "path": "/console/data/get/ch.2.peq.bands.0.freq/norm",
            "body": { "value": 0.3 }
        });
        let parsed = parse_channel_data_get_message(&msg).expect("should parse");
        assert_eq!(parsed.param_path, "peq.bands.0.freq");
    }

    #[test]
    fn falls_back_to_top_level_value_when_body_value_absent() {
        let msg = json!({
            "path": "/console/data/get/ch.1.solo/val",
            "value": true
        });
        let parsed = parse_channel_data_get_message(&msg).expect("should parse");
        assert_eq!(parsed.value, json!(true));
    }

    #[test]
    fn falls_back_to_top_level_value_when_body_value_is_explicitly_null() {
        // JS `??` treats an explicit `null` in body.value as "use the fallback" too.
        let msg = json!({
            "path": "/console/data/get/ch.1.solo/val",
            "body": { "value": null },
            "value": true
        });
        let parsed = parse_channel_data_get_message(&msg).expect("should parse");
        assert_eq!(parsed.value, json!(true));
    }

    #[test]
    fn zero_and_false_body_values_are_not_treated_as_absent() {
        let msg = json!({
            "path": "/console/data/get/ch.1.mix.lvl/val",
            "body": { "value": 0 }
        });
        let parsed = parse_channel_data_get_message(&msg).expect("should parse");
        assert_eq!(parsed.value, json!(0));
    }

    #[test]
    fn value_is_null_when_neither_body_value_nor_top_level_value_present() {
        let msg = json!({ "path": "/console/data/get/ch.1.mix.lvl/val" });
        let parsed = parse_channel_data_get_message(&msg).expect("should parse");
        assert_eq!(parsed.value, Value::Null);
    }

    #[test]
    fn rejects_paths_not_matching_console_data_get_ch_prefix() {
        let msg = json!({ "path": "/console/information", "body": {} });
        assert!(parse_channel_data_get_message(&msg).is_none());
    }

    #[test]
    fn rejects_missing_path() {
        let msg = json!({ "body": { "value": 1 } });
        assert!(parse_channel_data_get_message(&msg).is_none());
    }

    #[test]
    fn rejects_non_numeric_channel_index() {
        let msg = json!({ "path": "/console/data/get/ch.x.mix.lvl/val" });
        assert!(parse_channel_data_get_message(&msg).is_none());
    }

    #[test]
    fn rejects_signed_channel_index() {
        // JS's `^ch\.(\d+)\.` matches ASCII digits only, so a sign makes the whole regex
        // fail and the message is skipped. Rust's `i64::parse` would accept these.
        for path in [
            "/console/data/get/ch.-1.mix.lvl/val",
            "/console/data/get/ch.+5.mix.lvl/val",
        ] {
            let msg = json!({ "path": path, "body": { "value": 1 } });
            assert!(
                parse_channel_data_get_message(&msg).is_none(),
                "expected {path} to be rejected"
            );
        }
    }

    #[test]
    fn rejects_empty_channel_index() {
        let msg = json!({ "path": "/console/data/get/ch..mix.lvl/val" });
        assert!(parse_channel_data_get_message(&msg).is_none());
    }
}
