//! Bridge runtime configuration: load from JSON file/env vars, live-apply with change detection.
//!
//! See `index.js`'s `loadBridgeConfig`/`applyRuntimeConfig`/`stableConfigKeyFromRuntime` for the
//! original this ports. Unlike the JS original (which needs a `JSON.stringify`'d cache key to
//! detect changes, since plain JS objects lack structural equality), `RuntimeConfig` here just
//! derives `PartialEq` and compares directly — same "did anything change" semantics, simpler
//! mechanism.

use serde::{Deserialize, Serialize};

/// One entry in a configured input/bus track order: either a single channel/bus number, or a
/// `[left, right]` pair grouped into one stereo-linked Console 1 track.
///
/// Lives here rather than in `bridge_core::track_layout` (where it started) because it is the
/// deserialized shape of a config field: `bridge-core` must depend on `bridge-config` for
/// `RuntimeConfig`, so the reverse edge would be a cyclic package dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TrackOrderEntry {
    Single(i64),
    Pair(i64, i64),
}

/// Live, fully-resolved runtime configuration (always has a value for every field — unlike
/// `BridgeConfigPatch`, whose fields are all optional "what to change" deltas).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeConfig {
    /// Mixing Station WebSocket URL, e.g. `ws://localhost:8080`.
    pub mixing_station_ws_url: String,
    /// Whether every WS/MIDI JSON payload gets logged (see `LOG_JSON` env var).
    pub log_json: bool,
    /// Configured input-channel order for Console 1's 10-wide input banks. See
    /// [`TrackOrderEntry`] for the single-vs-stereo-pair shape.
    pub input_track_order: Vec<TrackOrderEntry>,
    /// Configured bus order for Console 1's 10-wide bus banks (Main is placed separately).
    pub bus_track_order: Vec<TrackOrderEntry>,
    /// Console 1's 6 physical sends mapped onto Mixing Station bus numbers. Entries are
    /// **1-based** MS bus numbers, matching `send_mapping::build_send_mapping`'s convention.
    pub c1_send_to_ms_bus_number: Vec<i64>,
    /// Metering2 push interval in ms, clamped to Mixing Station's documented 30..1000 range.
    pub metering2_interval_ms: u32,
    /// Console 1 display color for the Main bus, as a 0xRRGGBB integer.
    pub console1_main_color: u32,
    /// Console 1 display color for regular buses, as a 0xRRGGBB integer.
    pub console1_bus_color: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            mixing_station_ws_url: "ws://localhost:8080".to_string(),
            log_json: false,
            input_track_order: Vec::new(),
            bus_track_order: Vec::new(),
            c1_send_to_ms_bus_number: Vec::new(),
            metering2_interval_ms: 100,
            console1_main_color: 0x00a5ff,
            console1_bus_color: 0x800080,
        }
    }
}

/// Deserializes an `Option<Vec<TrackOrderEntry>>` field leniently: each element is parsed
/// independently, and any entry that fails to parse (wrong-length array, non-integer, etc.)
/// is silently dropped instead of failing the whole config load. JS has no equivalent parse-time
/// guard; it defers validation to layout-build time in `rebuildTrackLayout` (`index.js:1329-1394`),
/// which silently ignores malformed entries. This implementation filters earlier for the same net
/// effect. Flagged as a latent gap since Plan 2a, closed here because Plan 5b's Track Layout tab
/// is the first UI to make a corrupt persisted config's blast radius directly user-reachable.
fn deserialize_lenient_track_order<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<TrackOrderEntry>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<Vec<serde_json::Value>>::deserialize(deserializer)?;
    // An all-invalid present array becomes Some(vec![]) (explicit clear), distinct from an absent
    // key (None, untouched) — this is intentional, not a bug.
    Ok(raw.map(|values| {
        values
            .into_iter()
            .filter_map(|v| serde_json::from_value::<TrackOrderEntry>(v).ok())
            .collect()
    }))
}

/// A partial config update — from a JSON config file or a live `config:apply` message. Every
/// field is `Option`; `None` means "leave this setting unchanged."
///
/// The struct-level `#[serde(default)]` is redundant today (every field is already `Option<T>`,
/// which serde special-cases as optional on its own) but is kept as a guard: if a non-`Option`
/// field is ever added to this patch type, it would otherwise become a required JSON key and
/// silently break "partial update" semantics for any caller not yet setting it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BridgeConfigPatch {
    /// New Mixing Station WebSocket URL, if changed. Blank/whitespace-only values are ignored
    /// by `apply_patch` (matching the JS original's `cfg.mixingStationWsUrl.trim()` guard).
    pub mixing_station_ws_url: Option<String>,
    /// New `log_json` setting, if changed.
    pub log_json: Option<bool>,
    /// New input-track order, if changed. See `RuntimeConfig::input_track_order`. Malformed
    /// individual entries are skipped rather than failing the whole patch -- see
    /// `deserialize_lenient_track_order`.
    #[serde(deserialize_with = "deserialize_lenient_track_order")]
    pub input_track_order: Option<Vec<TrackOrderEntry>>,
    /// New bus-track order, if changed. See `RuntimeConfig::bus_track_order`. Same lenient
    /// parsing as `input_track_order`.
    #[serde(deserialize_with = "deserialize_lenient_track_order")]
    pub bus_track_order: Option<Vec<TrackOrderEntry>>,
    /// New send-to-bus mapping, if changed. See `RuntimeConfig::c1_send_to_ms_bus_number`.
    pub c1_send_to_ms_bus_number: Option<Vec<i64>>,
    /// New metering interval in ms (pre-clamp), if changed. `f64` because it may arrive as an
    /// arbitrary JSON number; `apply_patch` clamps/truncates it via `clamp_metering_interval_ms`.
    pub metering2_interval_ms: Option<f64>,
    /// New Main-bus display color, if changed.
    pub console1_main_color: Option<u32>,
    /// New regular-bus display color, if changed.
    pub console1_bus_color: Option<u32>,
}

/// What changed as a result of applying a patch — `url_changed` is broken out separately
/// from `anything_changed` because a caller needs to know specifically whether to reconnect
/// the Mixing Station WebSocket (URL change) vs. just rebuild layout/resubscribe (any other
/// change), matching `index.js`'s `applyRuntimeConfigAndResync` dispatch.
#[derive(Debug)]
pub struct ConfigApplyResult {
    /// Whether `mixing_station_ws_url` specifically changed.
    pub url_changed: bool,
    /// Whether any field changed at all.
    pub anything_changed: bool,
}

/// Apply a patch's present fields onto a live config, returning what changed. Fields absent
/// from the patch (`None`) are left untouched, matching the JS original's per-field
/// `if (cfg.x !== undefined) ...` guards.
pub fn apply_patch(config: &mut RuntimeConfig, patch: &BridgeConfigPatch) -> ConfigApplyResult {
    let before = config.clone();

    if let Some(url) = &patch.mixing_station_ws_url {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            config.mixing_station_ws_url = trimmed.to_string();
        }
    }
    if let Some(v) = patch.log_json {
        config.log_json = v;
    }
    if let Some(v) = &patch.input_track_order {
        config.input_track_order = v.clone();
    }
    if let Some(v) = &patch.bus_track_order {
        config.bus_track_order = v.clone();
    }
    if let Some(v) = &patch.c1_send_to_ms_bus_number {
        config.c1_send_to_ms_bus_number = v.clone();
    }
    if let Some(raw) = patch.metering2_interval_ms {
        if let Some(clamped) = clamp_metering_interval_ms(raw) {
            config.metering2_interval_ms = clamped;
        }
    }
    if let Some(v) = patch.console1_main_color {
        config.console1_main_color = v;
    }
    if let Some(v) = patch.console1_bus_color {
        config.console1_bus_color = v;
    }

    let url_changed = before.mixing_station_ws_url != config.mixing_station_ws_url;
    let anything_changed = before != *config;
    ConfigApplyResult {
        url_changed,
        anything_changed,
    }
}

/// Clamp a raw `metering2IntervalMs` value to Mixing Station's documented 30..1000ms range,
/// truncating (not rounding) like the JS original. Returns `None` for non-finite input (JS
/// leaves the setting unchanged in that case rather than clamping garbage).
pub fn clamp_metering_interval_ms(raw: f64) -> Option<u32> {
    if !raw.is_finite() {
        return None;
    }
    Some(raw.trunc().clamp(30.0, 1000.0) as u32)
}

/// Parse a `BRIDGE_CONFIG_PATH`-style JSON config file's raw contents into a patch. Missing
/// keys become `None` (not an error) via `#[serde(default)]`.
pub fn parse_config_file_contents(raw: &str) -> Result<BridgeConfigPatch, serde_json::Error> {
    serde_json::from_str(raw)
}

/// Resolve the bridge's persisted config file path within a given app config directory (e.g.
/// Tauri's `app_config_dir()`). This is the GUI's own settings-persistence concern, distinct
/// from the headless CLI's `BRIDGE_CONFIG_PATH`-env-var/cwd-relative scheme in `index.js`'s
/// `loadBridgeConfig` -- the two never need to share an implementation.
pub fn config_file_path(app_config_dir: &std::path::Path) -> std::path::PathBuf {
    app_config_dir.join("bridge-config.json")
}

/// Read and parse a persisted config file into a patch. Returns `None` if the file doesn't
/// exist or fails to parse -- a missing/corrupt file just means "no saved settings yet," not a
/// fatal error (matches `index.js`'s `loadBridgeConfig`'s try/catch-and-warn behavior).
pub fn load_config_file(path: &std::path::Path) -> Option<BridgeConfigPatch> {
    let raw = std::fs::read_to_string(path).ok()?;
    parse_config_file_contents(&raw).ok()
}

/// Persist the current live config to disk as pretty-printed JSON. Writes to a temp file in
/// the same directory first, then renames over the real path. A process crash or unexpected
/// exit mid-write can never leave a truncated/corrupt `bridge-config.json` behind (the old
/// truncate-in-place `std::fs::write` could) -- the previous good file stays intact until the
/// rename succeeds. This does NOT provide a power-loss durability guarantee: without an
/// explicit `fsync` before the rename, the OS may still reorder when the temp file's data
/// actually reaches disk relative to the rename's own metadata update, so a hardware power
/// loss (as opposed to a process crash) could still lose the just-written data -- but it would
/// fall back to the previous still-intact file, never a corrupt one, so this remains strictly
/// safer than before without being a full durability guarantee. On Windows specifically, the
/// underlying `MoveFileExW` call needs delete access to the destination, so this can newly
/// *fail* (returning `Err`, corrupting nothing) if another process -- an antivirus scanner or
/// backup agent, say -- holds the destination file open without share-delete access, a case
/// the old direct `std::fs::write` would have tolerated.
pub fn save_config_file(path: &std::path::Path, config: &RuntimeConfig) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp_path = path.with_extension(format!("json.tmp.{}", std::process::id()));
    if let Err(e) = std::fs::write(&tmp_path, &json) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    std::fs::rename(&tmp_path, path)
}

/// Parse the `LOG_JSON` env var's string convention (`"1"`/`"true"` -> `Some(true)`,
/// `"0"`/`"false"` -> `Some(false)`, anything else -> `None` meaning "don't touch this setting").
pub fn parse_bool_env(raw: &str) -> Option<bool> {
    match raw.trim().to_lowercase().as_str() {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_runtime_config_matches_js_defaults() {
        let config = RuntimeConfig::default();
        assert_eq!(config.mixing_station_ws_url, "ws://localhost:8080");
        assert!(!config.log_json);
        assert_eq!(config.metering2_interval_ms, 100);
        assert_eq!(config.console1_main_color, 0x00a5ff);
        assert_eq!(config.console1_bus_color, 0x800080);
    }

    #[test]
    fn apply_patch_updates_only_present_fields() {
        let mut config = RuntimeConfig::default();
        let patch = BridgeConfigPatch {
            log_json: Some(true),
            ..Default::default()
        };
        let result = apply_patch(&mut config, &patch);
        assert!(config.log_json);
        assert_eq!(config.mixing_station_ws_url, "ws://localhost:8080"); // untouched
        assert!(!result.url_changed);
        assert!(result.anything_changed);
    }

    #[test]
    fn apply_patch_with_no_fields_reports_nothing_changed() {
        let mut config = RuntimeConfig::default();
        let patch = BridgeConfigPatch::default();
        let result = apply_patch(&mut config, &patch);
        assert!(!result.anything_changed);
        assert!(!result.url_changed);
    }

    #[test]
    fn apply_patch_with_same_value_reports_nothing_changed() {
        let mut config = RuntimeConfig::default();
        let patch = BridgeConfigPatch {
            log_json: Some(false), // already false — same value, not a change
            ..Default::default()
        };
        let result = apply_patch(&mut config, &patch);
        assert!(!result.anything_changed);
        assert!(!result.url_changed);
    }

    #[test]
    fn apply_patch_detects_url_change_specifically() {
        let mut config = RuntimeConfig::default();
        let patch = BridgeConfigPatch {
            mixing_station_ws_url: Some("ws://127.0.0.1:9000".to_string()),
            ..Default::default()
        };
        let result = apply_patch(&mut config, &patch);
        assert_eq!(config.mixing_station_ws_url, "ws://127.0.0.1:9000");
        assert!(result.url_changed);
        assert!(result.anything_changed);
    }

    #[test]
    fn apply_patch_ignores_blank_url() {
        let mut config = RuntimeConfig::default();
        let patch = BridgeConfigPatch {
            mixing_station_ws_url: Some("   ".to_string()),
            ..Default::default()
        };
        let result = apply_patch(&mut config, &patch);
        assert_eq!(config.mixing_station_ws_url, "ws://localhost:8080");
        assert!(!result.url_changed);
    }

    #[test]
    fn apply_patch_trims_url_whitespace() {
        let mut config = RuntimeConfig::default();
        let patch = BridgeConfigPatch {
            mixing_station_ws_url: Some("  ws://127.0.0.1:8080  ".to_string()),
            ..Default::default()
        };
        apply_patch(&mut config, &patch);
        assert_eq!(config.mixing_station_ws_url, "ws://127.0.0.1:8080");
    }

    #[test]
    fn apply_patch_clamps_metering_interval() {
        let mut config = RuntimeConfig::default();
        let patch = BridgeConfigPatch {
            metering2_interval_ms: Some(15.0),
            ..Default::default()
        };
        apply_patch(&mut config, &patch);
        assert_eq!(config.metering2_interval_ms, 30);

        let patch2 = BridgeConfigPatch {
            metering2_interval_ms: Some(5000.0),
            ..Default::default()
        };
        apply_patch(&mut config, &patch2);
        assert_eq!(config.metering2_interval_ms, 1000);
    }

    #[test]
    fn clamp_metering_interval_ms_truncates_not_rounds() {
        assert_eq!(clamp_metering_interval_ms(123.9), Some(123));
    }

    #[test]
    fn clamp_metering_interval_ms_rejects_non_finite() {
        assert_eq!(clamp_metering_interval_ms(f64::NAN), None);
        assert_eq!(clamp_metering_interval_ms(f64::INFINITY), None);
    }

    #[test]
    fn parse_config_file_contents_partial_json_leaves_other_fields_none() {
        let patch = parse_config_file_contents(r#"{"logJson": true}"#).unwrap();
        assert_eq!(patch.log_json, Some(true));
        assert_eq!(patch.mixing_station_ws_url, None);
    }

    #[test]
    fn parse_config_file_contents_stereo_pair_track_order() {
        let patch = parse_config_file_contents(r#"{"inputTrackOrder": [0, [1, 2], 3]}"#).unwrap();
        let order = patch.input_track_order.unwrap();
        assert_eq!(order[0], TrackOrderEntry::Single(0));
        assert_eq!(order[1], TrackOrderEntry::Pair(1, 2));
        assert_eq!(order[2], TrackOrderEntry::Single(3));
    }

    #[test]
    fn parse_config_file_contents_rejects_invalid_json() {
        assert!(parse_config_file_contents("not json").is_err());
    }

    #[test]
    fn parse_bool_env_recognizes_1_and_true() {
        assert_eq!(parse_bool_env("1"), Some(true));
        assert_eq!(parse_bool_env("true"), Some(true));
        assert_eq!(parse_bool_env("TRUE"), Some(true));
    }

    #[test]
    fn parse_bool_env_recognizes_0_and_false() {
        assert_eq!(parse_bool_env("0"), Some(false));
        assert_eq!(parse_bool_env("false"), Some(false));
    }

    #[test]
    fn parse_bool_env_unrecognized_value_returns_none() {
        assert_eq!(parse_bool_env("yes"), None);
        assert_eq!(parse_bool_env(""), None);
    }

    #[test]
    fn runtime_config_serializes_with_camel_case_field_names() {
        let config = RuntimeConfig::default();
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["mixingStationWsUrl"], "ws://localhost:8080");
        assert_eq!(json["logJson"], false);
        assert_eq!(json["metering2IntervalMs"], 100);
        assert_eq!(json["console1MainColor"], 0x00a5ff);
        assert_eq!(json["console1BusColor"], 0x800080);
    }

    #[test]
    fn runtime_config_serializes_track_order_entries() {
        let config = RuntimeConfig {
            input_track_order: vec![TrackOrderEntry::Single(0), TrackOrderEntry::Pair(1, 2)],
            ..Default::default()
        };
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["inputTrackOrder"], serde_json::json!([0, [1, 2]]));
    }

    #[test]
    fn runtime_config_round_trips_through_serde_json() {
        let config = RuntimeConfig {
            mixing_station_ws_url: "ws://192.168.1.5:8080".to_string(),
            log_json: true,
            input_track_order: vec![TrackOrderEntry::Single(0), TrackOrderEntry::Pair(1, 2)],
            bus_track_order: vec![TrackOrderEntry::Single(3)],
            c1_send_to_ms_bus_number: vec![1, 2, 3, 4, 5, 6],
            metering2_interval_ms: 250,
            console1_main_color: 0x123456,
            console1_bus_color: 0x654321,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: RuntimeConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn config_file_path_joins_app_config_dir_and_filename() {
        let dir = std::path::Path::new("/some/app/config/dir");
        assert_eq!(
            config_file_path(dir),
            std::path::PathBuf::from("/some/app/config/dir/bridge-config.json")
        );
    }

    #[test]
    fn load_config_file_returns_none_for_missing_file() {
        let path = std::env::temp_dir().join("bridge_config_test_missing_9f3a.json");
        assert!(load_config_file(&path).is_none());
    }

    #[test]
    fn load_config_file_parses_a_valid_patch() {
        let path = std::env::temp_dir().join("bridge_config_test_valid_9f3a.json");
        std::fs::write(
            &path,
            r#"{"logJson": true, "mixingStationWsUrl": "ws://127.0.0.1:9000"}"#,
        )
        .unwrap();
        let patch = load_config_file(&path).unwrap();
        assert_eq!(patch.log_json, Some(true));
        assert_eq!(
            patch.mixing_station_ws_url,
            Some("ws://127.0.0.1:9000".to_string())
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_config_file_returns_none_for_corrupt_json() {
        let path = std::env::temp_dir().join("bridge_config_test_corrupt_9f3a.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(load_config_file(&path).is_none());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn save_config_file_writes_json_that_load_config_file_can_read_back() {
        let path = std::env::temp_dir().join("bridge_config_test_roundtrip_9f3a.json");
        let config = RuntimeConfig {
            mixing_station_ws_url: "ws://192.168.1.5:8080".to_string(),
            log_json: true,
            ..RuntimeConfig::default()
        };
        save_config_file(&path, &config).unwrap();
        let patch = load_config_file(&path).unwrap();
        assert_eq!(
            patch.mixing_station_ws_url,
            Some("ws://192.168.1.5:8080".to_string())
        );
        assert_eq!(patch.log_json, Some(true));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn save_config_file_leaves_no_temp_file_behind() {
        let dir = std::env::temp_dir().join(format!("bridge_config_atomic_write_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = config_file_path(&dir);

        save_config_file(&path, &RuntimeConfig::default()).unwrap();

        let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(entries, [std::ffi::OsString::from("bridge-config.json")]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_config_file_leaves_previous_file_intact_on_write_failure() {
        let dir = std::env::temp_dir().join(format!("bridge_config_atomic_write_failure_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = config_file_path(&dir);

        // Establish a known-good baseline with a real successful save.
        let good_config = RuntimeConfig {
            mixing_station_ws_url: "ws://10.0.0.1:8080".to_string(),
            ..RuntimeConfig::default()
        };
        save_config_file(&path, &good_config).unwrap();

        // Force the temp write to fail by pre-creating a DIRECTORY at the exact temp path
        // save_config_file will use -- std::fs::write on a directory path always fails (EISDIR
        // on POSIX, an access-denied-class error on Windows), on both platforms.
        let tmp_path = path.with_extension(format!("json.tmp.{}", std::process::id()));
        std::fs::create_dir_all(&tmp_path).unwrap();

        let bad_config = RuntimeConfig {
            mixing_station_ws_url: "ws://this-should-never-be-saved:1".to_string(),
            ..RuntimeConfig::default()
        };
        let result = save_config_file(&path, &bad_config);
        assert!(result.is_err());

        // The ORIGINAL file must be untouched -- this is exactly the property that
        // distinguishes atomic-write from the old truncate-in-place implementation, which
        // would have already destroyed the good file before ever hitting this error.
        let patch = load_config_file(&path).unwrap();
        let mut reconstructed = RuntimeConfig::default();
        apply_patch(&mut reconstructed, &patch);
        assert_eq!(reconstructed.mixing_station_ws_url, "ws://10.0.0.1:8080");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn patch_input_track_order_skips_malformed_entries_but_keeps_valid_ones() {
        let patch = parse_config_file_contents(
            r#"{"inputTrackOrder": [0, [1, 2], "not a number", [1, 2, 3], 5]}"#,
        )
        .unwrap();
        assert_eq!(
            patch.input_track_order.unwrap(),
            vec![
                TrackOrderEntry::Single(0),
                TrackOrderEntry::Pair(1, 2),
                TrackOrderEntry::Single(5),
            ]
        );
    }

    #[test]
    fn patch_bus_track_order_skips_malformed_entries_but_keeps_valid_ones() {
        let patch =
            parse_config_file_contents(r#"{"busTrackOrder": [1, [2, 3], null, [4, 5, 6]]}"#)
                .unwrap();
        assert_eq!(
            patch.bus_track_order.unwrap(),
            vec![TrackOrderEntry::Single(1), TrackOrderEntry::Pair(2, 3)]
        );
    }

    #[test]
    fn patch_missing_track_order_fields_stay_none() {
        let patch = parse_config_file_contents(r#"{"logJson": true}"#).unwrap();
        assert!(patch.input_track_order.is_none());
        assert!(patch.bus_track_order.is_none());
    }

    #[test]
    fn patch_all_malformed_entries_becomes_some_empty_vec_not_none() {
        let patch = parse_config_file_contents(
            r#"{"inputTrackOrder": ["a", "b", [1, 2, 3]], "busTrackOrder": [null, "x"]}"#,
        )
        .unwrap();
        // An explicitly-sent array with only malformed entries becomes Some(vec![]) (explicit
        // clear), distinct from an absent key (None, untouched). This is intentional, matching
        // the distinction between `{"inputTrackOrder": []}` and `{}`.
        assert_eq!(patch.input_track_order, Some(vec![]));
        assert_eq!(patch.bus_track_order, Some(vec![]));
    }
}
