//! Pure helpers for Console 1's stereo-linked hybrid pan control.
//!
//! Ported from `panUtils.js`. Kept separate from the rest of `bridge-core` so this logic
//! mirrors the JS module split (isolated for unit testing without spinning up real
//! MIDI/WS connections) — same rationale as `console1_status_bank` and `midi_color_utils`.

/// Threshold (0..1) where a stereo-linked pair's width reaches 0 and mono panning begins.
pub const STEREO_HYBRID_NARROW_ZONE: f64 = 0.25;

/// Result of converting a single Console 1 pan knob value into a stereo dual-mono pan pair.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DualMonoPans {
    pub left: f64,
    pub right: f64,
    pub width: f64,
    pub mid: f64,
}

/// Clamp a number to 0..1. Non-finite input clamps to 0.
///
/// # Examples
/// - `clamp01(1.5)` -> `1.0`
/// - `clamp01(f64::NAN)` -> `0.0`
pub fn clamp01(n: f64) -> f64 {
    if !n.is_finite() {
        return 0.0;
    }
    n.clamp(0.0, 1.0)
}

/// Convert a single Console 1 pan knob value (0..1) into a stereo dual-mono pan pair.
///
/// Design goal:
/// - Center: full stereo width (hard L/R)
/// - Small turns: narrow the image towards mono (mid stays centered) — this is "zone A",
///   active while the knob's distance from center is within `narrow_zone`.
/// - Past the narrow zone ("zone B"): the image is fully mono (width 0), and further
///   turning pans that mono image left/right (both channels move together).
///
/// `narrow_zone` is the 0..1 threshold where width reaches 0; a NaN or zero value falls
/// back to [`STEREO_HYBRID_NARROW_ZONE`] (mirrors JS's `Number(narrowZone) ||
/// STEREO_HYBRID_NARROW_ZONE`, where only falsy values — `0` and `NaN` — take the
/// default). Other non-finite values (±Infinity) are *not* falsy in JS, so they are
/// clamped into range like any other value, same as everything else, to `0.001..=0.99`.
///
/// # Examples
/// `hybrid_stereo_pan_to_dual_mono_pans(0.5, STEREO_HYBRID_NARROW_ZONE)` ->
/// `DualMonoPans { left: 0.0, right: 1.0, width: 1.0, mid: 0.5 }`
pub fn hybrid_stereo_pan_to_dual_mono_pans(pan01: f64, narrow_zone: f64) -> DualMonoPans {
    let p = clamp01(pan01);
    let x = (p - 0.5) * 2.0;
    let ax = x.abs();
    let raw_narrow_zone = if narrow_zone.is_nan() || narrow_zone == 0.0 {
        STEREO_HYBRID_NARROW_ZONE
    } else {
        narrow_zone
    };
    let t = raw_narrow_zone.clamp(0.001, 0.99);

    if ax <= t {
        // Zone A: width reduction only, mid stays centered.
        let width = 1.0 - ax / t;
        let mid = 0.5;
        let half = 0.5 * width;
        DualMonoPans {
            left: clamp01(mid - half),
            right: clamp01(mid + half),
            width,
            mid,
        }
    } else {
        // Zone B: mono (width=0), then pan the mono image.
        let mono_balance01 = (ax - t) / (1.0 - t);
        let balance = x.signum() * mono_balance01;
        let mid = clamp01(0.5 + 0.5 * balance);
        DualMonoPans {
            left: mid,
            right: mid,
            width: 0.0,
            mid,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_pans(pan01: f64) -> DualMonoPans {
        hybrid_stereo_pan_to_dual_mono_pans(pan01, STEREO_HYBRID_NARROW_ZONE)
    }

    #[test]
    fn clamp01_clamps_below_0_and_above_1() {
        assert_eq!(clamp01(-0.5), 0.0);
        assert_eq!(clamp01(1.5), 1.0);
    }

    #[test]
    fn clamp01_non_finite_clamps_to_0() {
        assert_eq!(clamp01(f64::NAN), 0.0);
        assert_eq!(clamp01(f64::INFINITY), 0.0);
        assert_eq!(clamp01(f64::NEG_INFINITY), 0.0);
    }

    #[test]
    fn clamp01_in_range_passes_through() {
        assert_eq!(clamp01(0.42), 0.42);
    }

    #[test]
    fn center_knob_is_full_stereo_width_hard_lr() {
        let result = default_pans(0.5);
        assert_eq!(
            result,
            DualMonoPans {
                left: 0.0,
                right: 1.0,
                width: 1.0,
                mid: 0.5
            }
        );
    }

    #[test]
    fn at_narrow_zone_edge_width_reaches_0() {
        let result = default_pans(0.5 + STEREO_HYBRID_NARROW_ZONE / 2.0);
        assert!(result.width < 1e-9);
        assert_eq!(result.left, result.right);
    }

    #[test]
    fn past_narrow_zone_pans_mono_image_right() {
        let result = default_pans(1.0);
        assert_eq!(result.width, 0.0);
        assert_eq!(result.left, result.right);
        assert_eq!(result.mid, 1.0);
    }

    #[test]
    fn past_narrow_zone_pans_mono_image_left() {
        let result = default_pans(0.0);
        assert_eq!(result.width, 0.0);
        assert_eq!(result.left, result.right);
        assert_eq!(result.mid, 0.0);
    }

    #[test]
    fn out_of_range_input_clamped_to_0_1_first() {
        assert_eq!(default_pans(-5.0), default_pans(0.0));
        assert_eq!(default_pans(5.0), default_pans(1.0));
    }

    #[test]
    fn infinite_narrow_zone_clamps_to_upper_bound_not_default() {
        // JS: `Number(Infinity) || default` -> Infinity is truthy, so it is NOT
        // replaced by the default; it is clamped to 0.99 like any other value.
        let with_infinity = hybrid_stereo_pan_to_dual_mono_pans(1.0, f64::INFINITY);
        let with_explicit_bound = hybrid_stereo_pan_to_dual_mono_pans(1.0, 0.99);
        assert_eq!(with_infinity, with_explicit_bound);
    }

    #[test]
    fn negative_infinite_narrow_zone_clamps_to_lower_bound_not_default() {
        let with_neg_infinity = hybrid_stereo_pan_to_dual_mono_pans(1.0, f64::NEG_INFINITY);
        let with_explicit_bound = hybrid_stereo_pan_to_dual_mono_pans(1.0, 0.001);
        assert_eq!(with_neg_infinity, with_explicit_bound);
    }
}
