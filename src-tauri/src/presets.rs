//! Named config snapshots ("presets"), distinct from the single "live" persisted config
//! `bridge-config` manages. Each preset is a full `RuntimeConfig` snapshot plus a name/
//! save-time timestamp, stored as one JSON file per preset under `<app_config_dir>/presets/`.
//! GUI-only concept -- `bridge-cli` has no equivalent, so this lives directly in `bridge-tauri`
//! rather than a shared crate.

use bridge_config::RuntimeConfig;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One entry in the preset list -- enough to render a row without loading the full config.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetSummary {
    pub id: String,
    pub name: String,
    pub updated_at: u64,
}

/// A full preset: name/save-time metadata plus the complete config snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetPayload {
    pub name: String,
    pub saved_at: String,
    pub config: RuntimeConfig,
}

/// Sanitizes a preset name into a safe filename stem (without extension). Mirrors the old
/// Electron app's `savePreset()`: strips anything outside `[a-z0-9._ -]` (case-insensitive),
/// collapses runs of whitespace, and falls back to `"Preset"` if the result is empty.
fn sanitize_preset_filename(name: &str) -> String {
    let stripped: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ' ' | '-'))
        .collect();
    let collapsed = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        "Preset".to_string()
    } else {
        collapsed
    }
}

fn preset_file_path(presets_dir: &Path, sanitized_name: &str) -> PathBuf {
    presets_dir.join(format!("{sanitized_name}.json"))
}

/// Rejects any id that isn't a plain filename component -- contains a path separator, or
/// resolves to something other than itself via `Path::file_name()`. Defense-in-depth: `id`
/// values are supposed to always originate from a prior `list_presets_in_dir` call (safe by
/// construction), but nothing enforces that once they cross the IPC boundary into a
/// `#[tauri::command]`'s `String` argument, so validating here too closes a path-traversal
/// risk regardless of what the frontend sends.
fn is_safe_preset_id(id: &str) -> bool {
    !id.is_empty()
        && !id.contains('/')
        && !id.contains('\\')
        && Path::new(id).file_name().map(|n| n.to_str()) == Some(Some(id))
}

fn current_iso_timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn current_unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Lists every preset in `presets_dir`, sorted by display name (case-insensitive). A missing
/// directory is an empty list, not an error (matches the old app treating "no presets folder
/// yet" as "no presets"). A file that fails to read or parse is still listed (id = filename,
/// name = filename minus extension, `updated_at: 0`) rather than silently skipped -- matches
/// the old app's `listPresets()` fallback, so a corrupt file stays visible/deletable instead of
/// vanishing from the UI.
pub fn list_presets_in_dir(presets_dir: &Path) -> std::io::Result<Vec<PresetSummary>> {
    if !presets_dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(presets_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let id = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let stem = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or(&id)
            .to_string();

        let parsed = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<PresetPayload>(&raw).ok());

        let summary = match parsed {
            Some(payload) => PresetSummary {
                id: id.clone(),
                name: payload.name,
                updated_at: entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            },
            None => PresetSummary {
                id: id.clone(),
                name: stem,
                updated_at: 0,
            },
        };
        out.push(summary);
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

/// Saves a preset (overwrite-or-create) under `sanitize_preset_filename(name)`. No collision
/// check here -- callers confirm with the user first via `preset_collision_in_dir`, which
/// checks the actual target filename rather than an in-memory list of display names.
pub fn save_preset_to_dir(
    presets_dir: &Path,
    name: &str,
    config: &RuntimeConfig,
) -> std::io::Result<PresetSummary> {
    std::fs::create_dir_all(presets_dir)?;
    let display_name = if name.trim().is_empty() {
        "Preset".to_string()
    } else {
        name.trim().to_string()
    };
    let sanitized = sanitize_preset_filename(&display_name);
    let payload = PresetPayload {
        name: display_name,
        saved_at: current_iso_timestamp(),
        config: config.clone(),
    };
    let json = serde_json::to_string_pretty(&payload)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let path = preset_file_path(presets_dir, &sanitized);
    std::fs::write(&path, json)?;
    Ok(PresetSummary {
        id: format!("{sanitized}.json"),
        name: payload.name,
        updated_at: current_unix_timestamp(),
    })
}

/// Checks whether saving under `name` would overwrite an existing preset file (sanitization
/// can map two different-looking display names to the same file, so this checks the actual
/// target filename, not an in-memory list of display names). Returns the EXISTING preset's
/// own stored display name for use in a confirmation prompt (which may differ from `name`
/// itself if sanitization is what caused the collision) -- `None` if no preset exists at the
/// target path yet. A corrupt/unparseable existing file still counts as a collision (falls
/// back to the sanitized filename as its name, mirroring `list_presets_in_dir`'s own
/// corrupt-file fallback) rather than `None`, since silently overwriting a corrupt-but-visible
/// preset row is exactly the kind of silent data loss this function exists to prevent.
pub fn preset_collision_in_dir(presets_dir: &Path, name: &str) -> Option<String> {
    let display_name = if name.trim().is_empty() {
        "Preset".to_string()
    } else {
        name.trim().to_string()
    };
    let sanitized = sanitize_preset_filename(&display_name);
    let path = preset_file_path(presets_dir, &sanitized);
    let raw = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<PresetPayload>(&raw) {
        Ok(existing) => Some(existing.name),
        Err(_) => Some(sanitized),
    }
}

/// Loads a preset by id (its filename, including `.json`). A missing or corrupt file is a
/// real error -- matches the old app's `loadPreset()`, which has no try/catch and propagates.
pub fn load_preset_from_dir(presets_dir: &Path, id: &str) -> std::io::Result<PresetPayload> {
    if !is_safe_preset_id(id) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid preset id",
        ));
    }
    let raw = std::fs::read_to_string(presets_dir.join(id))?;
    serde_json::from_str(&raw)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Deletes a preset by id.
pub fn delete_preset_from_dir(presets_dir: &Path, id: &str) -> std::io::Result<()> {
    if !is_safe_preset_id(id) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid preset id",
        ));
    }
    std::fs::remove_file(presets_dir.join(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_presets_dir(unique: &str) -> PathBuf {
        std::env::temp_dir().join(format!("bridge_tauri_presets_test_{unique}"))
    }

    fn cleanup(dir: &Path) {
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn sanitize_preset_filename_strips_disallowed_characters() {
        assert_eq!(sanitize_preset_filename("My/Preset:1"), "MyPreset1");
    }

    #[test]
    fn sanitize_preset_filename_collapses_whitespace() {
        assert_eq!(sanitize_preset_filename("Studio   A"), "Studio A");
    }

    #[test]
    fn sanitize_preset_filename_falls_back_when_empty() {
        assert_eq!(sanitize_preset_filename("///"), "Preset");
        assert_eq!(sanitize_preset_filename("   "), "Preset");
    }

    #[test]
    fn list_presets_in_dir_returns_empty_for_missing_directory() {
        let dir = temp_presets_dir("missing_7a1b");
        assert_eq!(list_presets_in_dir(&dir).unwrap(), Vec::new());
    }

    #[test]
    fn save_then_list_then_load_round_trips() {
        let dir = temp_presets_dir("roundtrip_7a1b");
        cleanup(&dir);
        let config = RuntimeConfig {
            log_json: true,
            ..RuntimeConfig::default()
        };
        let summary = save_preset_to_dir(&dir, "Studio A", &config).unwrap();
        assert_eq!(summary.id, "Studio A.json");
        assert_eq!(summary.name, "Studio A");

        let listed = list_presets_in_dir(&dir).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "Studio A.json");
        assert_eq!(listed[0].name, "Studio A");

        let loaded = load_preset_from_dir(&dir, "Studio A.json").unwrap();
        assert_eq!(loaded.name, "Studio A");
        assert_eq!(loaded.config, config);

        cleanup(&dir);
    }

    #[test]
    fn save_preset_falls_back_to_default_name_when_blank() {
        let dir = temp_presets_dir("blankname_7a1b");
        cleanup(&dir);
        let summary = save_preset_to_dir(&dir, "   ", &RuntimeConfig::default()).unwrap();
        assert_eq!(summary.name, "Preset");
        assert_eq!(summary.id, "Preset.json");
        cleanup(&dir);
    }

    #[test]
    fn save_preset_overwrites_existing_file_with_same_sanitized_name() {
        let dir = temp_presets_dir("overwrite_7a1b");
        cleanup(&dir);
        save_preset_to_dir(&dir, "Studio A", &RuntimeConfig::default()).unwrap();
        let updated_config = RuntimeConfig {
            log_json: true,
            ..RuntimeConfig::default()
        };
        save_preset_to_dir(&dir, "Studio A", &updated_config).unwrap();

        let listed = list_presets_in_dir(&dir).unwrap();
        assert_eq!(listed.len(), 1);
        let loaded = load_preset_from_dir(&dir, "Studio A.json").unwrap();
        assert!(loaded.config.log_json);
        cleanup(&dir);
    }

    #[test]
    fn delete_preset_removes_the_file() {
        let dir = temp_presets_dir("delete_7a1b");
        cleanup(&dir);
        save_preset_to_dir(&dir, "Temp", &RuntimeConfig::default()).unwrap();
        delete_preset_from_dir(&dir, "Temp.json").unwrap();
        assert_eq!(list_presets_in_dir(&dir).unwrap(), Vec::new());
        cleanup(&dir);
    }

    #[test]
    fn load_preset_from_dir_errors_on_missing_file() {
        let dir = temp_presets_dir("missingload_7a1b");
        assert!(load_preset_from_dir(&dir, "nope.json").is_err());
    }

    #[test]
    fn load_preset_from_dir_rejects_path_traversal_id() {
        let dir = temp_presets_dir("traversal_load_7a1b");
        assert!(load_preset_from_dir(&dir, "../../../etc/passwd").is_err());
    }

    #[test]
    fn delete_preset_from_dir_rejects_path_traversal_id() {
        let dir = temp_presets_dir("traversal_delete_7a1b");
        assert!(delete_preset_from_dir(&dir, "../../../etc/passwd").is_err());
    }

    #[test]
    fn preset_collision_in_dir_returns_none_when_nothing_saved_yet() {
        let dir = temp_presets_dir("collision_none_7a1b");
        assert_eq!(preset_collision_in_dir(&dir, "Anything"), None);
    }

    #[test]
    fn preset_collision_in_dir_detects_exact_name_collision() {
        let dir = temp_presets_dir("collision_exact_7a1b");
        cleanup(&dir);
        save_preset_to_dir(&dir, "Studio A", &RuntimeConfig::default()).unwrap();
        assert_eq!(
            preset_collision_in_dir(&dir, "Studio A"),
            Some("Studio A".to_string())
        );
        cleanup(&dir);
    }

    #[test]
    fn preset_collision_in_dir_detects_sanitization_induced_collision() {
        let dir = temp_presets_dir("collision_sanitized_7a1b");
        cleanup(&dir);
        save_preset_to_dir(&dir, "Studio A", &RuntimeConfig::default()).unwrap();
        // Different whitespace, same sanitized filename ("Studio A.json") -- this is exactly the
        // collision class the display-name-only check in PresetsTab.svelte used to miss.
        assert_eq!(
            preset_collision_in_dir(&dir, "Studio   A"),
            Some("Studio A".to_string())
        );
        cleanup(&dir);
    }

    #[test]
    fn preset_collision_in_dir_detects_corrupt_file_by_stem() {
        let dir = temp_presets_dir("collision_corrupt_7a1b");
        cleanup(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Broken.json"), "{ not valid json").unwrap();
        assert_eq!(
            preset_collision_in_dir(&dir, "Broken"),
            Some("Broken".to_string())
        );
        cleanup(&dir);
    }

    #[test]
    fn list_presets_in_dir_falls_back_for_a_corrupt_file() {
        let dir = temp_presets_dir("corrupt_7a1b");
        cleanup(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Broken.json"), "not json").unwrap();

        let listed = list_presets_in_dir(&dir).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "Broken.json");
        assert_eq!(listed[0].name, "Broken");
        assert_eq!(listed[0].updated_at, 0);
        cleanup(&dir);
    }
}
