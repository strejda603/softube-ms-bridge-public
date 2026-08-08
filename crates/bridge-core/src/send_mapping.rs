//! Console 1 <-> Mixing Station send-slot mapping.
//!
//! Console 1 has 6 physical send slots; Mixing Station can have up to 16 bus sends. Each C1
//! slot maps to a configured MS bus number, falling back to an identity mapping (slot i -> MS
//! send index i) if the configured bus number is missing or out of range. See `index.js`'s
//! `buildSendMapping` for the original this ports.

use std::collections::HashMap;

pub const NUMBER_OF_SENDS: usize = 6;

/// Console 1 send-slot -> Mixing Station bus-send-index mapping, plus its reverse lookup.
#[derive(Debug, Clone, PartialEq)]
pub struct SendMapping {
    /// `c1_to_ms_send_index[i]` = the MS send index Console 1 send slot `i` (0-based) maps to.
    pub c1_to_ms_send_index: [usize; NUMBER_OF_SENDS],
    /// MS send index -> the first Console 1 slot (0-based) mapped to it, if any.
    ///
    /// A `HashMap` rather than a fixed `[Option<usize>; NUMBER_OF_SENDS]` is deliberate: the
    /// key domain here is the MS send index, bounded by the *dynamic* `bus_channel_count`
    /// (which can be up to 16+), not by `NUMBER_OF_SENDS` (fixed at 6) — the 6-entry bound
    /// applies to how many values get inserted, not to the range of keys they can take.
    pub ms_send_index_to_c1_slot: HashMap<usize, usize>,
}

/// Build the send mapping from a configured `c1_send_to_ms_bus_number` list (1-based MS bus
/// numbers, one per Console 1 slot 0..5) and the current MS bus channel count.
pub fn build_send_mapping(
    c1_send_to_ms_bus_number: &[i64],
    bus_channel_count: usize,
) -> SendMapping {
    let mut c1_to_ms_send_index = [0usize; NUMBER_OF_SENDS];
    let mut ms_send_index_to_c1_slot: HashMap<usize, usize> = HashMap::new();

    for (i, slot) in c1_to_ms_send_index.iter_mut().enumerate() {
        let idx = match c1_send_to_ms_bus_number.get(i) {
            Some(&bus_num) => {
                let ms_send_index = bus_num - 1;
                if ms_send_index >= 0 && (ms_send_index as usize) < bus_channel_count {
                    ms_send_index as usize
                } else {
                    i
                }
            }
            None => i,
        };
        *slot = idx;
        ms_send_index_to_c1_slot.entry(idx).or_insert(i);
    }

    SendMapping {
        c1_to_ms_send_index,
        ms_send_index_to_c1_slot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_falls_back_to_identity_mapping() {
        let mapping = build_send_mapping(&[], 16);
        assert_eq!(mapping.c1_to_ms_send_index, [0, 1, 2, 3, 4, 5]);
        for i in 0..NUMBER_OF_SENDS {
            assert_eq!(mapping.ms_send_index_to_c1_slot.get(&i), Some(&i));
        }
    }

    #[test]
    fn configured_bus_numbers_map_slots_to_zero_based_ms_send_indices() {
        // C1 slot0 -> bus1 (ms index 0), slot1 -> bus2 (ms index 1), slot2 -> bus3 (ms index 2),
        // slot3 -> bus7 (ms index 6), slot4 -> bus9 (ms index 8), slot5 -> bus13 (ms index 12).
        let mapping = build_send_mapping(&[1, 2, 3, 7, 9, 13], 16);
        assert_eq!(mapping.c1_to_ms_send_index, [0, 1, 2, 6, 8, 12]);
        assert_eq!(mapping.ms_send_index_to_c1_slot.get(&6), Some(&3));
        assert_eq!(mapping.ms_send_index_to_c1_slot.get(&12), Some(&5));
    }

    #[test]
    fn out_of_range_bus_number_falls_back_to_identity_for_that_slot() {
        // bus_channel_count=16 means valid MS send indices are 0..15 (bus numbers 1..16).
        // Slot0 configured to bus99 (ms index 98) is out of range -> falls back to identity (0).
        let mapping = build_send_mapping(&[99, 2, 3, 4, 5, 6], 16);
        assert_eq!(mapping.c1_to_ms_send_index[0], 0);
        assert_eq!(mapping.c1_to_ms_send_index[1], 1);
    }

    #[test]
    fn zero_or_negative_bus_number_falls_back_to_identity_for_that_slot() {
        let mapping = build_send_mapping(&[0, -1, 3, 4, 5, 6], 16);
        assert_eq!(mapping.c1_to_ms_send_index[0], 0); // bus0 -> ms index -1, invalid -> identity
        assert_eq!(mapping.c1_to_ms_send_index[1], 1); // bus-1 -> ms index -2, invalid -> identity
    }

    #[test]
    fn shorter_config_array_falls_back_to_identity_for_missing_slots() {
        let mapping = build_send_mapping(&[1, 2], 16);
        assert_eq!(mapping.c1_to_ms_send_index, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn multiple_slots_mapping_to_the_same_ms_send_index_keeps_the_first() {
        // Slots 0 and 1 both configured to bus1 (ms index 0) — slot0 wins as the "primary" slot
        // for that MS send index (matches JS's `if (!msSendIndexToC1Slot.has(idx))` guard).
        let mapping = build_send_mapping(&[1, 1, 3, 4, 5, 6], 16);
        assert_eq!(mapping.c1_to_ms_send_index[0], 0);
        assert_eq!(mapping.c1_to_ms_send_index[1], 0);
        assert_eq!(mapping.ms_send_index_to_c1_slot.get(&0), Some(&0));
    }
}
