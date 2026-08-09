//! The bridge's run-state: `Standby` (Console 1 connected, no Mixing Station connection)
//! or `Running` (full bridging active). Tracked in `runtime.rs`'s `BridgeState`, mirrored
//! to the frontend via `BridgeEvent::LifecycleChanged`.

use serde::Serialize;

/// The bridge's lifecycle state. Unlike a plain `"standby"|"running"` string, this enum has
/// no third state to accidentally pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lifecycle {
    Standby,
    Running,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_serializes_to_lowercase_json_strings() {
        assert_eq!(
            serde_json::to_string(&Lifecycle::Standby).unwrap(),
            "\"standby\""
        );
        assert_eq!(
            serde_json::to_string(&Lifecycle::Running).unwrap(),
            "\"running\""
        );
    }
}
