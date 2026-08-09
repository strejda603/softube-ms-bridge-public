//! Decision logic for Console 1's control-channel messages (RESET/ENABLE/DISABLE/handshake
//! ack). Separates "what should happen" (pure, tested here) from "actually doing it" (async
//! engine glue in `bridge-cli`, a later task) — matching this migration's established pattern
//! of keeping decision logic pure and testable even when the actions it decides on require I/O.
//!
//! See `index.js`'s `handleConsole1ControlJson` for the original this ports. Parsing
//! `activeMeters` itself lives in `parse_active_meters` in this same file; applying it
//! (`meteredObjectIds`, the metering2 subscribe request, `batchSendChangedMeters`) is
//! `runtime.rs`'s job -- see `handle_active_meters_message` there.

use crate::lifecycle::Lifecycle;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedControlMessage {
    Reset,
    Enable,
    Disable,
    HandshakeAck,
}

/// Parse a Console 1 SysEx-decoded JSON payload's control-channel shape, if it has one.
/// Returns `None` for anything else (including a track-update message, or an `activeMeters`
/// message — see `parse_active_meters`, a separate function in this same file).
pub fn parse_control_message(parsed: &Value) -> Option<ParsedControlMessage> {
    if let Some(cmd) = parsed.get("cmd").and_then(|v| v.as_str()) {
        match cmd {
            "RESET" => return Some(ParsedControlMessage::Reset),
            "ENABLE" => return Some(ParsedControlMessage::Enable),
            "DISABLE" => return Some(ParsedControlMessage::Disable),
            // Unrecognized cmd — don't early-return None here; fall through to the handshake
            // check below, matching JS's independent if-branches (index.js:3023-3079), not a
            // single terminal "cmd present means decide now" dispatch.
            _ => {}
        }
    }
    if parsed
        .get("handshake")
        .and_then(|h| h.get("ack"))
        .and_then(|a| a.as_bool())
        == Some(true)
    {
        return Some(ParsedControlMessage::HandshakeAck);
    }
    None
}

/// Parses a Console 1 SysEx-decoded JSON payload's `activeMeters` field, if present — the
/// list of track IDs hardware currently wants metered. Returns `None` if the field is absent
/// or not a JSON array; non-string entries within the array are silently skipped (matches
/// JS's `for (const trackId of parsed.activeMeters)` loop, which never validates entry type
/// before calling `getObjectIdForTrackId`, which itself coerces via `String(trackId)` -- the
/// practical distinction only matters for malformed/adversarial input, which real Console 1
/// hardware never sends).
///
/// Deliberately independent of `parse_control_message`, not a variant of
/// `ParsedControlMessage` -- see `index.js`'s `handleConsole1ControlJson` (`index.js:3023-3079`):
/// `RESET`/`ENABLE`/`DISABLE` all `return` before ever reaching the `activeMeters` check, but a
/// `handshake.ack` message does NOT `return`, so `activeMeters` can fire in the same message as
/// a handshake ack. Callers must replicate that ordering themselves (see `runtime.rs`'s
/// `handle_inbound_midi_message`) -- this function only does the parsing, not the dispatch
/// ordering.
pub fn parse_active_meters(parsed: &Value) -> Option<Vec<String>> {
    let arr = parsed.get("activeMeters")?.as_array()?;
    Some(
        arr.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAction {
    /// Disable OSD then resend the handshake (Console 1 cleared its object list, wants a
    /// full resend).
    ResendHandshake,
    /// Force a full Console 1 re-sync — only meaningful while running (there's no Mixing
    /// Station data to resync from in standby).
    ScheduleFullResync,
    /// Re-affirm just the status/Start bank — used in standby instead of a full resync, since
    /// there are no real channels to resync and `ScheduleFullResync`'s underlying mechanism
    /// would otherwise create/activate every real channel slot, undoing standby's deactivation.
    ReaffirmStatusBank,
    EnableOsd,
    DisableOsd,
    /// Only reachable via a handshake ack when running and the initial dump hasn't gone out yet.
    FinalizeInitialization,
}

/// Decide what actions a parsed control message implies, given the current bridge lifecycle
/// and whether the initial track dump has already been sent.
///
/// Actions are returned in the order the caller must execute them (e.g. Reset's `DisableOsd`
/// must run before `ResendHandshake`) — don't reorder or collect into an unordered structure.
pub fn decide_control_actions(
    msg: ParsedControlMessage,
    lifecycle: Lifecycle,
    has_sent_initial_dump: bool,
) -> Vec<ControlAction> {
    match msg {
        ParsedControlMessage::Reset => {
            let mut actions = vec![ControlAction::DisableOsd, ControlAction::ResendHandshake];
            actions.push(if lifecycle == Lifecycle::Running {
                ControlAction::ScheduleFullResync
            } else {
                ControlAction::ReaffirmStatusBank
            });
            actions
        }
        ParsedControlMessage::Enable => vec![ControlAction::EnableOsd],
        ParsedControlMessage::Disable => vec![ControlAction::DisableOsd],
        ParsedControlMessage::HandshakeAck => {
            let mut actions = vec![ControlAction::EnableOsd];
            if !has_sent_initial_dump && lifecycle == Lifecycle::Running {
                actions.push(ControlAction::FinalizeInitialization);
            }
            actions
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_reset_command() {
        assert_eq!(
            parse_control_message(&json!({"cmd": "RESET"})),
            Some(ParsedControlMessage::Reset)
        );
    }

    #[test]
    fn parse_enable_and_disable_commands() {
        assert_eq!(
            parse_control_message(&json!({"cmd": "ENABLE"})),
            Some(ParsedControlMessage::Enable)
        );
        assert_eq!(
            parse_control_message(&json!({"cmd": "DISABLE"})),
            Some(ParsedControlMessage::Disable)
        );
    }

    #[test]
    fn parse_unknown_cmd_string_returns_none() {
        assert_eq!(
            parse_control_message(&json!({"cmd": "SOMETHING_ELSE"})),
            None
        );
    }

    #[test]
    fn unrecognized_cmd_falls_through_to_handshake_check() {
        // Matches JS's independent if-branches: an unrecognized cmd doesn't early-return None,
        // it falls through to check handshake.ack, same as if cmd were absent entirely.
        assert_eq!(
            parse_control_message(&json!({"cmd": "SOMETHING_ELSE", "handshake": {"ack": true}})),
            Some(ParsedControlMessage::HandshakeAck)
        );
    }

    #[test]
    fn parse_handshake_ack_true() {
        assert_eq!(
            parse_control_message(&json!({"handshake": {"ack": true}})),
            Some(ParsedControlMessage::HandshakeAck)
        );
    }

    #[test]
    fn parse_handshake_without_ack_true_is_not_a_control_message() {
        assert_eq!(
            parse_control_message(&json!({"handshake": {"ack": false}})),
            None
        );
        assert_eq!(parse_control_message(&json!({"handshake": {}})), None);
    }

    #[test]
    fn parse_track_update_message_is_not_a_control_message() {
        assert_eq!(
            parse_control_message(&json!({"trackId": "ABCD1234", "volume": 0})),
            None
        );
    }

    #[test]
    fn parse_active_meters_extracts_track_id_list() {
        assert_eq!(
            parse_active_meters(&json!({"activeMeters": ["ABCD1234", "EFGH5678"]})),
            Some(vec!["ABCD1234".to_string(), "EFGH5678".to_string()])
        );
    }

    #[test]
    fn parse_active_meters_handles_empty_array() {
        assert_eq!(
            parse_active_meters(&json!({"activeMeters": []})),
            Some(vec![])
        );
    }

    #[test]
    fn parse_active_meters_returns_none_when_field_absent() {
        assert_eq!(parse_active_meters(&json!({"cmd": "RESET"})), None);
    }

    #[test]
    fn parse_active_meters_skips_non_string_entries() {
        assert_eq!(
            parse_active_meters(&json!({"activeMeters": ["ABCD1234", 42, null, "EFGH5678"]})),
            Some(vec!["ABCD1234".to_string(), "EFGH5678".to_string()])
        );
    }

    #[test]
    fn parse_active_meters_returns_none_when_field_is_not_an_array() {
        assert_eq!(
            parse_active_meters(&json!({"activeMeters": "not-an-array"})),
            None
        );
    }

    #[test]
    fn active_meters_can_accompany_a_handshake_ack() {
        let msg = json!({"handshake": {"ack": true}, "activeMeters": ["ABCD1234"]});
        assert_eq!(
            parse_control_message(&msg),
            Some(ParsedControlMessage::HandshakeAck)
        );
        assert_eq!(
            parse_active_meters(&msg),
            Some(vec!["ABCD1234".to_string()])
        );
    }

    #[test]
    fn reset_while_running_schedules_full_resync() {
        let actions = decide_control_actions(ParsedControlMessage::Reset, Lifecycle::Running, true);
        assert_eq!(
            actions,
            vec![
                ControlAction::DisableOsd,
                ControlAction::ResendHandshake,
                ControlAction::ScheduleFullResync
            ]
        );
    }

    #[test]
    fn reset_while_standby_reaffirms_status_bank_instead() {
        let actions = decide_control_actions(ParsedControlMessage::Reset, Lifecycle::Standby, true);
        assert_eq!(
            actions,
            vec![
                ControlAction::DisableOsd,
                ControlAction::ResendHandshake,
                ControlAction::ReaffirmStatusBank
            ]
        );
    }

    #[test]
    fn enable_just_enables_osd() {
        assert_eq!(
            decide_control_actions(ParsedControlMessage::Enable, Lifecycle::Standby, true),
            vec![ControlAction::EnableOsd]
        );
    }

    #[test]
    fn disable_just_disables_osd() {
        assert_eq!(
            decide_control_actions(ParsedControlMessage::Disable, Lifecycle::Running, true),
            vec![ControlAction::DisableOsd]
        );
    }

    #[test]
    fn handshake_ack_while_running_and_dump_not_yet_sent_finalizes_init() {
        let actions = decide_control_actions(
            ParsedControlMessage::HandshakeAck,
            Lifecycle::Running,
            false,
        );
        assert_eq!(
            actions,
            vec![
                ControlAction::EnableOsd,
                ControlAction::FinalizeInitialization
            ]
        );
    }

    #[test]
    fn handshake_ack_while_dump_already_sent_does_not_refinalize() {
        let actions =
            decide_control_actions(ParsedControlMessage::HandshakeAck, Lifecycle::Running, true);
        assert_eq!(actions, vec![ControlAction::EnableOsd]);
    }

    #[test]
    fn handshake_ack_while_standby_does_not_finalize_even_if_dump_not_sent() {
        // Same standby-leak concern as ReaffirmStatusBank above — finalizing while standby
        // would create/activate every real channel slot, undoing standby's deactivation.
        let actions = decide_control_actions(
            ParsedControlMessage::HandshakeAck,
            Lifecycle::Standby,
            false,
        );
        assert_eq!(actions, vec![ControlAction::EnableOsd]);
    }
}
