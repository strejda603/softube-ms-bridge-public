//! Console 1 `trackId` generation and objectId<->trackId bookkeeping.
//!
//! See `index.js`'s `generateUniqueTrackId8Hex`/`getOrCreateTrackIdForObjectId` for the
//! original this ports. Unlike JS (which keeps two separately-updated maps plus a linear-scan
//! fallback for cache misses), this keeps one true bidirectional registry, updated atomically.

use rand::{Rng, RngExt};
use std::collections::{HashMap, HashSet};

/// Generate an 8-hex-char uppercase trackId not already present in `used`, retrying on
/// collision (32-bit keyspace, collisions are exceedingly rare but guarded against anyway,
/// matching the JS original's own retry loop).
pub fn generate_unique_track_id_8_hex(rng: &mut impl Rng, used: &mut HashSet<String>) -> String {
    loop {
        let bytes: [u8; 4] = rng.random();
        let id = format!(
            "{:02X}{:02X}{:02X}{:02X}",
            bytes[0], bytes[1], bytes[2], bytes[3]
        );
        if !used.contains(&id) {
            used.insert(id.clone());
            return id;
        }
    }
}

/// Stable objectId<->trackId bookkeeping for the lifetime of the process.
///
/// Collision-avoidance in `get_or_create` is scoped to currently-registered ids. If a future
/// method clears/resets `object_id_by_track_id`, it must NOT shrink the collision-avoidance pool
/// without also replicating JS's separate, permanent `usedTrackIds` Set — see `index.js:462-463`'s
/// explicit "intentionally do NOT clear" rationale: reusing a trackId across a live layout
/// rebuild within the same process is a real risk to avoid, not just a JS implementation detail.
#[derive(Debug, Default)]
pub struct TrackIdRegistry {
    track_id_by_object_id: HashMap<usize, String>,
    object_id_by_track_id: HashMap<String, usize>,
}

impl TrackIdRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the stable trackId for a layout slot, generating and registering one on first call.
    pub fn get_or_create(&mut self, object_id: usize, rng: &mut impl Rng) -> String {
        if let Some(existing) = self.track_id_by_object_id.get(&object_id) {
            return existing.clone();
        }
        let id = loop {
            let bytes: [u8; 4] = rng.random();
            let candidate = format!(
                "{:02X}{:02X}{:02X}{:02X}",
                bytes[0], bytes[1], bytes[2], bytes[3]
            );
            if !self.object_id_by_track_id.contains_key(&candidate) {
                break candidate;
            }
        };
        self.track_id_by_object_id.insert(object_id, id.clone());
        self.object_id_by_track_id.insert(id.clone(), object_id);
        id
    }

    /// Resolve a Console 1 `trackId` back to its objectId, if known.
    pub fn object_id_for_track_id(&self, track_id: &str) -> Option<usize> {
        self.object_id_by_track_id.get(track_id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn generate_unique_track_id_is_8_uppercase_hex_chars() {
        let mut rng = StdRng::seed_from_u64(1);
        let mut used = HashSet::new();
        let id = generate_unique_track_id_8_hex(&mut rng, &mut used);
        assert_eq!(id.len(), 8);
        assert!(id
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase()));
    }

    #[test]
    fn generate_unique_track_id_registers_itself_in_used() {
        let mut rng = StdRng::seed_from_u64(2);
        let mut used = HashSet::new();
        let id = generate_unique_track_id_8_hex(&mut rng, &mut used);
        assert!(used.contains(&id));
    }

    #[test]
    fn generate_unique_track_id_retries_on_forced_collision() {
        // Draw once to learn what a fresh seed produces, then re-seed identically but
        // pre-populate `used` with that exact value — forcing the loop to retry at least once
        // and produce something different, proving the retry-on-collision path actually works.
        let mut rng1 = StdRng::seed_from_u64(42);
        let mut used1 = HashSet::new();
        let first = generate_unique_track_id_8_hex(&mut rng1, &mut used1);

        let mut rng2 = StdRng::seed_from_u64(42);
        let mut used2 = HashSet::new();
        used2.insert(first.clone());
        let second = generate_unique_track_id_8_hex(&mut rng2, &mut used2);

        assert_ne!(first, second);
        assert!(used2.contains(&second));
    }

    #[test]
    fn registry_get_or_create_is_idempotent_for_same_object_id() {
        let mut rng = StdRng::seed_from_u64(3);
        let mut registry = TrackIdRegistry::new();
        let first = registry.get_or_create(5, &mut rng);
        let second = registry.get_or_create(5, &mut rng);
        assert_eq!(first, second);
    }

    #[test]
    fn registry_get_or_create_differs_across_object_ids() {
        let mut rng = StdRng::seed_from_u64(4);
        let mut registry = TrackIdRegistry::new();
        let a = registry.get_or_create(0, &mut rng);
        let b = registry.get_or_create(1, &mut rng);
        assert_ne!(a, b);
    }

    #[test]
    fn registry_resolves_track_id_back_to_object_id() {
        let mut rng = StdRng::seed_from_u64(5);
        let mut registry = TrackIdRegistry::new();
        let id = registry.get_or_create(7, &mut rng);
        assert_eq!(registry.object_id_for_track_id(&id), Some(7));
    }

    #[test]
    fn registry_unknown_track_id_resolves_to_none() {
        let registry = TrackIdRegistry::new();
        assert_eq!(registry.object_id_for_track_id("DEADBEEF"), None);
    }
}
