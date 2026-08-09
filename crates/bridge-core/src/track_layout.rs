//! Console 1 track-layout builder: input/bus banking, stereo-linked pairs, Main placement.
//!
//! Builds the full Console 1 Fader Mk III layout (10 faders/bank): 10-wide input banks
//! starting at object_id 0, then 10-wide bus banks with Main placed once on the last bus
//! bank's 10th fader. See `index.js`'s `rebuildTrackLayout` for the original this ports.

use bridge_config::TrackOrderEntry;
use std::collections::{HashMap, HashSet};

pub const FADER_BANK_SIZE: usize = 10;
pub const INPUTS_PER_BANK: usize = 10;

/// The kind of a Console 1 fader layout slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutSlotKind {
    /// A partial-bank padding slot — no real channel behind it (not related to the
    /// removed hardware status bank; this is used for e.g. the trailing empty faders of
    /// an under-full input or bus bank).
    Empty,
    Input,
    Bus,
    Main,
}

/// One slot in the Console 1 fader layout. `ms_channels`/`ms_primary` are empty/`None` for
/// `Empty` padding slots — only Input/Bus/Main slots reference real MS channels.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutSlot {
    pub object_id: usize,
    pub kind: LayoutSlotKind,
    pub ms_channels: Vec<usize>,
    pub ms_primary: Option<usize>,
    pub pan_locked: bool,
}

/// Inputs this layout builder needs — mirrors `index.js`'s runtime-configurable track-layout
/// globals (overridable at connect time via Mixing Station's `/console/information`, see the
/// upcoming Plan 2b/2c), not fixed constants.
pub struct TrackLayoutParams<'a> {
    /// Configured input order. Entries are **0-based** MS channel indices (0..input_channel_count).
    pub input_track_order: &'a [TrackOrderEntry],
    /// Configured bus order. Entries are **1-based** bus numbers (1..=bus_channel_count) —
    /// note this differs from `input_track_order`'s 0-based convention.
    pub bus_track_order: &'a [TrackOrderEntry],
    pub input_channel_count: usize,
    /// MS channel index where bus 1 starts; bus N maps to MS channel `bus_channel_start + (N - 1)`.
    pub bus_channel_start: usize,
    pub bus_channel_count: usize,
    /// 1 or 2 MS channel indices for Main (2 = stereo L/R).
    pub main_stereo_channels: &'a [usize],
}

struct OrderedTrack {
    ms_channels: Vec<usize>,
    ms_primary: usize,
    pan_locked: bool,
}

fn push_mono_input(
    ch: i64,
    input_channel_count: usize,
    used: &mut HashSet<usize>,
    out: &mut Vec<OrderedTrack>,
) {
    if ch < 0 || (ch as usize) >= input_channel_count {
        return;
    }
    let ch = ch as usize;
    if used.contains(&ch) {
        return;
    }
    used.insert(ch);
    out.push(OrderedTrack {
        ms_channels: vec![ch],
        ms_primary: ch,
        pan_locked: false,
    });
}

fn push_stereo_input(
    left: i64,
    right: i64,
    input_channel_count: usize,
    used: &mut HashSet<usize>,
    out: &mut Vec<OrderedTrack>,
) {
    if left < 0 || right < 0 || left == right {
        return;
    }
    if (left as usize) >= input_channel_count || (right as usize) >= input_channel_count {
        return;
    }
    let (left, right) = (left as usize, right as usize);
    if used.contains(&left) || used.contains(&right) {
        return;
    }
    used.insert(left);
    used.insert(right);
    out.push(OrderedTrack {
        ms_channels: vec![left, right],
        ms_primary: left,
        pan_locked: true,
    });
}

fn push_bus_mono(
    bus_num: i64,
    bus_channel_start: usize,
    bus_channel_count: usize,
    used: &mut HashSet<usize>,
    out: &mut Vec<OrderedTrack>,
) {
    if bus_num < 1 || (bus_num as usize) > bus_channel_count {
        return;
    }
    let bus_num = bus_num as usize;
    if used.contains(&bus_num) {
        return;
    }
    used.insert(bus_num);
    let ms_ch = bus_channel_start + (bus_num - 1);
    out.push(OrderedTrack {
        ms_channels: vec![ms_ch],
        ms_primary: ms_ch,
        pan_locked: false,
    });
}

fn push_bus_stereo(
    left_bus: i64,
    right_bus: i64,
    bus_channel_start: usize,
    bus_channel_count: usize,
    used: &mut HashSet<usize>,
    out: &mut Vec<OrderedTrack>,
) {
    if left_bus < 1 || right_bus < 1 || left_bus == right_bus {
        return;
    }
    if (left_bus as usize) > bus_channel_count || (right_bus as usize) > bus_channel_count {
        return;
    }
    let (left_bus, right_bus) = (left_bus as usize, right_bus as usize);
    if used.contains(&left_bus) || used.contains(&right_bus) {
        return;
    }
    used.insert(left_bus);
    used.insert(right_bus);
    let left_ch = bus_channel_start + (left_bus - 1);
    let right_ch = bus_channel_start + (right_bus - 1);
    out.push(OrderedTrack {
        ms_channels: vec![left_ch, right_ch],
        ms_primary: left_ch,
        pan_locked: true,
    });
}

/// Build the full Console 1 track layout: input banks + bus banks (with Main placed once on
/// the last bus bank's 10th fader).
pub fn build_track_layout(params: &TrackLayoutParams) -> Vec<LayoutSlot> {
    let mut slots: Vec<LayoutSlot> = Vec::new();

    // --- Inputs ---
    let mut used_inputs: HashSet<usize> = HashSet::new();
    let mut ordered_input_tracks: Vec<OrderedTrack> = Vec::new();

    for entry in params.input_track_order {
        match *entry {
            TrackOrderEntry::Pair(l, r) => push_stereo_input(
                l,
                r,
                params.input_channel_count,
                &mut used_inputs,
                &mut ordered_input_tracks,
            ),
            TrackOrderEntry::Single(ch) => push_mono_input(
                ch,
                params.input_channel_count,
                &mut used_inputs,
                &mut ordered_input_tracks,
            ),
        }
    }
    for ch in 0..params.input_channel_count {
        if !used_inputs.contains(&ch) {
            push_mono_input(
                ch as i64,
                params.input_channel_count,
                &mut used_inputs,
                &mut ordered_input_tracks,
            );
        }
    }

    let input_banks = ordered_input_tracks.len().div_ceil(INPUTS_PER_BANK);
    for bank in 0..input_banks {
        for i in 0..INPUTS_PER_BANK {
            let input_pos = bank * INPUTS_PER_BANK + i;
            if input_pos < ordered_input_tracks.len() {
                let t = &ordered_input_tracks[input_pos];
                slots.push(LayoutSlot {
                    object_id: slots.len(),
                    kind: LayoutSlotKind::Input,
                    ms_channels: t.ms_channels.clone(),
                    ms_primary: Some(t.ms_primary),
                    pan_locked: t.pan_locked,
                });
            } else {
                slots.push(LayoutSlot {
                    object_id: slots.len(),
                    kind: LayoutSlotKind::Empty,
                    ms_channels: vec![],
                    ms_primary: None,
                    pan_locked: false,
                });
            }
        }
    }

    // --- Buses (+ Main) ---
    let mut used_bus_nums: HashSet<usize> = HashSet::new();
    let mut ordered_bus_tracks: Vec<OrderedTrack> = Vec::new();

    for entry in params.bus_track_order {
        match *entry {
            TrackOrderEntry::Pair(l, r) => push_bus_stereo(
                l,
                r,
                params.bus_channel_start,
                params.bus_channel_count,
                &mut used_bus_nums,
                &mut ordered_bus_tracks,
            ),
            TrackOrderEntry::Single(n) => push_bus_mono(
                n,
                params.bus_channel_start,
                params.bus_channel_count,
                &mut used_bus_nums,
                &mut ordered_bus_tracks,
            ),
        }
    }
    for bus_num in 1..=params.bus_channel_count {
        if !used_bus_nums.contains(&bus_num) {
            push_bus_mono(
                bus_num as i64,
                params.bus_channel_start,
                params.bus_channel_count,
                &mut used_bus_nums,
                &mut ordered_bus_tracks,
            );
        }
    }

    let bus_banks = std::cmp::max(1, (ordered_bus_tracks.len() + 1).div_ceil(FADER_BANK_SIZE));
    let mut bus_track_index = 0usize;
    for bank in 0..bus_banks {
        for i in 0..FADER_BANK_SIZE {
            let is_last_slot_of_last_bus_bank = bank == bus_banks - 1 && i == FADER_BANK_SIZE - 1;
            if is_last_slot_of_last_bus_bank {
                slots.push(LayoutSlot {
                    object_id: slots.len(),
                    kind: LayoutSlotKind::Main,
                    ms_channels: params.main_stereo_channels.to_vec(),
                    ms_primary: params.main_stereo_channels.first().copied(),
                    pan_locked: false,
                });
                continue;
            }
            if bus_track_index < ordered_bus_tracks.len() {
                let t = &ordered_bus_tracks[bus_track_index];
                bus_track_index += 1;
                slots.push(LayoutSlot {
                    object_id: slots.len(),
                    kind: LayoutSlotKind::Bus,
                    ms_channels: t.ms_channels.clone(),
                    ms_primary: Some(t.ms_primary),
                    pan_locked: t.pan_locked,
                });
            } else {
                slots.push(LayoutSlot {
                    object_id: slots.len(),
                    kind: LayoutSlotKind::Empty,
                    ms_channels: vec![],
                    ms_primary: None,
                    pan_locked: false,
                });
            }
        }
    }

    slots
}

/// Map each MS channel index to the layout slot object_ids that reference it (a channel can be
/// referenced by more than one slot, e.g. a stereo pair's two channels both point at one slot,
/// or in principle multiple slots could reference an overlapping channel).
pub fn object_ids_by_ms_channel(slots: &[LayoutSlot]) -> HashMap<usize, Vec<usize>> {
    let mut map: HashMap<usize, Vec<usize>> = HashMap::new();
    for slot in slots {
        for &ch in &slot.ms_channels {
            map.entry(ch).or_default().push(slot.object_id);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_params() -> (Vec<TrackOrderEntry>, Vec<TrackOrderEntry>, Vec<usize>) {
        (vec![], vec![], vec![70, 71])
    }

    #[test]
    fn first_real_input_slot_is_object_id_0() {
        let (input_order, bus_order, main) = base_params();
        let params = TrackLayoutParams {
            input_track_order: &input_order,
            bus_track_order: &bus_order,
            input_channel_count: 4,
            bus_channel_start: 48,
            bus_channel_count: 2,
            main_stereo_channels: &main,
        };
        let slots = build_track_layout(&params);
        assert_eq!(slots[0].kind, LayoutSlotKind::Input);
        assert_eq!(slots[0].ms_channels, vec![0]);
        assert_eq!(slots[0].ms_primary, Some(0));
    }

    #[test]
    fn unordered_input_channels_default_to_ascending_ms_channel_order() {
        let (_, bus_order, main) = base_params();
        let input_order = vec![];
        let params = TrackLayoutParams {
            input_track_order: &input_order,
            bus_track_order: &bus_order,
            input_channel_count: 3,
            bus_channel_start: 48,
            bus_channel_count: 2,
            main_stereo_channels: &main,
        };
        let slots = build_track_layout(&params);
        let input_slots: Vec<&LayoutSlot> = slots
            .iter()
            .filter(|s| s.kind == LayoutSlotKind::Input)
            .collect();
        assert_eq!(input_slots.len(), 3);
        assert_eq!(input_slots[0].ms_primary, Some(0));
        assert_eq!(input_slots[1].ms_primary, Some(1));
        assert_eq!(input_slots[2].ms_primary, Some(2));
    }

    #[test]
    fn configured_input_order_is_respected_and_unlisted_channels_appended() {
        // Channel 2 explicitly first, channel 0 and 1 unlisted -> appended in ascending order after.
        let input_order = vec![TrackOrderEntry::Single(2)];
        let bus_order = vec![];
        let main = vec![70, 71];
        let params = TrackLayoutParams {
            input_track_order: &input_order,
            bus_track_order: &bus_order,
            input_channel_count: 3,
            bus_channel_start: 48,
            bus_channel_count: 2,
            main_stereo_channels: &main,
        };
        let slots = build_track_layout(&params);
        let input_slots: Vec<&LayoutSlot> = slots
            .iter()
            .filter(|s| s.kind == LayoutSlotKind::Input)
            .collect();
        assert_eq!(input_slots[0].ms_primary, Some(2));
        assert_eq!(input_slots[1].ms_primary, Some(0));
        assert_eq!(input_slots[2].ms_primary, Some(1));
    }

    #[test]
    fn stereo_linked_input_pair_becomes_one_slot_with_pan_locked() {
        let input_order = vec![TrackOrderEntry::Pair(0, 1)];
        let bus_order = vec![];
        let main = vec![70, 71];
        let params = TrackLayoutParams {
            input_track_order: &input_order,
            bus_track_order: &bus_order,
            input_channel_count: 4,
            bus_channel_start: 48,
            bus_channel_count: 2,
            main_stereo_channels: &main,
        };
        let slots = build_track_layout(&params);
        let input_slots: Vec<&LayoutSlot> = slots
            .iter()
            .filter(|s| s.kind == LayoutSlotKind::Input)
            .collect();
        assert_eq!(input_slots[0].ms_channels, vec![0, 1]);
        assert_eq!(input_slots[0].ms_primary, Some(0));
        assert!(input_slots[0].pan_locked);
        // Channels 2 and 3 remain individual mono slots after the stereo pair.
        assert_eq!(input_slots[1].ms_channels, vec![2]);
        assert!(!input_slots[1].pan_locked);
    }

    #[test]
    fn input_banks_are_10_wide_with_empty_slots_padding_a_partial_bank() {
        let input_order = vec![];
        let bus_order = vec![];
        let main = vec![70, 71];
        let params = TrackLayoutParams {
            input_track_order: &input_order,
            bus_track_order: &bus_order,
            input_channel_count: 3, // fewer than one bank's 10 slots
            bus_channel_start: 48,
            bus_channel_count: 2,
            main_stereo_channels: &main,
        };
        let slots = build_track_layout(&params);
        // The one input bank occupies slots 0..10 (3 real + 7 empty).
        let input_bank = &slots[0..10];
        assert_eq!(
            input_bank
                .iter()
                .filter(|s| s.kind == LayoutSlotKind::Input)
                .count(),
            3
        );
        assert_eq!(
            input_bank
                .iter()
                .filter(|s| s.kind == LayoutSlotKind::Empty)
                .count(),
            7
        );
    }

    #[test]
    fn main_is_placed_once_on_the_10th_fader_of_the_last_bus_bank() {
        let input_order = vec![];
        let bus_order = vec![];
        let main = vec![70, 71];
        let params = TrackLayoutParams {
            input_track_order: &input_order,
            bus_track_order: &bus_order,
            input_channel_count: 0,
            bus_channel_start: 48,
            bus_channel_count: 2, // fewer than one bank's 10 slots -> exactly one bus bank
            main_stereo_channels: &main,
        };
        let slots = build_track_layout(&params);
        // 0 input banks (0 inputs) + 1 bus bank (10) = 10 slots total.
        assert_eq!(slots.len(), 10);
        let main_slots: Vec<&LayoutSlot> = slots
            .iter()
            .filter(|s| s.kind == LayoutSlotKind::Main)
            .collect();
        assert_eq!(main_slots.len(), 1);
        assert_eq!(main_slots[0].object_id, 9); // last slot of the single bus bank
        assert_eq!(main_slots[0].ms_channels, vec![70, 71]);
    }

    #[test]
    fn bus_stereo_pair_uses_bus_channel_start_offset() {
        let bus_order = vec![TrackOrderEntry::Pair(1, 2)];
        let input_order = vec![];
        let main = vec![70, 71];
        let params = TrackLayoutParams {
            input_track_order: &input_order,
            bus_track_order: &bus_order,
            input_channel_count: 0,
            bus_channel_start: 48,
            bus_channel_count: 4,
            main_stereo_channels: &main,
        };
        let slots = build_track_layout(&params);
        let bus_slots: Vec<&LayoutSlot> = slots
            .iter()
            .filter(|s| s.kind == LayoutSlotKind::Bus)
            .collect();
        assert_eq!(bus_slots[0].ms_channels, vec![48, 49]); // bus1 -> ch48, bus2 -> ch49
        assert!(bus_slots[0].pan_locked);
    }

    #[test]
    fn invalid_stereo_pair_entries_are_silently_dropped() {
        // left==right, and out-of-range indices, should not panic and should not produce a slot.
        let input_order = vec![TrackOrderEntry::Pair(0, 0), TrackOrderEntry::Pair(99, 100)];
        let bus_order = vec![];
        let main = vec![70, 71];
        let params = TrackLayoutParams {
            input_track_order: &input_order,
            bus_track_order: &bus_order,
            input_channel_count: 2,
            bus_channel_start: 48,
            bus_channel_count: 2,
            main_stereo_channels: &main,
        };
        let slots = build_track_layout(&params);
        let input_slots: Vec<&LayoutSlot> = slots
            .iter()
            .filter(|s| s.kind == LayoutSlotKind::Input)
            .collect();
        // Both configured pairs are invalid -> falls through to default ascending mono channels 0,1.
        assert_eq!(input_slots.len(), 2);
        assert_eq!(input_slots[0].ms_channels, vec![0]);
        assert_eq!(input_slots[1].ms_channels, vec![1]);
    }

    #[test]
    fn object_ids_by_ms_channel_maps_each_channel_to_its_slot() {
        let input_order = vec![];
        let bus_order = vec![];
        let main = vec![70, 71];
        let params = TrackLayoutParams {
            input_track_order: &input_order,
            bus_track_order: &bus_order,
            input_channel_count: 2,
            bus_channel_start: 48,
            bus_channel_count: 2,
            main_stereo_channels: &main,
        };
        let slots = build_track_layout(&params);
        let map = object_ids_by_ms_channel(&slots);
        assert_eq!(map.get(&0), Some(&vec![0])); // first input slot, object_id 0
        assert_eq!(map.get(&1), Some(&vec![1]));
    }
}
