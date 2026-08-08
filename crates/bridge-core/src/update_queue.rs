//! Coalescing queue for outbound Console 1 track field updates.
//!
//! See `index.js`'s `queueConsole1TrackUpdate` for the original this ports. The JS version's
//! `setTimeout`-based flush timer and `osdEnabled` gating are async-engine responsibilities,
//! not part of this pure coalescing structure — see `bridge-cli`'s main loop (a later task) for
//! where those live in this port.

use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueuedUpdate {
    /// Merged partial field updates for one trackId — later `queue()` calls for the same field
    /// overwrite earlier ones, non-overlapping fields accumulate.
    pub fields: HashMap<String, Value>,
    /// Sticky: once true for a trackId (via any `queue()` call), stays true until drained,
    /// matching JS's "once forced, always forced for that flush" comment.
    pub force_send: bool,
}

#[derive(Debug, Default)]
pub struct UpdateQueue {
    entries: HashMap<String, QueuedUpdate>,
}

impl UpdateQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a partial field update for a trackId. No-op for an empty `partial`.
    pub fn queue(&mut self, track_id: String, partial: HashMap<String, Value>, force_send: bool) {
        if partial.is_empty() {
            return;
        }
        let entry = self.entries.entry(track_id).or_default();
        entry.fields.extend(partial);
        entry.force_send |= force_send;
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drain and return only the force-send entries, leaving non-forced entries queued.
    pub fn take_forced(&mut self) -> Vec<(String, QueuedUpdate)> {
        let forced_keys: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, v)| v.force_send)
            .map(|(k, _)| k.clone())
            .collect();
        forced_keys
            .into_iter()
            .filter_map(|k| self.entries.remove(&k).map(|v| (k, v)))
            .collect()
    }

    /// Drain and return every queued entry, forced or not.
    pub fn take_all(&mut self) -> Vec<(String, QueuedUpdate)> {
        std::mem::take(&mut self.entries).into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn queue_empty_partial_is_a_no_op() {
        let mut q = UpdateQueue::new();
        q.queue("T1".into(), HashMap::new(), false);
        assert!(q.is_empty());
    }

    #[test]
    fn queue_merges_non_overlapping_fields_for_same_track() {
        let mut q = UpdateQueue::new();
        q.queue("T1".into(), fields(&[("volume", json!(0))]), false);
        q.queue("T1".into(), fields(&[("mute", json!(true))]), false);
        let all = q.take_all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "T1");
        assert_eq!(all[0].1.fields.get("volume"), Some(&json!(0)));
        assert_eq!(all[0].1.fields.get("mute"), Some(&json!(true)));
    }

    #[test]
    fn queue_later_value_wins_for_same_field() {
        let mut q = UpdateQueue::new();
        q.queue("T1".into(), fields(&[("volume", json!(0))]), false);
        q.queue("T1".into(), fields(&[("volume", json!(-6))]), false);
        let all = q.take_all();
        assert_eq!(all[0].1.fields.get("volume"), Some(&json!(-6)));
    }

    #[test]
    fn queue_force_send_is_sticky() {
        let mut q = UpdateQueue::new();
        q.queue("T1".into(), fields(&[("color", json!(1))]), true);
        q.queue("T1".into(), fields(&[("name", json!("X"))]), false);
        let all = q.take_all();
        assert!(all[0].1.force_send);
    }

    #[test]
    fn take_forced_only_returns_and_removes_forced_entries() {
        let mut q = UpdateQueue::new();
        q.queue("FORCED".into(), fields(&[("color", json!(1))]), true);
        q.queue("NORMAL".into(), fields(&[("volume", json!(0))]), false);

        let forced = q.take_forced();
        assert_eq!(forced.len(), 1);
        assert_eq!(forced[0].0, "FORCED");

        // The normal (non-forced) entry is still queued.
        assert!(!q.is_empty());
        let remaining = q.take_all();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].0, "NORMAL");
    }

    #[test]
    fn take_all_drains_the_whole_queue() {
        let mut q = UpdateQueue::new();
        q.queue("A".into(), fields(&[("x", json!(1))]), false);
        q.queue("B".into(), fields(&[("y", json!(2))]), true);
        let all = q.take_all();
        assert_eq!(all.len(), 2);
        assert!(q.is_empty());
    }

    #[test]
    fn separate_track_ids_stay_separate() {
        let mut q = UpdateQueue::new();
        q.queue("A".into(), fields(&[("volume", json!(1))]), false);
        q.queue("B".into(), fields(&[("volume", json!(2))]), false);
        let all = q.take_all();
        assert_eq!(all.len(), 2);
    }
}
