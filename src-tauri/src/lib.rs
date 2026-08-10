//! Tauri desktop shell for Softube MS Bridge. Spawns `bridge_core::runtime` as a managed
//! background task and exposes it to the webview via commands + events.
mod cli_args;
mod presets;
mod status_gather;
mod update_checker;

use bridge_config::{BridgeConfigPatch, RuntimeConfig};
use bridge_core::lifecycle::Lifecycle;
use bridge_core::runtime::{spawn_bridge_runtime, BridgeCommand, BridgeEvent};
use bridge_core::status_monitor::StatusSnapshot;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{mpsc, Notify};

/// Tauri-side mirror of what we've told the runtime. `bridge_core::runtime` doesn't expose a
/// way to read its private state back out, so this crate tracks its own copy: `config` is
/// updated locally (via `bridge_config::apply_patch`) every time we send a patch, and
/// `lifecycle` is updated optimistically at send time by `lifecycle_start`/`lifecycle_stop`
/// (the runtime only ever changes lifecycle via those two command arms, and processes commands
/// strictly FIFO) plus forced to `Standby` once the runtime is gone -- NOT from the runtime's
/// own `BridgeEvent::LifecycleChanged` events, which the events-forwarder loop just relays to
/// the frontend without writing back to this mirror, since that write could lag behind and
/// clobber a newer optimistic update with a stale value.
struct AppState {
    command_tx: mpsc::UnboundedSender<BridgeCommand>,
    config: Mutex<RuntimeConfig>,
    lifecycle: Mutex<Lifecycle>,
    /// `false` once the runtime task has exited for any reason (graceful shutdown, a caught
    /// panic, or a silent self-exit e.g. the MIDI thread dying without panicking) -- every
    /// command and the graceful-shutdown path check this first so they don't act as if a dead
    /// runtime were still there.
    runtime_alive: Mutex<bool>,
    /// Notified once the runtime's event stream closes during shutdown; `graceful_shutdown`
    /// below waits on this (bounded by a timeout) before exiting the process.
    shutdown_complete: Arc<Notify>,
    /// Where the live config gets persisted -- Tauri's `app_config_dir()` joined with
    /// `bridge-config.json`, resolved once at startup.
    config_path: std::path::PathBuf,
    /// Where named preset snapshots are stored -- `<app_config_dir>/presets/`, resolved once
    /// at startup alongside `config_path`.
    presets_dir: std::path::PathBuf,
}

/// `Err` once the runtime task has exited, so commands don't silently mutate the mirror or
/// send into a channel with nothing left listening on the other end.
fn require_runtime_alive(state: &AppState) -> Result<(), String> {
    if *state.runtime_alive.lock().unwrap() {
        Ok(())
    } else {
        Err("bridge runtime is no longer running".to_string())
    }
}

#[tauri::command]
async fn lifecycle_start(
    state: tauri::State<'_, AppState>,
    patch: Option<BridgeConfigPatch>,
) -> Result<(), String> {
    require_runtime_alive(&state)?;
    // Locked for the whole function, including the send below: two overlapping `invoke`s
    // (each its own concurrent Tauri task) could otherwise interleave their mutate-then-send
    // steps out of order and leave the mirror holding a value the runtime never actually got.
    // This mutex now doubles as a command-submission ordering lock across all three mutating
    // commands (lifecycle_start/lifecycle_stop/config_apply), not just config-mutation
    // protection -- lifecycle_stop locks it too even though it never touches `config`'s value.
    let mut config = state.config.lock().unwrap();
    if let Some(p) = &patch {
        // save_config_file below also runs under this same lock, deliberately: it's a small,
        // local, synchronous file write, so briefly blocking other commands on it is an
        // accepted tradeoff against the complexity of a separate persistence lock.
        let result = bridge_config::apply_patch(&mut config, p);
        if result.anything_changed {
            if let Err(e) = bridge_config::save_config_file(&state.config_path, &config) {
                eprintln!("[config] Failed to persist config: {e}");
            }
        }
    }
    // Matches runtime.rs's enter_running_state, which unconditionally transitions to Running
    // regardless of prior state -- set optimistically here (not just from the round-tripped
    // LifecycleChanged event) so config_apply's FIFO-ordering assumption holds.
    *state.lifecycle.lock().unwrap() = Lifecycle::Running;
    state
        .command_tx
        .send(BridgeCommand::Start(patch))
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn lifecycle_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    require_runtime_alive(&state)?;
    // lifecycle_stop doesn't touch `config`'s contents, but it still locks it for the whole
    // function (including the send below) to participate in the same command-submission
    // ordering lock lifecycle_start/config_apply use -- otherwise this could still interleave
    // with a concurrent config_apply and leave the mirror holding a patch the runtime actually
    // dropped (because Stop reached it first).
    let _config = state.config.lock().unwrap();
    *state.lifecycle.lock().unwrap() = Lifecycle::Standby;
    state
        .command_tx
        .send(BridgeCommand::Stop)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn config_apply(
    state: tauri::State<'_, AppState>,
    patch: BridgeConfigPatch,
) -> Result<(), String> {
    require_runtime_alive(&state)?;
    // The runtime itself ignores ConfigApply while standby (a standby-leak guard), so only
    // mutate the local mirror when the runtime is actually going to adopt the patch -- otherwise
    // get_config would report a value the runtime never applied. lifecycle_start/lifecycle_stop
    // update `lifecycle` optimistically at send time, and the runtime processes commands
    // strictly FIFO, so this read is accurate as of the moment this command was issued.
    // Locked for the whole function (including the send below) for the same interleaving
    // reason as lifecycle_start. save_config_file below also runs under this same lock, for
    // the same reason it does in lifecycle_start -- a brief local file write is fine to hold
    // it across.
    let mut config = state.config.lock().unwrap();
    let runtime_will_be_running = *state.lifecycle.lock().unwrap() == Lifecycle::Running;
    if runtime_will_be_running {
        let result = bridge_config::apply_patch(&mut config, &patch);
        if result.anything_changed {
            if let Err(e) = bridge_config::save_config_file(&state.config_path, &config) {
                eprintln!("[config] Failed to persist config: {e}");
            }
        }
    }
    state
        .command_tx
        .send(BridgeCommand::ConfigApply(patch))
        .map_err(|e| e.to_string())
}

/// Persists standby-state config edits so quitting the app doesn't silently discard them --
/// covers edits made before ever hitting Start this session, edits made after an explicit
/// Stop, and edits made after the runtime dies unexpectedly (`lifecycle_start`/`config_apply`
/// already persist for the starting/running cases, see their own doc comments). A deliberate
/// no-op while actually running: `config_apply` already owns persistence for that case, and
/// applying a possibly-stale debounced draft on top of a live running config here could race
/// with (and clobber) a concurrent `config_apply`'s result -- this is why `lifecycle` is read
/// AFTER `config` is locked, inside the same critical section, not before: reading it earlier
/// would let a concurrent `lifecycle_start` finish (and flip to Running) in the gap, and this
/// command would then persist a now-stale draft over what Start already correctly wrote.
/// Doesn't call `require_runtime_alive` -- like `get_config`/`bridge_status`, this never touches
/// `command_tx`, so a dead runtime doesn't change what this command can safely do (it's still
/// just a local mutation + file write). This also means it participates in the
/// command-submission ordering lock only as a BLOCKER (it takes the same `config` mutex the
/// other three commands do, so it can't splice into their critical sections), never as a
/// SUBMITTER -- it has nothing to send over `command_tx`, so it can't itself reorder anything
/// on the runtime's FIFO queue.
#[tauri::command]
async fn save_draft_config(
    state: tauri::State<'_, AppState>,
    patch: BridgeConfigPatch,
) -> Result<(), String> {
    let mut config = state.config.lock().unwrap();
    let is_standby = *state.lifecycle.lock().unwrap() == Lifecycle::Standby;
    if !is_standby {
        return Ok(());
    }
    let result = bridge_config::apply_patch(&mut config, &patch);
    if result.anything_changed {
        bridge_config::save_config_file(&state.config_path, &config).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// get_config/bridge_status deliberately skip require_runtime_alive: they're pure reads with
// nothing to mutate or protect, and gating them would just turn "the mirror lies forever" into
// "the command silently errors with no UI feedback" (the smoke page's refreshStatus() has no
// .catch()). The mirror is accurate enough to return unconditionally -- the terminal block in
// setup() already forces `lifecycle` to Standby once the runtime is gone, and `config` stays
// whatever it last legitimately was.
#[tauri::command]
async fn get_config(state: tauri::State<'_, AppState>) -> Result<RuntimeConfig, String> {
    Ok(state.config.lock().unwrap().clone())
}

#[tauri::command]
async fn bridge_status(state: tauri::State<'_, AppState>) -> Result<Lifecycle, String> {
    Ok(*state.lifecycle.lock().unwrap())
}

/// Pull-based counterpart to the `status://update` push event -- lets the frontend fetch the
/// current status on mount instead of only relying on the NEXT change event, which could be
/// an arbitrarily long time away (or never, for a stable session) if the frontend's listener
/// attaches after the poll thread's first tick already fired and nothing has changed since.
#[tauri::command]
async fn get_status(
    state: tauri::State<'_, std::sync::Arc<std::sync::Mutex<Option<StatusSnapshot>>>>,
) -> Result<Option<StatusSnapshot>, String> {
    Ok(*state.lock().unwrap())
}

#[tauri::command]
async fn list_presets(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<presets::PresetSummary>, String> {
    presets::list_presets_in_dir(&state.presets_dir).map_err(|e| e.to_string())
}

#[tauri::command]
async fn save_preset(
    state: tauri::State<'_, AppState>,
    name: String,
    config: RuntimeConfig,
) -> Result<presets::PresetSummary, String> {
    presets::save_preset_to_dir(&state.presets_dir, &name, &config).map_err(|e| e.to_string())
}

#[tauri::command]
async fn load_preset(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<presets::PresetPayload, String> {
    presets::load_preset_from_dir(&state.presets_dir, &id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_preset(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    presets::delete_preset_from_dir(&state.presets_dir, &id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn check_preset_collision(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<Option<String>, String> {
    Ok(presets::preset_collision_in_dir(&state.presets_dir, &name))
}

#[tauri::command]
async fn export_preset(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<bool, String> {
    let payload =
        presets::load_preset_from_dir(&state.presets_dir, &id).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;

    let default_name = format!("{}.json", payload.name);
    let chosen = app_handle
        .dialog()
        .file()
        .add_filter("JSON", &["json"])
        .set_file_name(&default_name)
        .set_title("Export Preset")
        .blocking_save_file();

    let Some(file_path) = chosen else {
        return Ok(false); // user cancelled -- not an error
    };
    let path = file_path.into_path().map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
async fn import_preset(app_handle: tauri::AppHandle) -> Result<Option<presets::PresetPayload>, String> {
    let chosen = app_handle
        .dialog()
        .file()
        .add_filter("JSON", &["json"])
        .set_title("Import Preset")
        .blocking_pick_file();

    let Some(file_path) = chosen else {
        return Ok(None); // user cancelled -- not an error
    };
    let path = file_path.into_path().map_err(|e| e.to_string())?;
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let payload: presets::PresetPayload =
        serde_json::from_str(&raw).map_err(|e| format!("Invalid preset file: {e}"))?;
    Ok(Some(payload))
}

#[tauri::command]
async fn open_presets_folder(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    std::fs::create_dir_all(&state.presets_dir).map_err(|e| e.to_string())?;
    app_handle
        .opener()
        .open_path(state.presets_dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn open_kofi_page(app_handle: tauri::AppHandle) -> Result<(), String> {
    app_handle
        .opener()
        .open_url("https://ko-fi.com/K3N223V22V", None::<&str>)
        .map_err(|e| e.to_string())
}

/// Result of an update check, mirrored to the frontend as `UpdateCheckResult`. `error: true`
/// means the check itself failed (network error, bad API response) -- distinct from
/// `available: false` with `error: false`, which means the check succeeded and the app is
/// already up to date.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct UpdateCheckResult {
    available: bool,
    latest_version: Option<String>,
    download_url: Option<String>,
    release_url: Option<String>,
    error: bool,
}

/// Public repo whose GitHub Releases this GUI checks against.
const UPDATE_CHECK_REPO: &str = "strejda603/softube-ms-bridge-public";

#[tauri::command]
async fn check_for_update() -> UpdateCheckResult {
    let release = match update_checker::fetch_latest_release(UPDATE_CHECK_REPO).await {
        Ok(release) => release,
        Err(e) => {
            eprintln!("[update] check failed: {e}");
            return UpdateCheckResult {
                error: true,
                ..Default::default()
            };
        }
    };

    if !update_checker::is_newer_version(env!("CARGO_PKG_VERSION"), &release.tag_name) {
        return UpdateCheckResult::default();
    }

    let asset = update_checker::pick_release_asset(&release.assets, std::env::consts::OS);
    UpdateCheckResult {
        available: true,
        latest_version: Some(release.tag_name.trim_start_matches(['v', 'V']).to_string()),
        download_url: Some(
            asset
                .map(|a| a.browser_download_url.clone())
                .unwrap_or_else(|| release.html_url.clone()),
        ),
        release_url: Some(release.html_url),
        error: false,
    }
}

#[tauri::command]
async fn open_download(app_handle: tauri::AppHandle, url: String) -> Result<(), String> {
    let Ok(parsed) = url::Url::parse(&url) else {
        return Ok(()); // Malformed URL -- silently ignore, matching the original's behavior.
    };
    if parsed.scheme() == "https" && parsed.host_str() == Some("github.com") {
        app_handle
            .opener()
            .open_url(&url, None::<&str>)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Turns a panicking task's `JoinError` into a human-readable message, handling both the
/// `&str` shape (`panic!("literal")`) and `String` shape (`panic!("{}", formatted)`) that
/// `std::panic::catch_unwind` payloads commonly take.
fn panic_reason(join_error: tokio::task::JoinError) -> String {
    let panic_payload = join_error.into_panic();
    panic_payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic_payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_string())
}

/// Returns the `--lang <code>` value from this launch's CLI args, if any -- a session-only
/// override the frontend applies once on mount via `i18n.svelte.ts`'s `setLocale(code, false)`
/// (see its doc comment), leaving the user's saved locale preference untouched for next time.
#[tauri::command]
fn get_launch_lang(cli_args: tauri::State<cli_args::CliArgs>) -> Option<String> {
    cli_args.lang.clone()
}

/// Returns this launch's parsed `--start`/`--stop`/`--preset`/`--ws`/`--interval`/`--log`
/// values (everything `get_launch_lang` doesn't already cover) -- applied once by the frontend
/// on mount, same call-once-at-launch contract as `get_launch_lang`.
#[tauri::command]
fn get_launch_args(cli_args: tauri::State<cli_args::CliArgs>) -> cli_args::CliArgs {
    cli_args.inner().clone()
}

/// Sends `BridgeCommand::Shutdown` and waits (bounded) for the runtime to confirm it's done --
/// shared by the window-close handler and the Ctrl+C/SIGINT handler below, since both need the
/// identical sequence, just triggered by a different event. Callers are responsible for
/// destroying any window and calling `app_handle.exit(0)` afterward -- this function only runs
/// the runtime-shutdown half, not process teardown, since the Ctrl+C caller has no window to
/// destroy.
async fn graceful_shutdown(app_handle: &tauri::AppHandle) {
    // AppState is managed asynchronously in setup(), not synchronously before the window/signal
    // handler exists -- a shutdown trigger landing in that narrow startup window has no runtime
    // to shut down yet anyway.
    let Some(state) = app_handle.try_state::<AppState>() else {
        return;
    };
    // The runtime may have already died on its own (crash or silent self-exit) before this
    // trigger arrived -- notify_waiters() already fired once in that case and stores no permit
    // for a later subscriber, so waiting again here would just burn the full timeout on a
    // notification that's never coming.
    if !*state.runtime_alive.lock().unwrap() {
        return;
    }
    // Pre-register the waiter before sending Shutdown: notify_waiters() stores no permit for
    // later subscribers, and Notified only registers on its first poll (not construction), so
    // sending first risks a lost wakeup if the runtime finishes before this future is ever
    // polled.
    let notified = state.shutdown_complete.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    // Narrower version of the same lost-wakeup class as the check above: the runtime could
    // still die and fire notify_waiters() in the gap between the first check and this enable()
    // call. Re-check now that the waiter is actually registered.
    if !*state.runtime_alive.lock().unwrap() {
        return;
    }
    let _ = state.command_tx.send(BridgeCommand::Shutdown);
    // Bounded: a wedged shutdown must not hang the process forever. Matches bridge-cli's own 5s
    // drain timeout in spirit (shorter here since a GUI close/Ctrl+C should feel responsive; the
    // runtime's own internal WS_SHUTDOWN_GRACE is 500ms, well under this).
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), notified).await;
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Skip argv[0] (the executable path), matching Electron's own `getUserArgv` convention.
    // `args_os()` + lossy conversion (not `args()`) so a non-UTF-8 argv entry (e.g. an OS-supplied
    // file path on macOS, which allows arbitrary byte sequences) degrades to lossy text instead of
    // panicking before the window ever appears.
    let cli_args = cli_args::parse_cli_args(
        &std::env::args_os()
            .skip(1)
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
    );

    tauri::Builder::default()
        // Must be the first plugin registered: it needs to claim the single-instance lock
        // before any window is created. On a second launch it runs in the *first* instance's
        // process (the second process exits immediately), so this closure just re-focuses the
        // existing window -- there's no use case for multiple windows against one Console 1
        // Fader/Mixing Station pair.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .manage(cli_args)
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // A directory-creation failure here (read-only home, permissions, something
            // squatting the path) shouldn't take down the whole app over a persistence nicety --
            // fall back to a config_path in the (uncreated) directory: load_config_file below
            // will just fail to find anything there and return None, and later save attempts
            // will fail too, both already handled as no-ops/logged errors rather than panics.
            let config_dir = app.path().app_config_dir()?;
            let config_path = std::fs::create_dir_all(&config_dir)
                .map(|_| bridge_config::config_file_path(&config_dir))
                .unwrap_or_else(|e| {
                    eprintln!("[config] Could not create config directory, persistence disabled this session: {e}");
                    bridge_config::config_file_path(&config_dir)
                });
            let presets_dir = config_dir.join("presets");

            let mut initial_config = RuntimeConfig::default();
            if let Some(patch) = bridge_config::load_config_file(&config_path) {
                bridge_config::apply_patch(&mut initial_config, &patch);
            }

            let status_state: std::sync::Arc<std::sync::Mutex<Option<StatusSnapshot>>> =
                std::sync::Arc::new(std::sync::Mutex::new(None));
            app.manage(status_state.clone());

            let app_handle = app.handle().clone();

            // spawn_bridge_runtime calls plain tokio::spawn internally, which requires an
            // entered tokio runtime context -- setup() itself doesn't have one, but a task
            // spawned via tauri::async_runtime::spawn does (verified in this plan's scratch
            // crate). manage() runs from inside this same task for the same reason.
            tauri::async_runtime::spawn(async move {
                let handle = spawn_bridge_runtime(initial_config.clone());
                app_handle.manage(AppState {
                    command_tx: handle.command_tx,
                    config: Mutex::new(initial_config),
                    lifecycle: Mutex::new(Lifecycle::Standby),
                    runtime_alive: Mutex::new(true),
                    shutdown_complete: Arc::new(Notify::new()),
                    config_path,
                    presets_dir,
                });
                // Signals the frontend that commands are safe to call -- manage() above and
                // this emit happen with no `.await` between them, so in practice no command
                // could race ahead of it, but a future real UI should still gate on this rather
                // than assume instant availability.
                let _ = app_handle.emit("bridge://ready", ());

                // Panic isolation: bridge_core::runtime::BridgeEvent::Crashed exists
                // specifically for this (see its doc comment), but nothing emits it yet. The
                // events-forwarder loop below can't tell "the task panicked" apart from "the
                // task shut down cleanly" -- both close `events` the same way. Watching the
                // task's own join handle can: a panic resolves it to Err(JoinError) with
                // is_panic() true, which a clean exit never produces. Note this only catches a
                // panic that propagates out of the runtime task's own top-level body (its main
                // select-loop / dispatch code) -- panics inside the MIDI thread or WS engine are
                // already isolated separately (swallowed at their own join points) and never
                // reach this join handle.
                let watcher_app_handle = app_handle.clone();
                let join_handle = handle.join_handle;
                tauri::async_runtime::spawn(async move {
                    if let Err(join_error) = join_handle.await {
                        if join_error.is_panic() {
                            let reason = panic_reason(join_error);
                            let _ = watcher_app_handle
                                .emit("bridge://event", &BridgeEvent::Crashed(reason));
                        }
                    }
                });

                let mut events = handle.events;
                while let Some(event) = events.recv().await {
                    let _ = app_handle.emit("bridge://event", &event);
                }
                // Runtime task exited (its events sender was dropped) -- shutdown is complete.
                // This runs for every way the runtime task can end: graceful shutdown, a caught
                // panic (Task 6's watcher above), or a silent self-exit (e.g. the MIDI thread
                // dying without panicking, which the runtime's select loop treats as a clean
                // `break`, emitting no Crashed event at all). So this is the one place that can
                // reliably mark the runtime as gone -- Lifecycle has no "gone" variant of its
                // own, so Standby is the most honest approximation available.
                let state = app_handle.state::<AppState>();
                *state.runtime_alive.lock().unwrap() = false;
                *state.lifecycle.lock().unwrap() = Lifecycle::Standby;
                // Without this, a silent self-exit (the runtime's own select loop `break`ing
                // without ever calling enter_standby_state, e.g. the MIDI thread dying) leaves
                // the frontend's mirrored `lifecycle` state stuck on "running" forever -- Start
                // stays disabled and Stop just rejects every click with "bridge runtime is no
                // longer running", with no visible recovery short of relaunching the app. Harmless
                // on the graceful window-close path too: the app is exiting, nobody's listening.
                let _ = app_handle.emit("bridge://event", &BridgeEvent::LifecycleChanged(Lifecycle::Standby));
                state.shutdown_complete.notify_waiters();
            });

            // Status-monitor: polls every 2s on its own dedicated thread (not a tokio task --
            // see status_gather's module doc for why), independent of the bridge runtime's own
            // lifecycle. A real Console 1/iPad/Mixing Station status shouldn't blank out just
            // because the user hasn't hit Start yet. Mirrors app/statusMonitor.js running in
            // Electron's main process regardless of the bridge child process's state.
            let status_app_handle = app.handle().clone();
            let status_state_for_thread = status_state.clone();
            std::thread::spawn(move || {
                status_gather::run_status_poll_loop(move |snapshot| {
                    *status_state_for_thread.lock().unwrap() = Some(snapshot);
                    let _ = status_app_handle.emit("status://update", &snapshot);
                });
            });

            // Ctrl+C/SIGINT graceful shutdown -- without this, a developer running
            // `npm run tauri dev` and hitting Ctrl+C kills the process with zero Rust code
            // running: no signal handler is otherwise registered anywhere in this binary, so
            // the OS default disposition (immediate termination) applies. This leaves any real
            // connected Console 1 hardware showing stale track/OSD data until the next graceful
            // start. The packaged/production app is unaffected in practice -- a real user closes
            // via the window (WindowEvent::CloseRequested, already handled above), not by
            // sending a terminal signal -- but installing this handler unconditionally is
            // harmless and also covers the same class of process-level termination (e.g. `kill
            // -INT <pid>`) in a packaged build. Also note: registering this handler replaces the
            // OS default disposition for SIGINT, so a second Ctrl+C after the first is captured
            // and ignored rather than force-killing the process (see tokio::signal::ctrl_c's own
            // docs) -- functionally fine here since graceful_shutdown's 3s timeout below already
            // bounds the worst case either way.
            let ctrl_c_app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    graceful_shutdown(&ctrl_c_app_handle).await;
                    ctrl_c_app_handle.exit(0);
                }
            });

            // SIGTERM graceful shutdown -- same rationale as the Ctrl+C handler above, covering
            // a plain `kill <pid>` (no `-INT`). Unix-only: Windows has no equivalent signal a
            // process can trap, so `kill`-style termination there ends the process immediately
            // regardless, same as before this handler existed.
            #[cfg(unix)]
            {
                let sigterm_app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tokio::signal::unix::{signal, SignalKind};
                    if let Ok(mut sig) = signal(SignalKind::terminate()) {
                        sig.recv().await;
                        graceful_shutdown(&sigterm_app_handle).await;
                        sigterm_app_handle.exit(0);
                    }
                });
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let window = window.clone();
                let app_handle = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    graceful_shutdown(&app_handle).await;
                    window.destroy().ok();
                    app_handle.exit(0);
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_launch_lang,
            get_launch_args,
            lifecycle_start,
            lifecycle_stop,
            config_apply,
            save_draft_config,
            get_config,
            bridge_status,
            get_status,
            list_presets,
            save_preset,
            load_preset,
            delete_preset,
            check_preset_collision,
            export_preset,
            import_preset,
            open_presets_folder,
            open_kofi_page,
            check_for_update,
            open_download,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::panic_reason;

    #[tokio::test]
    async fn panic_reason_extracts_a_str_literal_payload() {
        let join_error = tokio::spawn(async { panic!("literal") }).await.unwrap_err();
        assert_eq!(panic_reason(join_error), "literal");
    }

    #[tokio::test]
    async fn panic_reason_extracts_a_string_payload() {
        let join_error = tokio::spawn(async { panic!("{}", "formatted".to_string()) })
            .await
            .unwrap_err();
        assert_eq!(panic_reason(join_error), "formatted");
    }

    #[tokio::test]
    async fn panic_reason_falls_back_for_other_payload_types() {
        let join_error = tokio::spawn(async { std::panic::panic_any(42u32) })
            .await
            .unwrap_err();
        assert_eq!(panic_reason(join_error), "unknown panic");
    }
}
