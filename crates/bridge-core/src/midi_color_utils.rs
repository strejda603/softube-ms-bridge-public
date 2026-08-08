//! Pure helpers for converting Mixing Station color values into Softube's 24-bit color
//! integer format.
//!
//! Mixing Station may provide a palette index (0..15), a `styleClass` string, or an
//! already-encoded RGB int. All three are normalized here into Softube's 24-bit integer
//! format, which is `0xBBGGRR` — red in the least-significant byte, green in the middle
//! byte, blue in the most-significant byte (the reverse of the more common `0xRRGGBB`
//! layout, per the JS original's own comment).
//!
//! Ported from `midiColorUtils.js` — kept as small, pure, side-effect-free functions so
//! they can be unit-tested directly, mirroring the JS module's own rationale for being
//! split out of the bridge's main event-handling code.

use serde_json::Value;
use std::collections::HashMap;

/// Base palette colors for Mixing Station's 8 primary palette indices (0..7), already in
/// Softube's `0xBBGGRR` 24-bit format.
pub const MS_PALETTE_BASE_COLORS: [u32; 8] = [
    0x000000, 0x0000ff, 0x00ff00, 0x00ffff, 0xff0000, 0xff00ff, 0xffff00, 0xffffff,
];

/// Maps Mixing Station's `styleClass` strings (e.g. `"mixer-green"`) to a palette index
/// 0..15. Indices 0..7 are the base colors; 8..15 are their "-inv" (inverted-look)
/// counterparts, rendered as a lighter tint rather than a true color inversion — see
/// [`ms_color_to_softube_color_int`].
pub fn ms_styleclass_to_palette_index() -> HashMap<&'static str, u8> {
    HashMap::from([
        ("mixer-black", 0),
        ("mixer-red", 1),
        ("mixer-green", 2),
        ("mixer-yellow", 3),
        ("mixer-blue", 4),
        ("mixer-magenta", 5),
        ("mixer-cyan", 6),
        ("mixer-white", 7),
        ("mixer-black-inv", 8),
        ("mixer-red-inv", 9),
        ("mixer-green-inv", 10),
        ("mixer-yellow-inv", 11),
        ("mixer-blue-inv", 12),
        ("mixer-magenta-inv", 13),
        ("mixer-cyan-inv", 14),
        ("mixer-white-inv", 15),
    ])
}

/// Lighten a Softube 24-bit color (`0xBBGGRR`, red in the LSB) by blending each channel
/// towards white (255) by `amount01` (0..1). `amount01 == 0.0` returns the color
/// unchanged; `amount01 == 1.0` returns pure white.
///
/// Used for the palette's "-inv" indices (8..15): rather than truly inverting the color,
/// Softube's on-screen display renders the "inverted-look" variant of a track color as a
/// lighter tint of the same base hue, which stays legible against the display's dark
/// background.
pub fn tint_softube_color(color_int: u32, amount01: f64) -> u32 {
    let r = (color_int & 0xff) as f64;
    let g = ((color_int >> 8) & 0xff) as f64;
    let b = ((color_int >> 16) & 0xff) as f64;
    let lr = (r + (255.0 - r) * amount01).round() as u32 & 0xff;
    let lg = (g + (255.0 - g) * amount01).round() as u32 & 0xff;
    let lb = (b + (255.0 - b) * amount01).round() as u32 & 0xff;
    lr | (lg << 8) | (lb << 16)
}

/// Convert a Mixing Station color value into Softube's 24-bit `0xBBGGRR` color int.
///
/// Accepts three input shapes, mirroring the JS original's `typeof` dispatch:
/// - a palette index number 0..15 (0..7 map directly to [`MS_PALETTE_BASE_COLORS`]; 8..15
///   are the "-inv" variants, tinted 60% towards white via [`tint_softube_color`]);
/// - an already-encoded raw RGB int, 0..=`0xffffff`, passed through unchanged (numbers
///   16..=`0xffffff` fall into this branch since they're outside the palette-index range);
/// - a `styleClass` string (e.g. `"mixer-green"`), looked up via
///   [`ms_styleclass_to_palette_index`] and resolved the same way as a numeric palette
///   index.
///
/// Returns `None` for an out-of-range number (negative or `> 0xffffff`), an unrecognized
/// style class string, or any other JSON value shape (object, array, bool, null).
///
/// JS has a single untyped `number`, so `msColorToSoftubeColorInt` doesn't distinguish
/// integers from whole-number floats (e.g. `8.0` behaves identically to `8`). JSON
/// numbers can arrive from `serde_json` as either `as_i64()` or `as_f64()` depending on
/// how the source encoded them (Mixing Station's WS JSON may emit a color as `8.0`
/// rather than `8`), so we accept `as_i64()` directly and, failing that, fall back to
/// `as_f64()` — but only when it's a finite whole number, so a genuinely fractional value
/// like `1.5` (which JS itself would mangle into a bogus non-integer array index and
/// therefore also treat as absent) still correctly returns `None`.
pub fn ms_color_to_softube_color_int(value: &Value) -> Option<u32> {
    let as_int = value.as_i64().or_else(|| {
        value
            .as_f64()
            .filter(|f| f.is_finite() && f.fract() == 0.0)
            .map(|f| f as i64)
    });
    if let Some(n) = as_int {
        return if (0..=15).contains(&n) {
            let idx = n as usize;
            let base = MS_PALETTE_BASE_COLORS[idx % 8];
            Some(if idx < 8 {
                base
            } else {
                tint_softube_color(base, 0.6)
            })
        } else if (0..=0xffffff).contains(&n) {
            Some(n as u32)
        } else {
            None
        };
    }
    if let Some(s) = value.as_str() {
        if let Some(&idx) = ms_styleclass_to_palette_index().get(s) {
            let base = MS_PALETTE_BASE_COLORS[(idx % 8) as usize];
            return Some(if idx < 8 {
                base
            } else {
                tint_softube_color(base, 0.6)
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tint_amount_0_returns_original_color() {
        assert_eq!(tint_softube_color(0x0000ff, 0.0), 0x0000ff);
    }

    #[test]
    fn tint_amount_1_blends_fully_to_white() {
        assert_eq!(tint_softube_color(0x0000ff, 1.0), 0xffffff);
    }

    #[test]
    fn palette_index_0_to_7_maps_directly_to_base_colors() {
        assert_eq!(
            ms_color_to_softube_color_int(&json!(1)),
            Some(MS_PALETTE_BASE_COLORS[1])
        );
        assert_eq!(
            ms_color_to_softube_color_int(&json!(2)),
            Some(MS_PALETTE_BASE_COLORS[2])
        );
    }

    #[test]
    fn palette_index_8_to_15_tints_base_color_inv_variant() {
        let result = ms_color_to_softube_color_int(&json!(9));
        assert_eq!(
            result,
            Some(tint_softube_color(MS_PALETTE_BASE_COLORS[1], 0.6))
        );
    }

    #[test]
    fn already_encoded_rgb_int_passes_through_unchanged() {
        assert_eq!(
            ms_color_to_softube_color_int(&json!(0x123456)),
            Some(0x123456)
        );
    }

    #[test]
    fn styleclass_string_maps_via_lookup_table() {
        let idx = ms_styleclass_to_palette_index()["mixer-green"];
        assert_eq!(
            ms_color_to_softube_color_int(&json!("mixer-green")),
            Some(MS_PALETTE_BASE_COLORS[idx as usize])
        );
    }

    #[test]
    fn unknown_styleclass_string_returns_none() {
        assert_eq!(
            ms_color_to_softube_color_int(&json!("not-a-real-class")),
            None
        );
    }

    #[test]
    fn out_of_range_number_returns_none() {
        assert_eq!(ms_color_to_softube_color_int(&json!(-1)), None);
        assert_eq!(ms_color_to_softube_color_int(&json!(0x1000000_i64)), None);
    }

    #[test]
    fn whole_number_float_behaves_identically_to_integer() {
        assert_eq!(
            ms_color_to_softube_color_int(&json!(8.0)),
            ms_color_to_softube_color_int(&json!(8))
        );
        assert_eq!(
            ms_color_to_softube_color_int(&json!(100.0)),
            ms_color_to_softube_color_int(&json!(100))
        );
    }

    #[test]
    fn genuinely_fractional_number_still_returns_none() {
        assert_eq!(ms_color_to_softube_color_int(&json!(1.5)), None);
    }
}
