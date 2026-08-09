import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Mirrors `bridge_config::TrackOrderEntry` -- a single channel/bus number, or a `[left,
 * right]` pair grouped into one stereo-linked Console 1 track. */
export type TrackOrderEntry = number | [number, number];

/** Mirrors `bridge_config::RuntimeConfig` (camelCase per its `#[serde(rename_all =
 * "camelCase")]`). */
export interface RuntimeConfig {
  mixingStationWsUrl: string;
  logJson: boolean;
  inputTrackOrder: TrackOrderEntry[];
  busTrackOrder: TrackOrderEntry[];
  c1SendToMsBusNumber: number[];
  metering2IntervalMs: number;
  console1MainColor: number;
  console1BusColor: number;
}

/** Mirrors `bridge_config::BridgeConfigPatch` -- every field optional, `undefined`/absent
 * means "leave this setting unchanged." */
export interface BridgeConfigPatch {
  mixingStationWsUrl?: string;
  logJson?: boolean;
  inputTrackOrder?: TrackOrderEntry[];
  busTrackOrder?: TrackOrderEntry[];
  c1SendToMsBusNumber?: number[];
  metering2IntervalMs?: number;
  console1MainColor?: number;
  console1BusColor?: number;
}

/** Mirrors `bridge_core::lifecycle::Lifecycle` (`#[serde(rename_all =
 * "lowercase")]`). */
export type Lifecycle = "standby" | "running";

/** Mirrors `bridge_core::runtime::BridgeEvent` (`#[serde(tag = "type", content = "data",
 * rename_all = "camelCase")]`, with `ConfigApplied`'s own fields also camelCase). */
export type BridgeEvent =
  | { type: "log"; data: string }
  | { type: "lifecycleChanged"; data: Lifecycle }
  | { type: "configApplied"; data: { urlChanged: boolean; anythingChanged: boolean } }
  | { type: "crashed"; data: string };

/** Mirrors `bridge_tauri::presets::PresetSummary` — metadata about a saved preset. */
export interface PresetSummary {
  id: string;
  name: string;
  updatedAt: number;
}

/** Mirrors `bridge_tauri::presets::PresetPayload` — full preset data when loading. */
export interface PresetPayload {
  name: string;
  savedAt: string;
  config: RuntimeConfig;
}

/** Mirrors `bridge_core::status_monitor::StatusSnapshot` (`#[serde(rename_all =
 * "camelCase")]`) -- one boolean per detected integration, independent of bridge lifecycle.
 * Polled every 2s Tauri-side and pushed via `status://update` only when the snapshot changes
 * -- a stable session emits nothing after the first tick, so consumers must seed from
 * `getStatus()` on mount rather than waiting for the first event. */
export interface StatusSnapshot {
  mixingStation: boolean;
  console1Osd: boolean;
}

// All commands below reject with the Rust-side error string on failure (e.g.
// `require_runtime_alive`'s "bridge runtime is no longer running").
export function lifecycleStart(patch?: BridgeConfigPatch): Promise<void> {
  return invoke("lifecycle_start", { patch: patch ?? null });
}

export function lifecycleStop(): Promise<void> {
  return invoke("lifecycle_stop");
}

export function configApply(patch: BridgeConfigPatch): Promise<void> {
  return invoke("config_apply", { patch });
}

export function getConfig(): Promise<RuntimeConfig> {
  return invoke("get_config");
}

export function bridgeStatus(): Promise<Lifecycle> {
  return invoke("bridge_status");
}

export function listPresets(): Promise<PresetSummary[]> {
  return invoke("list_presets");
}

export function savePreset(name: string, config: RuntimeConfig): Promise<PresetSummary> {
  return invoke("save_preset", { name, config });
}

export function loadPreset(id: string): Promise<PresetPayload> {
  return invoke("load_preset", { id });
}

export function deletePreset(id: string): Promise<void> {
  return invoke("delete_preset", { id });
}

export function checkPresetCollision(name: string): Promise<string | null> {
  return invoke("check_preset_collision", { name });
}

export function exportPreset(id: string): Promise<boolean> {
  return invoke("export_preset", { id });
}

export function importPreset(): Promise<PresetPayload | null> {
  return invoke("import_preset");
}

export function openPresetsFolder(): Promise<void> {
  return invoke("open_presets_folder");
}

export function openKofiPage(): Promise<void> {
  return invoke("open_kofi_page");
}

export function getStatus(): Promise<StatusSnapshot | null> {
  return invoke("get_status");
}

export function saveDraftConfig(patch: BridgeConfigPatch): Promise<void> {
  return invoke("save_draft_config", { patch });
}

export function onReady(callback: () => void): Promise<UnlistenFn> {
  return listen("bridge://ready", () => callback());
}

export function onBridgeEvent(callback: (event: BridgeEvent) => void): Promise<UnlistenFn> {
  return listen<BridgeEvent>("bridge://event", (event) => callback(event.payload));
}

export function onStatusUpdate(callback: (snapshot: StatusSnapshot) => void): Promise<UnlistenFn> {
  return listen<StatusSnapshot>("status://update", (event) => callback(event.payload));
}

// Rust's Option<String> serializes to `null` via invoke (same pattern as getStatus's
// StatusSnapshot | null above), not `undefined` -- match that here, not string | undefined.
export function getLaunchLang(): Promise<string | null> {
  return invoke("get_launch_lang");
}

/** Mirrors `bridge_tauri::cli_args::CliArgs` -- this launch's parsed CLI flags. `lang` is also
 * present on the wire but is applied separately via `getLaunchLang()`; ignored here. */
export interface LaunchArgs {
  start: boolean;
  stop: boolean;
  preset: string | null;
  ws: string | null;
  interval: number | null;
  log: boolean;
}

export function getLaunchArgs(): Promise<LaunchArgs> {
  return invoke("get_launch_args");
}
