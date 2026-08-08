//! Pure helpers for decoding Mixing Station metering2 payloads and converting dB values into
//! Console 1's expected normalized peak meter range.
//!
//! Ported from `meteringUtils.js`. Kept as small, pure functions (no I/O) so they can be
//! unit-tested in isolation, mirroring the JS module's own rationale for being split out of
//! `index.js`.

use base64::{engine::general_purpose::STANDARD, Engine as _};

/// Result of decoding a metering2 binary payload: how many raw int16 values were packed per
/// subscribed parameter, and the maximum dB value seen for each parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct MeteringDecodeResult {
    pub values_per_param: usize,
    pub max_db_by_param: Vec<f64>,
}

/// Decodes a non-padded base64 string.
///
/// Mixing Station's metering2 wire format sends base64 payloads with the trailing `=`
/// padding stripped off. This pads the string back out to a multiple of 4 characters (the
/// standard base64 block size) before decoding, so the payload round-trips correctly.
///
/// The JS original's "non-string input" fallback path (`typeof s !== "string"`) is dropped
/// here: Rust's `&str` parameter type makes that case structurally impossible to hit.
pub fn decode_non_padded_base64(s: &str) -> Vec<u8> {
    let rem = s.len() % 4;
    let pad = if rem == 0 { 0 } else { 4 - rem };
    let mut padded = String::with_capacity(s.len() + pad);
    padded.push_str(s);
    for _ in 0..pad {
        padded.push('=');
    }
    STANDARD.decode(&padded).unwrap_or_default()
}

/// Decodes a metering2 binary payload.
///
/// Values are packed as big-endian int16s, scaled by 100 (i.e. a meter reading of 1.02 dB is
/// encoded on the wire as the integer `102`; dividing by 100.0 recovers the dB value). This
/// matches Node's `buf.readInt16BE(offset)` semantics exactly (two bytes, most-significant
/// byte first, two's-complement signed).
///
/// The subscription doesn't tell us per-channel stereo/extra meter counts up front, so this
/// only supports the case where every subscribed parameter contributes the same number of
/// values; the total value count must therefore divide evenly by `param_count`. When it
/// doesn't (or the count/buffer is otherwise invalid), this returns `None` rather than
/// guessing at a split.
///
/// For each parameter, the *maximum* dB value across its values-per-param slice is kept
/// (peak-hold across however many values --e.g. stereo L/R-- landed in that slot).
///
/// The JS original's "non-integer paramCount (e.g. 1.5)" guard is dropped here: `param_count`
/// is a plain `i64`, which cannot represent a fraction, so that branch is structurally
/// unreachable. The `0` and negative-count guards are kept since those are valid `i64` values.
pub fn decode_metering2_binary(b64: &str, param_count: i64) -> Option<MeteringDecodeResult> {
    if param_count <= 0 {
        return None;
    }
    let param_count = param_count as usize;
    let buf = decode_non_padded_base64(b64);
    if buf.len() < 2 {
        return None;
    }
    let total_values = buf.len() / 2;
    if total_values == 0 || !total_values.is_multiple_of(param_count) {
        return None;
    }

    let values_per_param = total_values / param_count;
    let mut max_db_by_param = vec![f64::NEG_INFINITY; param_count];
    for (p, slot) in max_db_by_param.iter_mut().enumerate() {
        let start = p * values_per_param;
        let end = start + values_per_param;
        for i in start..end {
            let off = i * 2;
            if off + 1 >= buf.len() {
                break;
            }
            let raw = i16::from_be_bytes([buf[off], buf[off + 1]]);
            let db = raw as f64 / 100.0;
            if db.is_finite() {
                *slot = slot.max(db);
            }
        }
    }

    Some(MeteringDecodeResult {
        values_per_param,
        max_db_by_param,
    })
}

/// Converts a dB value from Mixing Station's metering2 stream into Console 1's expected
/// normalized peak meter value (range 0..1).
///
/// Values arrive in dB, are converted to linear gain (`10^(db/20)`), then scaled by
/// `sqrt(2)` — the same RMS-to-peak conversion factor Softube's Cubase reference driver
/// script applies — and finally clamped into 0..1, which is the range Console 1's meter
/// field expects.
///
/// Takes a plain `f64` rather than `serde_json::Value`: callers in the WS-handling layer are
/// expected to have already parsed the wire value to a number before calling this, so "was
/// this a valid number at all" is handled at the call site. `f64::NAN` (and any other
/// non-finite float) is the Rust equivalent of the JS version's "non-finite input (including
/// a non-numeric string coerced via `Number(db)`)" fallback, and returns `0.0` here too.
pub fn db_to_console1_meter_norm(db: f64) -> f64 {
    if !db.is_finite() {
        return 0.0;
    }
    let lin = 10f64.powf(db / 20.0);
    if !lin.is_finite() {
        return 0.0;
    }
    let peak = lin * std::f64::consts::SQRT_2;
    peak.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64_no_pad(bytes: &[u8]) -> String {
        STANDARD.encode(bytes).trim_end_matches('=').to_string()
    }

    #[test]
    fn decode_non_padded_base64_decodes_a_non_padded_string() {
        let buf = vec![0x00u8, 0x64];
        let b64 = b64_no_pad(&buf);
        assert_eq!(decode_non_padded_base64(&b64), buf);
    }

    #[test]
    fn single_param_single_value_1_02db_encoded_as_102() {
        let mut buf = [0u8; 2];
        buf.copy_from_slice(&102i16.to_be_bytes());
        let b64 = b64_no_pad(&buf);
        let result = decode_metering2_binary(&b64, 1).unwrap();
        assert_eq!(result.values_per_param, 1);
        assert_eq!(result.max_db_by_param[0], 1.02);
    }

    #[test]
    fn multiple_params_take_max_value_per_param() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(-2000i16).to_be_bytes()); // param0 value A: -20.00
        buf.extend_from_slice(&(-1000i16).to_be_bytes()); // param0 value B: -10.00 (max)
        buf.extend_from_slice(&(500i16).to_be_bytes()); // param1 value A: 5.00 (max)
        buf.extend_from_slice(&(200i16).to_be_bytes()); // param1 value B: 2.00
        let b64 = b64_no_pad(&buf);
        let result = decode_metering2_binary(&b64, 2).unwrap();
        assert_eq!(result.values_per_param, 2);
        assert_eq!(result.max_db_by_param, vec![-10.0, 5.0]);
    }

    #[test]
    fn invalid_param_count_returns_none() {
        assert_eq!(decode_metering2_binary("AAA", 0), None);
        assert_eq!(decode_metering2_binary("AAA", -1), None);
    }

    #[test]
    fn buffer_too_short_returns_none() {
        assert_eq!(decode_metering2_binary("", 1), None);
    }

    #[test]
    fn value_count_not_divisible_by_param_count_returns_none() {
        let buf = [0u8; 6]; // 3 int16 values, paramCount=2 doesn't divide evenly
        let b64 = b64_no_pad(&buf);
        assert_eq!(decode_metering2_binary(&b64, 2), None);
    }

    #[test]
    fn db_to_meter_norm_very_loud_values_clamp_to_1() {
        assert_eq!(db_to_console1_meter_norm(20.0), 1.0);
    }

    #[test]
    fn db_to_meter_norm_silence_is_near_0() {
        assert!(db_to_console1_meter_norm(-90.0) < 0.001);
    }

    #[test]
    fn db_to_meter_norm_0db_converts_to_sqrt2_clamped_to_1() {
        assert_eq!(db_to_console1_meter_norm(0.0), 1.0);
    }

    #[test]
    fn db_to_meter_norm_non_finite_input_returns_0() {
        assert_eq!(db_to_console1_meter_norm(f64::NAN), 0.0);
    }
}
