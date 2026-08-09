//! Pure helpers for the Console 1 Fader's status/Start bank (bank 0).
//!
//! Ported from `console1StatusBank.js`. Kept separate from the rest of the bridge logic so
//! this can be unit-tested without spinning up real MIDI/WS connections — same rationale as
//! the JS original.

use crate::lifecycle::Lifecycle;
use serde_json::Value;

/// A single status indicator's identity: the field name used by the live status snapshot
/// (matches `app/statusMonitor.js`'s `computeStatus()` field names, e.g. "ipad") and its
/// display label.
pub struct StatusIndicator {
    pub key: &'static str,
    pub label: &'static str,
}

/// The 7 status indicators, in the same order as the GUI topbar (Feature A) and using the
/// same field names as `app/statusMonitor.js`'s `computeStatus()` return value.
pub const STATUS_BANK_INDICATORS: [StatusIndicator; 7] = [
    StatusIndicator {
        key: "ipad",
        label: "iPad",
    },
    StatusIndicator {
        key: "spdSxPro",
        label: "SPD-SX PRO",
    },
    StatusIndicator {
        key: "midiMaestro",
        label: "MIDI Maestro",
    },
    StatusIndicator {
        key: "bomeMtp",
        label: "Bome MIDI",
    },
    StatusIndicator {
        key: "mixingStation",
        label: "Mixing Station",
    },
    StatusIndicator {
        key: "console1Osd",
        label: "C1 OSD",
    },
    StatusIndicator {
        key: "abletonLive",
        label: "Ableton",
    },
];

/// Total slot count of the fixed status/Start bank (bank 0).
pub const STATUS_BANK_SIZE: usize = 10;

/// 0-based `object_id` of the Start slot — the last slot of bank 0.
pub const START_SLOT_OBJECT_ID: usize = STATUS_BANK_SIZE - 1;

/// Number of status indicators in the first group, before the first spacer.
pub const STATUS_BANK_FIRST_GROUP_SIZE: usize = 3;

/// What kind of slot a [`StatusBankSlot`] is.
///
/// `Status`'s `key`/`label` live inside the variant itself (rather than as separate
/// `Option` fields on `StatusBankSlot`) so a slot can't be constructed in an
/// inconsistent state — e.g. `Empty` with a `key` set, or `Status` missing one. Illegal
/// states are unrepresentable by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// A status indicator slot. `key` matches `app/statusMonitor.js`'s `computeStatus()`
    /// field names (e.g. "ipad"); `label` is the display name.
    Status {
        key: &'static str,
        label: &'static str,
    },
    Empty,
    Start,
}

/// A single slot of the fixed status/Start bank.
///
/// `ms_channels`/`ms_primary` are always empty/`None` — these slots have no Mixing Station
/// channel — kept as fields (rather than omitted) to mirror the JS shape used elsewhere for
/// bank slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusBankSlot {
    pub object_id: usize,
    pub kind: SlotKind,
    pub ms_channels: Vec<u32>,
    pub ms_primary: Option<u32>,
}

/// Start/Stop toggle display: the label and color to show on the Start slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartSlotDisplay {
    pub name: &'static str,
    pub color: u32,
}

/// A hardware trigger event implied by a Start-slot `selected` field going `true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareTrigger {
    Start,
    Stop,
}

/// Build the fixed status/Start bank: the first 3 indicators, an empty spacer, the
/// remaining 4 indicators, another empty spacer, then the Start slot as the 10th/last slot.
///
/// ```
/// # use bridge_core::console1_status_bank::{build_status_bank_slots, SlotKind};
/// assert_eq!(build_status_bank_slots()[9].kind, SlotKind::Start);
/// ```
pub fn build_status_bank_slots() -> Vec<StatusBankSlot> {
    let mut slots: Vec<StatusBankSlot> = Vec::with_capacity(STATUS_BANK_SIZE);

    let (first_group, second_group) = STATUS_BANK_INDICATORS.split_at(STATUS_BANK_FIRST_GROUP_SIZE);

    for indicator in first_group {
        slots.push(StatusBankSlot {
            object_id: slots.len(),
            kind: SlotKind::Status {
                key: indicator.key,
                label: indicator.label,
            },
            ms_channels: Vec::new(),
            ms_primary: None,
        });
    }
    slots.push(StatusBankSlot {
        object_id: slots.len(),
        kind: SlotKind::Empty,
        ms_channels: Vec::new(),
        ms_primary: None,
    });
    for indicator in second_group {
        slots.push(StatusBankSlot {
            object_id: slots.len(),
            kind: SlotKind::Status {
                key: indicator.key,
                label: indicator.label,
            },
            ms_channels: Vec::new(),
            ms_primary: None,
        });
    }
    slots.push(StatusBankSlot {
        object_id: slots.len(),
        kind: SlotKind::Empty,
        ms_channels: Vec::new(),
        ms_primary: None,
    });

    slots.push(StatusBankSlot {
        object_id: slots.len(),
        kind: SlotKind::Start,
        ms_channels: Vec::new(),
        ms_primary: None,
    });

    slots
}

/// Start/Stop toggle display for the Start slot, driven by the bridge's current lifecycle
/// state.
pub fn start_slot_display_for(
    lifecycle: Lifecycle,
    main_color: u32,
    stop_color: u32,
) -> StartSlotDisplay {
    match lifecycle {
        Lifecycle::Running => StartSlotDisplay {
            name: "Stop",
            color: stop_color,
        },
        Lifecycle::Standby => StartSlotDisplay {
            name: "Start",
            color: main_color,
        },
    }
}

/// Decide what hardware trigger event (if any) a Start-slot `selected` field implies.
///
/// Only a literal JSON `true` is meaningful — same strict-equality convention as
/// [`status_slot_color_for`] and the existing bus/main sends-mode selection handling in
/// `index.js`. This is deliberate, not an oversight: `selected_value` comes straight off
/// raw incoming SysEx JSON, where any other JSON value (numbers, strings, objects, `null`,
/// or a `false` deselection) must NOT be treated as a trigger.
pub fn hardware_trigger_type_for(
    lifecycle: Lifecycle,
    selected_value: &Value,
) -> Option<HardwareTrigger> {
    if selected_value != &Value::Bool(true) {
        return None;
    }
    Some(match lifecycle {
        Lifecycle::Running => HardwareTrigger::Stop,
        Lifecycle::Standby => HardwareTrigger::Start,
    })
}

/// On/off color for a status indicator slot, given a boolean (or boolean-ish) value from a
/// live status snapshot.
///
/// Only literal JSON `true` counts as "on" — same strict-equality convention as
/// [`hardware_trigger_type_for`], so a missing/malformed status field (`null`, a stray
/// number, a string, ...) defaults to "off" rather than needing special-casing by the
/// caller.
pub fn status_slot_color_for(is_on: &Value, on_color: u32, off_color: u32) -> u32 {
    if is_on == &Value::Bool(true) {
        on_color
    } else {
        off_color
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_bank_size_and_start_slot_id_are_consistent() {
        assert_eq!(STATUS_BANK_SIZE, 10);
        assert_eq!(START_SLOT_OBJECT_ID, 9);
    }

    #[test]
    fn object_ids_match_construction_order() {
        let slots = build_status_bank_slots();
        assert_eq!(slots.len(), 10);
        for (i, slot) in slots.iter().enumerate() {
            assert_eq!(slot.object_id, i);
        }
    }

    #[test]
    fn build_status_bank_slots_layout_and_indicator_order() {
        let slots = build_status_bank_slots();
        let kinds: Vec<SlotKind> = slots.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            vec![
                SlotKind::Status {
                    key: STATUS_BANK_INDICATORS[0].key,
                    label: STATUS_BANK_INDICATORS[0].label
                },
                SlotKind::Status {
                    key: STATUS_BANK_INDICATORS[1].key,
                    label: STATUS_BANK_INDICATORS[1].label
                },
                SlotKind::Status {
                    key: STATUS_BANK_INDICATORS[2].key,
                    label: STATUS_BANK_INDICATORS[2].label
                },
                SlotKind::Empty,
                SlotKind::Status {
                    key: STATUS_BANK_INDICATORS[3].key,
                    label: STATUS_BANK_INDICATORS[3].label
                },
                SlotKind::Status {
                    key: STATUS_BANK_INDICATORS[4].key,
                    label: STATUS_BANK_INDICATORS[4].label
                },
                SlotKind::Status {
                    key: STATUS_BANK_INDICATORS[5].key,
                    label: STATUS_BANK_INDICATORS[5].label
                },
                SlotKind::Status {
                    key: STATUS_BANK_INDICATORS[6].key,
                    label: STATUS_BANK_INDICATORS[6].label
                },
                SlotKind::Empty,
                SlotKind::Start,
            ]
        );

        let status_kinds: Vec<(&str, &str)> = slots
            .iter()
            .filter_map(|s| match s.kind {
                SlotKind::Status { key, label } => Some((key, label)),
                _ => None,
            })
            .collect();
        let expected: Vec<(&str, &str)> = STATUS_BANK_INDICATORS
            .iter()
            .map(|i| (i.key, i.label))
            .collect();
        assert_eq!(status_kinds, expected);
    }

    #[test]
    fn build_status_bank_slots_first_group_size() {
        let slots = build_status_bank_slots();
        assert_eq!(
            slots[0].kind,
            SlotKind::Status {
                key: STATUS_BANK_INDICATORS[0].key,
                label: STATUS_BANK_INDICATORS[0].label,
            }
        );
        assert!(matches!(
            slots[STATUS_BANK_FIRST_GROUP_SIZE - 1].kind,
            SlotKind::Status { .. }
        ));
        assert_eq!(slots[STATUS_BANK_FIRST_GROUP_SIZE].kind, SlotKind::Empty);
    }

    #[test]
    fn build_status_bank_slots_no_ms_channel_mapping() {
        let slots = build_status_bank_slots();
        for slot in &slots {
            assert!(slot.ms_channels.is_empty());
            assert_eq!(slot.ms_primary, None);
        }
    }

    #[test]
    fn build_status_bank_slots_start_slot_id_matches_constant() {
        let slots = build_status_bank_slots();
        let start_slot = slots.iter().find(|s| s.kind == SlotKind::Start).unwrap();
        assert_eq!(start_slot.object_id, START_SLOT_OBJECT_ID);
        assert!(STATUS_BANK_INDICATORS.len() < STATUS_BANK_SIZE);
    }

    #[test]
    fn start_slot_display_standby_shows_start_with_main_color() {
        let display = start_slot_display_for(Lifecycle::Standby, 0x00a5ff, 0x0000ff);
        assert_eq!(
            display,
            StartSlotDisplay {
                name: "Start",
                color: 0x00a5ff
            }
        );
    }

    #[test]
    fn start_slot_display_running_shows_stop_with_stop_color() {
        let display = start_slot_display_for(Lifecycle::Running, 0x00a5ff, 0x0000ff);
        assert_eq!(
            display,
            StartSlotDisplay {
                name: "Stop",
                color: 0x0000ff
            }
        );
    }

    #[test]
    fn hardware_trigger_selected_true_while_standby_means_start() {
        assert_eq!(
            hardware_trigger_type_for(Lifecycle::Standby, &json!(true)),
            Some(HardwareTrigger::Start)
        );
    }

    #[test]
    fn hardware_trigger_selected_true_while_running_means_stop() {
        assert_eq!(
            hardware_trigger_type_for(Lifecycle::Running, &json!(true)),
            Some(HardwareTrigger::Stop)
        );
    }

    #[test]
    fn hardware_trigger_false_or_null_is_not_a_trigger() {
        assert_eq!(
            hardware_trigger_type_for(Lifecycle::Standby, &json!(false)),
            None
        );
        assert_eq!(
            hardware_trigger_type_for(Lifecycle::Standby, &Value::Null),
            None
        );
        assert_eq!(
            hardware_trigger_type_for(Lifecycle::Running, &json!(false)),
            None
        );
    }

    #[test]
    fn hardware_trigger_only_literal_true_triggers_not_merely_truthy() {
        assert_eq!(
            hardware_trigger_type_for(Lifecycle::Standby, &json!(1)),
            None
        );
        assert_eq!(
            hardware_trigger_type_for(Lifecycle::Standby, &json!("true")),
            None
        );
        assert_eq!(
            hardware_trigger_type_for(Lifecycle::Standby, &json!({})),
            None
        );
    }

    #[test]
    fn status_slot_color_true_means_on_color() {
        assert_eq!(
            status_slot_color_for(&json!(true), 0x00ff00, 0x0000ff),
            0x00ff00
        );
    }

    #[test]
    fn status_slot_color_false_means_off_color() {
        assert_eq!(
            status_slot_color_for(&json!(false), 0x00ff00, 0x0000ff),
            0x0000ff
        );
    }

    #[test]
    fn status_slot_color_null_means_off_color() {
        assert_eq!(
            status_slot_color_for(&Value::Null, 0x00ff00, 0x0000ff),
            0x0000ff
        );
    }

    #[test]
    fn status_slot_color_any_non_true_value_means_off_not_just_false_null() {
        assert_eq!(
            status_slot_color_for(&json!(1), 0x00ff00, 0x0000ff),
            0x0000ff
        );
        assert_eq!(
            status_slot_color_for(&json!("true"), 0x00ff00, 0x0000ff),
            0x0000ff
        );
    }
}
