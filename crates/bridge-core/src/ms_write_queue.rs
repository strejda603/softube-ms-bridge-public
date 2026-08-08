//! Coalescing queue for outbound Console1 -> Mixing Station writes (faders, pans, mutes,
//! etc.). Sibling to `bare_update_queue::BareUpdateQueue` (same `(String,String)`-keyed
//! shape) but for the opposite direction. No timer here -- that's a `bridge-cli`
//! event-loop concern, matching every other queue module in this migration.
//!
//! Port of the coalescing half of `index.js`'s `queueMsWrite`.

use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct MsWriteEntry {
    pub path: String,
    pub format: String,
    pub value: Value,
}

#[derive(Default)]
pub struct MsWriteQueue {
    entries: HashMap<(String, String), Value>,
}

impl MsWriteQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn queue(&mut self, path: String, format: String, value: Value) {
        self.entries.insert((path, format), value);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn take_all(&mut self) -> Vec<MsWriteEntry> {
        std::mem::take(&mut self.entries)
            .into_iter()
            .map(|((path, format), value)| MsWriteEntry {
                path,
                format,
                value,
            })
            .collect()
    }
}

/// Builds the `/console/data/set/<path>/<format>` WS request body for a single write.
pub fn build_ms_set_request(path: &str, format: &str, value: &Value) -> Value {
    json!({
        "path": format!("/console/data/set/{path}/{format}"),
        "method": "POST",
        "body": { "value": value },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn queue_then_take_all_returns_the_entry() {
        let mut q = MsWriteQueue::new();
        q.queue("ch.0.mix.lvl".to_string(), "val".to_string(), json!(-6.0));
        let entries = q.take_all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "ch.0.mix.lvl");
        assert_eq!(entries[0].format, "val");
        assert_eq!(entries[0].value, json!(-6.0));
        assert!(q.is_empty());
    }

    #[test]
    fn later_queue_for_same_path_and_format_overwrites_earlier_value() {
        let mut q = MsWriteQueue::new();
        q.queue("ch.0.mix.pan".to_string(), "norm".to_string(), json!(0.2));
        q.queue("ch.0.mix.pan".to_string(), "norm".to_string(), json!(0.8));
        let entries = q.take_all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].value, json!(0.8));
    }

    #[test]
    fn same_path_different_format_stays_separate() {
        let mut q = MsWriteQueue::new();
        q.queue("ch.0.mix.pan".to_string(), "val".to_string(), json!(0.0));
        q.queue("ch.0.mix.pan".to_string(), "norm".to_string(), json!(0.5));
        let entries = q.take_all();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn different_paths_stay_separate() {
        let mut q = MsWriteQueue::new();
        q.queue("ch.0.mix.lvl".to_string(), "val".to_string(), json!(-6.0));
        q.queue("ch.1.mix.lvl".to_string(), "val".to_string(), json!(-3.0));
        let entries = q.take_all();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn take_all_on_empty_queue_returns_empty_vec() {
        let mut q = MsWriteQueue::new();
        assert!(q.take_all().is_empty());
    }

    #[test]
    fn build_set_request_produces_the_correct_ws_payload_shape() {
        let payload = build_ms_set_request("ch.0.mix.lvl", "val", &json!(-6.0));
        assert_eq!(
            payload,
            json!({
                "path": "/console/data/set/ch.0.mix.lvl/val",
                "method": "POST",
                "body": { "value": -6.0 }
            })
        );
    }
}
