//! Live per-slot Console 1 track state: default-track construction, naming, and the cache
//! that holds every slot's current `TrackInfo`.
//!
//! See `index.js`'s `createDefaultTrackForSlot`/`getDefaultNameForObjectId`/
//! `getOrCreateTrackInfo`/`resetRealChannelTrackCache` for the originals this ports.

use crate::lifecycle::Lifecycle;
use crate::dsp_field_metadata::headamp_gain_db_to_normalized;
use crate::sysex::ConsoleTrackFields;
use crate::track_id::TrackIdRegistry;
use crate::track_layout::{LayoutSlot, LayoutSlotKind};
use rand::Rng;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Live per-slot Console 1 track state — mirrors `createDefaultTrackForSlot`'s returned object.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackInfo {
    pub track: i64,
    pub is_active: bool,
    pub track_id: String,
    pub color: u32,
    pub name: String,
    /// Number, or the JSON string `"-Infinity"` for silence.
    pub volume: Value,
    pub meter: Vec<f64>,
    pub mute: bool,
    pub solo: bool,
    pub selected: bool,
    pub max_volume_value: f64,
    pub max_send_value: f64,
    pub pan: f64,
    /// send1..6 levels (index 0 = send1).
    pub send_levels: [Value; 6],
    /// send1On..6On (index 0 = send1On).
    pub send_on: [bool; 6],
    /// Filter/EQ/Compressor 0..1 sync values, keyed by bare field name (e.g. `"filterLcFreq"`,
    /// `"eq2Gain"`, `"compRatio"`) — see `dsp_field_metadata::dsp_field_metadata` for the fixed
    /// field set. Never sent via `trackBatch` — see `sysex::ConsoleTrackFields`'s doc comment.
    pub dsp_fields: HashMap<String, Value>,
    /// Real-unit ("val"-format) companion values for continuous DSP fields, display-only — see
    /// `dsp_field_metadata::format_console1_dsp_display_value`'s `real_val` parameter.
    pub dsp_real_values: HashMap<String, f64>,
}

/// Project a `TrackInfo` down onto the wire-safe `trackBatch` field set. `dsp_fields`/
/// `dsp_real_values` are dropped here, not merely left unmapped — `ConsoleTrackFields` is
/// structurally incapable of holding them, which is the whole point: this conversion is the
/// single place that decides what's safe to put in a `trackBatch` SysEx message, so a future
/// `TrackInfo` field rename can't silently reintroduce a disallowed field onto the wire without
/// a compile error here.
impl From<&TrackInfo> for ConsoleTrackFields {
    fn from(track: &TrackInfo) -> Self {
        ConsoleTrackFields {
            track: track.track,
            is_active: track.is_active,
            track_id: track.track_id.clone(),
            color: track.color,
            name: track.name.clone(),
            volume: track.volume.clone(),
            meter: track.meter.clone(),
            mute: track.mute,
            solo: track.solo,
            selected: track.selected,
            max_volume_value: track.max_volume_value,
            max_send_value: track.max_send_value,
            pan: track.pan,
            send1: track.send_levels[0].clone(),
            send1_on: track.send_on[0],
            send2: track.send_levels[1].clone(),
            send2_on: track.send_on[1],
            send3: track.send_levels[2].clone(),
            send3_on: track.send_on[2],
            send4: track.send_levels[3].clone(),
            send4_on: track.send_on[3],
            send5: track.send_levels[4].clone(),
            send5_on: track.send_on[4],
            send6: track.send_levels[5].clone(),
            send6_on: track.send_on[5],
        }
    }
}

/// Colors used when building a slot's default `TrackInfo`.
pub struct DefaultTrackColors {
    pub bus_color: u32,
    pub main_color: u32,
}

/// Compute a layout slot's default display name (the fallback used when Mixing Station
/// provides an empty channel name).
pub fn default_name_for_slot(slot: &LayoutSlot, bus_channel_start: usize) -> String {
    match slot.kind {
        LayoutSlotKind::Input => {
            if slot.ms_channels.len() == 2 {
                format!("Ch {}+{}", slot.ms_channels[0] + 1, slot.ms_channels[1] + 1)
            } else if let Some(p) = slot.ms_primary {
                format!("Ch {}", p + 1)
            } else {
                String::new()
            }
        }
        LayoutSlotKind::Bus => {
            if slot.ms_channels.len() == 2 {
                format!(
                    "Bus {}+{}",
                    slot.ms_channels[0] - bus_channel_start + 1,
                    slot.ms_channels[1] - bus_channel_start + 1
                )
            } else if let Some(p) = slot.ms_primary {
                format!("Bus {}", p - bus_channel_start + 1)
            } else {
                String::new()
            }
        }
        LayoutSlotKind::Main => "Main".to_string(),
        _ => String::new(),
    }
}

fn default_dsp_fields() -> HashMap<String, Value> {
    let mut m = HashMap::new();
    m.insert("filterLcOn".to_string(), json!(false));
    m.insert("filterLcFreq".to_string(), json!(0));
    m.insert(
        "filterPreGain".to_string(),
        json!(headamp_gain_db_to_normalized(0.0)),
    );
    m.insert("filterPhaseInvert".to_string(), json!(false));
    for n in 1..=4 {
        m.insert(format!("eq{n}On"), json!(false));
        m.insert(format!("eq{n}Freq"), json!(0.5));
        m.insert(format!("eq{n}Gain"), json!(0.5));
        m.insert(format!("eq{n}Q"), json!(0.5));
        m.insert(format!("eq{n}Type"), json!(0));
    }
    m.insert("compOn".to_string(), json!(false));
    m.insert("compRatio".to_string(), json!(0));
    m.insert("compAttack".to_string(), json!(0));
    m.insert("compRelease".to_string(), json!(0));
    m.insert("compMakeup".to_string(), json!(0));
    m.insert("compComp".to_string(), json!(0));
    m.insert("compKnee".to_string(), json!(0));
    m.insert("compWetdry".to_string(), json!(0));
    m
}

/// Build the default Console 1 track object for a freshly-created layout slot.
pub fn create_default_track_for_slot(
    slot: &LayoutSlot,
    track_id: String,
    _lifecycle: Lifecycle,
    colors: &DefaultTrackColors,
    bus_channel_start: usize,
) -> TrackInfo {
    let mut is_active = true;
    let mut color = 6842214u32;
    let mut name = default_name_for_slot(slot, bus_channel_start);
    let mut send_on = [false; 6];

    match slot.kind {
        LayoutSlotKind::Bus => color = colors.bus_color,
        LayoutSlotKind::Main => color = colors.main_color,
        LayoutSlotKind::Empty => is_active = false,
        LayoutSlotKind::Input => {}
    }

    TrackInfo {
        track: slot.object_id as i64 + 1,
        is_active,
        track_id,
        color,
        name,
        volume: json!(0),
        meter: vec![0.0],
        mute: false,
        solo: false,
        selected: false,
        max_volume_value: 10.0,
        max_send_value: 10.0,
        pan: 0.5,
        send_levels: [json!(0), json!(0), json!(0), json!(0), json!(0), json!(0)],
        send_on,
        dsp_fields: default_dsp_fields(),
        dsp_real_values: HashMap::new(),
    }
}

/// The full per-slot track cache, plus the trackId registry that backs it.
#[derive(Debug, Default)]
pub struct TrackCache {
    tracks: HashMap<usize, TrackInfo>,
    track_ids: TrackIdRegistry,
}

impl TrackCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the cached track for a slot, creating its default state on first access.
    pub fn get_or_create(
        &mut self,
        object_id: usize,
        slot: &LayoutSlot,
        lifecycle: Lifecycle,
        colors: &DefaultTrackColors,
        bus_channel_start: usize,
        rng: &mut impl Rng,
    ) -> &TrackInfo {
        if !self.tracks.contains_key(&object_id) {
            let track_id = self.track_ids.get_or_create(object_id, rng);
            let track =
                create_default_track_for_slot(slot, track_id, lifecycle, colors, bus_channel_start);
            self.tracks.insert(object_id, track);
        }
        self.tracks
            .get(&object_id)
            .expect("just inserted or already present")
    }

    /// Get the cached track for a slot, if one has been created yet.
    pub fn get(&self, object_id: usize) -> Option<&TrackInfo> {
        self.tracks.get(&object_id)
    }

    /// Get a mutable reference to the cached track for a slot, if one has been created yet.
    pub fn get_mut(&mut self, object_id: usize) -> Option<&mut TrackInfo> {
        self.tracks.get_mut(&object_id)
    }

    /// Resolve a Console 1 `trackId` back to its objectId, if known.
    pub fn object_id_for_track_id(&self, track_id: &str) -> Option<usize> {
        self.track_ids.object_id_for_track_id(track_id)
    }

    /// Drop every cached track. Used entering standby and on layout rebuild.
    pub fn clear(&mut self) {
        self.tracks.clear();
    }

    /// Iterate over every currently-cached `(object_id, track)` pair.
    pub fn iter(&self) -> impl Iterator<Item = (&usize, &TrackInfo)> {
        self.tracks.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn colors() -> DefaultTrackColors {
        DefaultTrackColors {
            bus_color: 0x800080,
            main_color: 0x00a5ff,
        }
    }

    fn input_slot(object_id: usize, ms_channels: Vec<usize>) -> LayoutSlot {
        LayoutSlot {
            object_id,
            kind: LayoutSlotKind::Input,
            ms_primary: ms_channels.first().copied(),
            ms_channels,
            pan_locked: false,
        }
    }

    #[test]
    fn default_name_mono_input_slot() {
        let slot = input_slot(10, vec![0]);
        assert_eq!(default_name_for_slot(&slot, 48), "Ch 1");
    }

    #[test]
    fn default_name_stereo_input_slot() {
        let slot = input_slot(10, vec![4, 5]);
        assert_eq!(default_name_for_slot(&slot, 48), "Ch 5+6");
    }

    #[test]
    fn default_name_mono_bus_slot_uses_bus_channel_start_offset() {
        let slot = LayoutSlot {
            object_id: 20,
            kind: LayoutSlotKind::Bus,
            ms_channels: vec![48],
            ms_primary: Some(48),
            pan_locked: false,
        };
        assert_eq!(default_name_for_slot(&slot, 48), "Bus 1");
    }

    #[test]
    fn default_name_stereo_bus_slot() {
        let slot = LayoutSlot {
            object_id: 20,
            kind: LayoutSlotKind::Bus,
            ms_channels: vec![48, 49],
            ms_primary: Some(48),
            pan_locked: true,
        };
        assert_eq!(default_name_for_slot(&slot, 48), "Bus 1+2");
    }

    #[test]
    fn default_name_main_slot_is_literally_main() {
        let slot = LayoutSlot {
            object_id: 29,
            kind: LayoutSlotKind::Main,
            ms_channels: vec![70, 71],
            ms_primary: Some(70),
            pan_locked: false,
        };
        assert_eq!(default_name_for_slot(&slot, 48), "Main");
    }

    #[test]
    fn default_name_empty_slot_is_blank() {
        let slot = LayoutSlot {
            object_id: 15,
            kind: LayoutSlotKind::Empty,
            ms_channels: vec![],
            ms_primary: None,
            pan_locked: false,
        };
        assert_eq!(default_name_for_slot(&slot, 48), "");
    }

    #[test]
    fn create_default_track_input_slot_active_default_color() {
        let slot = input_slot(10, vec![0]);
        let track = create_default_track_for_slot(
            &slot,
            "ABCD1234".into(),
            Lifecycle::Standby,
            &colors(),
            48,
        );
        assert!(track.is_active);
        assert_eq!(track.color, 6842214);
        assert_eq!(track.name, "Ch 1");
        assert_eq!(track.track, 11); // object_id 10 -> 1-based track 11
        assert_eq!(track.track_id, "ABCD1234");
        assert_eq!(track.pan, 0.5);
        assert_eq!(track.max_volume_value, 10.0);
        assert!(track.send_on.iter().all(|&on| !on));
    }

    #[test]
    fn create_default_track_empty_slot_is_inactive() {
        let slot = LayoutSlot {
            object_id: 15,
            kind: LayoutSlotKind::Empty,
            ms_channels: vec![],
            ms_primary: None,
            pan_locked: false,
        };
        let track =
            create_default_track_for_slot(&slot, "X".into(), Lifecycle::Standby, &colors(), 48);
        assert!(!track.is_active);
    }

    #[test]
    fn create_default_track_bus_slot_uses_bus_color() {
        let slot = LayoutSlot {
            object_id: 20,
            kind: LayoutSlotKind::Bus,
            ms_channels: vec![48],
            ms_primary: Some(48),
            pan_locked: false,
        };
        let c = colors();
        let track = create_default_track_for_slot(&slot, "X".into(), Lifecycle::Standby, &c, 48);
        assert_eq!(track.color, c.bus_color);
    }

    #[test]
    fn create_default_track_main_slot_uses_main_color() {
        let slot = LayoutSlot {
            object_id: 29,
            kind: LayoutSlotKind::Main,
            ms_channels: vec![70, 71],
            ms_primary: Some(70),
            pan_locked: false,
        };
        let c = colors();
        let track = create_default_track_for_slot(&slot, "X".into(), Lifecycle::Standby, &c, 48);
        assert_eq!(track.color, c.main_color);
    }

    #[test]
    fn create_default_track_dsp_fields_have_correct_defaults() {
        let slot = input_slot(10, vec![0]);
        let track =
            create_default_track_for_slot(&slot, "X".into(), Lifecycle::Standby, &colors(), 48);
        assert_eq!(track.dsp_fields["filterLcOn"], json!(false));
        assert_eq!(track.dsp_fields["filterLcFreq"], json!(0));
        assert_eq!(
            track.dsp_fields["filterPreGain"],
            json!(headamp_gain_db_to_normalized(0.0))
        );
        assert_eq!(track.dsp_fields["eq1Freq"], json!(0.5));
        assert_eq!(track.dsp_fields["eq4Type"], json!(0));
        assert_eq!(track.dsp_fields["compKnee"], json!(0));
        assert!(track.dsp_real_values.is_empty());
    }

    #[test]
    fn default_dsp_fields_key_set_matches_dsp_field_metadata_names() {
        use std::collections::HashSet;
        let defaults = default_dsp_fields();
        let default_keys: HashSet<&str> = defaults.keys().map(|s| s.as_str()).collect();
        let known_names: HashSet<&str> = crate::dsp_field_metadata::DSP_FIELD_NAMES
            .iter()
            .copied()
            .collect();
        assert_eq!(default_keys, known_names, "default_dsp_fields()'s keys must exactly match dsp_field_metadata::DSP_FIELD_NAMES — a mismatch means the two lists have drifted apart");
    }

    #[test]
    fn cache_get_or_create_is_idempotent_and_generates_a_track_id() {
        let mut rng = StdRng::seed_from_u64(10);
        let mut cache = TrackCache::new();
        let slot = input_slot(10, vec![0]);
        let c = colors();
        let first = cache
            .get_or_create(10, &slot, Lifecycle::Standby, &c, 48, &mut rng)
            .clone();
        let second = cache
            .get_or_create(10, &slot, Lifecycle::Standby, &c, 48, &mut rng)
            .clone();
        assert_eq!(first, second);
        assert_eq!(first.track_id.len(), 8);
    }

    #[test]
    fn cache_object_id_for_track_id_resolves_after_creation() {
        let mut rng = StdRng::seed_from_u64(11);
        let mut cache = TrackCache::new();
        let slot = input_slot(10, vec![0]);
        let c = colors();
        let track = cache
            .get_or_create(10, &slot, Lifecycle::Standby, &c, 48, &mut rng)
            .clone();
        assert_eq!(cache.object_id_for_track_id(&track.track_id), Some(10));
    }

    #[test]
    fn cache_clear_drops_all_cached_tracks() {
        let mut rng = StdRng::seed_from_u64(12);
        let mut cache = TrackCache::new();
        let c = colors();
        let slot_a = input_slot(0, vec![0]);
        let slot_b = input_slot(10, vec![10]);
        cache.get_or_create(0, &slot_a, Lifecycle::Standby, &c, 48, &mut rng);
        cache.get_or_create(10, &slot_b, Lifecycle::Standby, &c, 48, &mut rng);

        cache.clear();

        assert!(cache.get(0).is_none());
        assert!(cache.get(10).is_none());
    }

    #[test]
    fn console_track_fields_from_track_info_maps_every_field_and_drops_dsp_state() {
        let slot = input_slot(10, vec![0]);
        let mut track = create_default_track_for_slot(
            &slot,
            "ABCD1234".into(),
            Lifecycle::Standby,
            &colors(),
            48,
        );
        track.mute = true;
        track.pan = 0.25;
        track.send_levels[2] = json!(7);
        track.send_on[2] = true;

        let fields = ConsoleTrackFields::from(&track);
        assert_eq!(fields.track, track.track);
        assert_eq!(fields.is_active, track.is_active);
        assert_eq!(fields.track_id, track.track_id);
        assert_eq!(fields.color, track.color);
        assert_eq!(fields.name, track.name);
        assert_eq!(fields.volume, track.volume);
        assert_eq!(fields.meter, track.meter);
        assert_eq!(fields.mute, track.mute);
        assert_eq!(fields.solo, track.solo);
        assert_eq!(fields.selected, track.selected);
        assert_eq!(fields.max_volume_value, track.max_volume_value);
        assert_eq!(fields.max_send_value, track.max_send_value);
        assert_eq!(fields.pan, track.pan);
        assert_eq!(fields.send1, track.send_levels[0]);
        assert_eq!(fields.send1_on, track.send_on[0]);
        assert_eq!(fields.send3, track.send_levels[2]);
        assert_eq!(fields.send3_on, track.send_on[2]);
        assert_eq!(fields.send6, track.send_levels[5]);
        assert_eq!(fields.send6_on, track.send_on[5]);

        // The wire-safe projection never carries DSP state — confirmed via the serialized
        // JSON's key set, matching `sysex::ConsoleTrackFields`'s own
        // `console_track_fields_serializes_to_exactly_the_allowed_keys` test.
        let value = serde_json::to_value(&fields).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert!(!keys
            .iter()
            .any(|k| k.starts_with("filter") || k.starts_with("eq") || k.starts_with("comp")));
    }
}
