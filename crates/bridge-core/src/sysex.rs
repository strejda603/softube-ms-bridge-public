//! SysEx JSON framing for the Console 1 <-> bridge protocol.
//!
//! Console 1 SysEx frames are `0xF0 0x7D "stc1" <JSON bytes> 0xF7` — the entire payload is a
//! JSON object encoded byte-for-byte, restricted to printable ASCII (0x20-0x7E); anything else
//! must be JSON `\uXXXX`-escaped (with UTF-16 surrogate pairs above the Basic Multilingual
//! Plane) since that's what the wire format's own escape syntax requires. See `index.js`'s
//! `sendSysexToConsole1`/`parseSysexJson` for the original implementation this ports.

use serde::Serialize;
use serde_json::Value;

pub const SYSEX_START: u8 = 0xf0;
pub const SYSEX_STOP: u8 = 0xf7;
pub const SYSEX_MANUFACTURER: u8 = 0x7d;
pub const SYSEX_MAGIC: &[u8] = b"stc1";

/// Parse a raw Console 1 SysEx message into its JSON payload.
///
/// Returns `None` if the message is too short, has the wrong start/manufacturer/stop bytes,
/// wrong magic, or the extracted bytes aren't valid JSON.
pub fn parse_sysex_json(message: &[u8]) -> Option<Value> {
    let min_len = 2 + SYSEX_MAGIC.len() + 2 + 1; // start+manufacturer + magic + "{}" + stop
    if message.len() < min_len {
        return None;
    }
    if message[0] != SYSEX_START || message[1] != SYSEX_MANUFACTURER {
        return None;
    }
    if message[message.len() - 1] != SYSEX_STOP {
        return None;
    }
    let magic_start = 2;
    if &message[magic_start..magic_start + SYSEX_MAGIC.len()] != SYSEX_MAGIC {
        return None;
    }

    let json_bytes = &message[magic_start + SYSEX_MAGIC.len()..message.len() - 1];
    // Each wire byte is one JSON-source character (the encoder guarantees pure ASCII on the
    // wire, with anything else already re-escaped as literal `\uXXXX` text) — reconstructing
    // via `byte as char` mirrors the JS decoder's `String.fromCharCode` per byte exactly, and
    // serde_json's own parser un-escapes the `\uXXXX` sequences back into real Unicode text.
    let json_str: String = json_bytes.iter().map(|&b| b as char).collect();
    serde_json::from_str(&json_str).ok()
}

/// Build a Console 1 SysEx message from a JSON value: `0xF0 0x7D "stc1" <escaped JSON> 0xF7`.
///
/// Non-ASCII-printable characters in the serialized JSON are re-escaped as `\uXXXX` (with
/// surrogate pairs above the BMP) so the wire payload stays pure ASCII 0x20-0x7E throughout,
/// matching Console 1's SysEx transport requirement.
pub fn build_sysex_frame(value: &Value) -> Vec<u8> {
    let json_str = serde_json::to_string(value).expect("serde_json::Value always serializes");
    let mut data = Vec::with_capacity(json_str.len() + 8);
    data.push(SYSEX_START);
    data.push(SYSEX_MANUFACTURER);
    data.extend_from_slice(SYSEX_MAGIC);

    for ch in json_str.chars() {
        let cp = ch as u32;
        if (0x20..0x7f).contains(&cp) {
            data.push(cp as u8);
            continue;
        }
        if cp <= 0xffff {
            push_escape(&mut data, cp);
        } else {
            // Surrogate-pair math for codepoints above the Basic Multilingual Plane — required
            // by JSON's own `\uXXXX` escape syntax regardless of the host language's string
            // representation. `0xD800` is the high-surrogate base and `0xDC00` the low-surrogate
            // base; each surrogate carries 10 bits of the 20-bit offset above U+10000, high bits
            // first.
            let code = cp - 0x10000;
            let hi = 0xd800 + ((code >> 10) & 0x3ff);
            let lo = 0xdc00 + (code & 0x3ff);
            push_escape(&mut data, hi);
            push_escape(&mut data, lo);
        }
    }
    data.push(SYSEX_STOP);
    data
}

fn push_escape(data: &mut Vec<u8>, codepoint: u32) {
    data.extend_from_slice(format!("\\u{codepoint:04x}").as_bytes());
}

/// The fixed set of fields Console 1 Fader Mk III's firmware accepts inside a `trackBatch`
/// entry. Any Filter/EQ/Compressor field is structurally impossible to construct here — this
/// firmware rejects the ENTIRE outbound `trackBatch` message if any entry contains a field name
/// outside this schema (confirmed via real-hardware A/B testing, see the project `CLAUDE.md`).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleTrackFields {
    pub track: i64,
    pub is_active: bool,
    pub track_id: String,
    pub color: u32,
    pub name: String,
    /// Number, or the JSON string `"-Infinity"` for silence — see `value_coercion`.
    pub volume: Value,
    pub meter: Vec<f64>,
    pub mute: bool,
    pub solo: bool,
    pub selected: bool,
    pub max_volume_value: f64,
    pub max_send_value: f64,
    pub pan: f64,
    pub send1: Value,
    pub send1_on: bool,
    pub send2: Value,
    pub send2_on: bool,
    pub send3: Value,
    pub send3_on: bool,
    pub send4: Value,
    pub send4_on: bool,
    pub send5: Value,
    pub send5_on: bool,
    pub send6: Value,
    pub send6_on: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_track_fields() -> ConsoleTrackFields {
        ConsoleTrackFields {
            track: 1,
            is_active: true,
            track_id: "ABCD1234".to_string(),
            color: 0x00a5ff,
            name: "Ch 1".to_string(),
            volume: json!(0),
            meter: vec![0.0],
            mute: false,
            solo: false,
            selected: false,
            max_volume_value: 10.0,
            max_send_value: 10.0,
            pan: 0.5,
            send1: json!(0),
            send1_on: false,
            send2: json!(0),
            send2_on: false,
            send3: json!(0),
            send3_on: false,
            send4: json!(0),
            send4_on: false,
            send5: json!(0),
            send5_on: false,
            send6: json!(0),
            send6_on: false,
        }
    }

    #[test]
    fn console_track_fields_serializes_to_exactly_the_allowed_keys() {
        let value = serde_json::to_value(sample_track_fields()).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        keys.sort();
        let mut expected = vec![
            "track",
            "isActive",
            "trackId",
            "color",
            "name",
            "volume",
            "meter",
            "mute",
            "solo",
            "selected",
            "maxVolumeValue",
            "maxSendValue",
            "pan",
            "send1",
            "send1On",
            "send2",
            "send2On",
            "send3",
            "send3On",
            "send4",
            "send4On",
            "send5",
            "send5On",
            "send6",
            "send6On",
        ];
        expected.sort();
        assert_eq!(keys, expected);
    }

    #[test]
    fn round_trip_preserves_ascii_object() {
        let value = json!({"cmd": "RESET"});
        let frame = build_sysex_frame(&value);
        assert_eq!(frame[0], SYSEX_START);
        assert_eq!(frame[1], SYSEX_MANUFACTURER);
        assert_eq!(&frame[2..6], SYSEX_MAGIC);
        assert_eq!(*frame.last().unwrap(), SYSEX_STOP);
        assert_eq!(parse_sysex_json(&frame), Some(value));
    }

    #[test]
    fn round_trip_escapes_non_ascii_bmp_character_and_stays_pure_ascii_on_the_wire() {
        let value = json!({"name": "Kuba Bém"});
        let frame = build_sysex_frame(&value);
        for &b in &frame[6..frame.len() - 1] {
            assert!(
                (0x20..0x7f).contains(&b),
                "non-ASCII byte {b:#x} found on the wire"
            );
        }
        assert_eq!(parse_sysex_json(&frame), Some(value));
    }

    #[test]
    fn round_trip_escapes_astral_plane_character_as_surrogate_pair() {
        let value = json!({"name": "😀"}); // U+1F600, outside the Basic Multilingual Plane
        let frame = build_sysex_frame(&value);
        for &b in &frame[6..frame.len() - 1] {
            assert!((0x20..0x7f).contains(&b));
        }
        assert_eq!(parse_sysex_json(&frame), Some(value));
    }

    #[test]
    fn parse_rejects_wrong_start_byte() {
        let mut frame = build_sysex_frame(&json!({"cmd": "RESET"}));
        frame[0] = 0x00;
        assert_eq!(parse_sysex_json(&frame), None);
    }

    #[test]
    fn parse_rejects_wrong_magic() {
        let mut frame = build_sysex_frame(&json!({"cmd": "RESET"}));
        frame[2] = b'x';
        assert_eq!(parse_sysex_json(&frame), None);
    }

    #[test]
    fn parse_rejects_wrong_stop_byte() {
        let mut frame = build_sysex_frame(&json!({"cmd": "RESET"}));
        let last = frame.len() - 1;
        frame[last] = 0x00;
        assert_eq!(parse_sysex_json(&frame), None);
    }

    #[test]
    fn parse_rejects_too_short_message() {
        assert_eq!(parse_sysex_json(&[0xf0, 0x7d]), None);
    }

    #[test]
    fn parse_rejects_invalid_json_payload() {
        let mut data = vec![SYSEX_START, SYSEX_MANUFACTURER];
        data.extend_from_slice(SYSEX_MAGIC);
        data.extend_from_slice(b"not json");
        data.push(SYSEX_STOP);
        assert_eq!(parse_sysex_json(&data), None);
    }
}
