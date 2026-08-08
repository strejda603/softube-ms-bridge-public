//! Suppresses the immediate MS broadcast-echo of a value the bridge itself just wrote,
//! so it isn't mistaken for an external change. Suppresses exactly one matching echo
//! within a fixed window, then stops suppressing.
//!
//! Port of `index.js`'s `shouldSuppressMsEcho`/`noteMsWrite`.

use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long after a write an echo of it is still considered an echo rather than a genuine
/// external change. An echo arriving later than this is treated as real.
pub const MS_ECHO_SUPPRESS_WINDOW: Duration = Duration::from_millis(150);

struct RecentWrite {
    value: Value,
    at: Instant,
}

/// Tracks the bridge's own pending writes to Mixing Station so their echoes can be
/// recognized and dropped.
///
/// Keys are `"<path>|<format>"` — the MS console-data path and its value format joined by
/// a pipe, e.g. `"ch.0.mix.pan|norm"` or `"ch.0.mix.lvl|val"`. Callers must build the key
/// the same way on both [`note_write`](Self::note_write) and
/// [`should_suppress`](Self::should_suppress); keys are compared verbatim.
///
/// The tracker never reads a clock — the current time is passed in — so callers control
/// time and behavior stays deterministic.
#[derive(Default)]
pub struct EchoSuppressionTracker {
    recent_writes: HashMap<String, RecentWrite>,
}

/// Loose equality matching JS's `==` for the value shapes MS actually sends: numbers,
/// bools, strings.
///
/// Two gaps between `Value`'s `PartialEq` and JS `==` matter here:
/// - JS has a single numeric type, but `serde_json::Number` distinguishes integer from
///   float representations, so `json!(-6) != json!(-6.0)` under `==`. The same value can
///   arrive as either variant depending on whether it was built locally from an `f64` or
///   parsed off the wire as a whole number, so numbers are compared as `f64`.
/// - `true == 1` and `false == 0` under JS `==`, and MS represents booleans as `0`/`1`.
fn loosely_equal(a: &Value, b: &Value) -> bool {
    if a == b {
        return true;
    }
    match (a, b) {
        (Value::Number(_), Value::Number(_)) => a.as_f64() == b.as_f64(),
        (Value::Bool(x), Value::Number(_)) => b.as_f64() == Some(*x as i32 as f64),
        (Value::Number(_), Value::Bool(y)) => a.as_f64() == Some(*y as i32 as f64),
        _ => false,
    }
}

impl EchoSuppressionTracker {
    /// Creates a tracker with no pending writes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that the bridge wrote `value` to `key` at `at`, arming suppression of the
    /// echo MS will broadcast back.
    ///
    /// At most one write per key is pending: recording a second write for the same key
    /// replaces the first, so only the newest value can be suppressed.
    pub fn note_write(&mut self, key: &str, value: Value, at: Instant) {
        self.recent_writes
            .insert(key.to_string(), RecentWrite { value, at });
    }

    /// Returns whether an inbound `value` for `key` at `now` is the echo of the bridge's
    /// own write and should be ignored.
    ///
    /// Suppresses at most once per recorded write: a match consumes the pending entry, so
    /// a repeat of the same value is reported as a genuine external change. Values are
    /// compared loosely, matching JS `==` (`true` equals `1`, `-6` equals `-6.0`). A pending entry older
    /// than [`MS_ECHO_SUPPRESS_WINDOW`] is discarded without suppressing; a non-matching
    /// value leaves the entry armed for a later matching echo.
    pub fn should_suppress(&mut self, key: &str, value: &Value, now: Instant) -> bool {
        let Some(rec) = self.recent_writes.get(key) else {
            return false;
        };
        if now.saturating_duration_since(rec.at) > MS_ECHO_SUPPRESS_WINDOW {
            self.recent_writes.remove(key);
            return false;
        }
        if loosely_equal(&rec.value, value) {
            self.recent_writes.remove(key);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{Duration, Instant};

    #[test]
    fn no_recorded_write_does_not_suppress() {
        let mut tracker = EchoSuppressionTracker::new();
        let now = Instant::now();
        assert!(!tracker.should_suppress("ch.0.mix.pan|norm", &json!(0.5), now));
    }

    #[test]
    fn matching_value_within_window_is_suppressed_exactly_once() {
        let mut tracker = EchoSuppressionTracker::new();
        let t0 = Instant::now();
        tracker.note_write("ch.0.mix.pan|norm", json!(0.5), t0);

        let t1 = t0 + Duration::from_millis(50);
        assert!(tracker.should_suppress("ch.0.mix.pan|norm", &json!(0.5), t1));
        // Second matching echo is NOT suppressed -- entry was consumed.
        assert!(!tracker.should_suppress("ch.0.mix.pan|norm", &json!(0.5), t1));
    }

    #[test]
    fn expired_entry_is_not_suppressed_and_is_cleaned_up() {
        let mut tracker = EchoSuppressionTracker::new();
        let t0 = Instant::now();
        tracker.note_write("ch.0.mix.on|val", json!(true), t0);

        let t1 = t0 + Duration::from_millis(151);
        assert!(!tracker.should_suppress("ch.0.mix.on|val", &json!(true), t1));
        // Entry was removed by the expiry check -- a later exact-match at a fresh write
        // time still requires a new note_write, not a stale hit.
        assert!(!tracker.should_suppress("ch.0.mix.on|val", &json!(true), t1));
    }

    #[test]
    fn mismatched_value_is_not_suppressed_and_entry_survives() {
        let mut tracker = EchoSuppressionTracker::new();
        let t0 = Instant::now();
        tracker.note_write("ch.0.mix.lvl|val", json!(-6.0), t0);

        let t1 = t0 + Duration::from_millis(10);
        assert!(!tracker.should_suppress("ch.0.mix.lvl|val", &json!(-3.0), t1));
        // Original entry still present -- a later exact match still suppresses.
        assert!(tracker.should_suppress("ch.0.mix.lvl|val", &json!(-6.0), t1));
    }

    #[test]
    fn loose_equality_treats_bool_true_and_one_as_equal() {
        let mut tracker = EchoSuppressionTracker::new();
        let t0 = Instant::now();
        tracker.note_write("ch.0.solo|val", json!(true), t0);
        assert!(tracker.should_suppress("ch.0.solo|val", &json!(1), t0));
    }

    #[test]
    fn loose_equality_treats_bool_false_and_zero_as_equal() {
        let mut tracker = EchoSuppressionTracker::new();
        let t0 = Instant::now();
        tracker.note_write("ch.0.mix.on|val", json!(false), t0);
        assert!(tracker.should_suppress("ch.0.mix.on|val", &json!(0), t0));
    }

    #[test]
    fn loose_equality_treats_integer_and_float_representations_as_equal() {
        let mut tracker = EchoSuppressionTracker::new();
        let t0 = Instant::now();
        tracker.note_write("ch.0.mix.lvl|val", json!(-6), t0);
        assert!(tracker.should_suppress("ch.0.mix.lvl|val", &json!(-6.0), t0));
    }

    #[test]
    fn different_keys_do_not_interfere() {
        let mut tracker = EchoSuppressionTracker::new();
        let t0 = Instant::now();
        tracker.note_write("ch.0.mix.pan|norm", json!(0.5), t0);
        assert!(!tracker.should_suppress("ch.1.mix.pan|norm", &json!(0.5), t0));
    }

    #[test]
    fn note_write_overwrites_a_prior_pending_entry_for_the_same_key() {
        let mut tracker = EchoSuppressionTracker::new();
        let t0 = Instant::now();
        tracker.note_write("ch.0.mix.lvl|val", json!(-6.0), t0);
        tracker.note_write("ch.0.mix.lvl|val", json!(-3.0), t0);
        assert!(!tracker.should_suppress("ch.0.mix.lvl|val", &json!(-6.0), t0));
        assert!(tracker.should_suppress("ch.0.mix.lvl|val", &json!(-3.0), t0));
    }
}
