//! Headless Softube MS Bridge -- thin binary. All orchestration lives in
//! `bridge_core::runtime` (Plan 3); this binary just spawns the runtime, prints its
//! events to stdout, and forwards Ctrl+C/SIGTERM as a clean shutdown.

use bridge_config::{BridgeConfigPatch, RuntimeConfig};
use bridge_core::runtime::{spawn_bridge_runtime, BridgeCommand, BridgeEvent};

/// Waits for `SIGTERM` (e.g. a plain `kill <pid>`, no `-INT`). Unix-only -- Windows has no
/// equivalent signal a process can trap: a `kill`-style termination there ends the process
/// immediately regardless, so this future never resolves on non-unix targets and the
/// `tokio::select!` arm awaiting it simply never wins.
#[cfg(unix)]
async fn wait_for_sigterm() {
    use tokio::signal::unix::{signal, SignalKind};
    match signal(SignalKind::terminate()) {
        Ok(mut sig) => {
            sig.recv().await;
        }
        Err(_) => std::future::pending::<()>().await,
    }
}

#[cfg(not(unix))]
async fn wait_for_sigterm() {
    std::future::pending::<()>().await
}

fn print_event(event: &BridgeEvent) {
    // Deliberately exhaustive -- a new BridgeEvent variant must fail to compile here,
    // not silently no-op.
    match event {
        BridgeEvent::Log(line) => println!("{line}"),
        BridgeEvent::LifecycleChanged(lifecycle) => println!("[Lifecycle] state: {lifecycle:?}"),
        BridgeEvent::ConfigApplied {
            url_changed,
            anything_changed,
        } => {
            println!(
                "[config] applied (url_changed={url_changed}, anything_changed={anything_changed})"
            );
        }
        BridgeEvent::Crashed(reason) => eprintln!("[Crashed] {reason}"),
    }
}

/// Resolves the config file path: `BRIDGE_CONFIG_PATH` env var if set to a non-blank value,
/// else `bridge-config.json` relative to the current working directory. Matches JS's
/// `loadBridgeConfig` (`index.js:271-273`).
fn resolve_config_path() -> std::path::PathBuf {
    match std::env::var("BRIDGE_CONFIG_PATH") {
        Ok(v) if !v.trim().is_empty() => std::path::PathBuf::from(v),
        _ => std::path::PathBuf::from("bridge-config.json"),
    }
}

/// Builds the startup `RuntimeConfig` from a config-file patch (if any) and the raw
/// `MIXING_STATION_WS_URL`/`LOG_JSON` env var strings (if set) -- deliberately takes these as
/// plain parameters rather than reading `std::env`/`std::fs` itself, so it's a pure function
/// safe to unit-test without env-var/filesystem side effects. Matches JS's `loadBridgeConfig`
/// (`index.js:267-326`) precedence: env overrides file overrides hardcoded defaults.
fn build_startup_config(
    file_patch: Option<BridgeConfigPatch>,
    ws_url_env: Option<String>,
    log_json_env: Option<String>,
) -> RuntimeConfig {
    let mut config = RuntimeConfig::default();
    if let Some(patch) = file_patch {
        bridge_config::apply_patch(&mut config, &patch);
    }
    if let Some(url) = ws_url_env {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            config.mixing_station_ws_url = trimmed.to_string();
        }
    }
    if let Some(raw) = log_json_env {
        if let Some(value) = bridge_config::parse_bool_env(&raw) {
            config.log_json = value;
        }
    }
    config
}

#[tokio::main]
async fn main() {
    println!("Softube MS Bridge (Rust) starting...");

    let config_path = resolve_config_path();
    let file_patch = bridge_config::load_config_file(&config_path);
    let config = build_startup_config(
        file_patch,
        std::env::var("MIXING_STATION_WS_URL").ok(),
        std::env::var("LOG_JSON").ok(),
    );

    let mut handle = spawn_bridge_runtime(config);
    println!("Softube-MS-Bridge running (standby). Press CTRL+C to exit.");

    tokio::pin! {
        let ctrl_c = tokio::signal::ctrl_c();
        let sigterm = wait_for_sigterm();
    }

    loop {
        tokio::select! {
            _ = &mut ctrl_c => break,
            _ = &mut sigterm => break,
            event = handle.events.recv() => match event {
                Some(event) => {
                    print_event(&event);
                    if matches!(event, BridgeEvent::Crashed(_)) {
                        break;
                    }
                }
                None => break, // runtime task exited
            },
        }
    }

    println!("\nShutting down Softube-MS-Bridge...");
    let _ = handle.command_tx.send(BridgeCommand::Shutdown);

    // The runtime reports its shutdown steps (RESET, deactivate, OSD off, WS close) as
    // ordinary events, so they only reach stdout if we keep draining until the task drops
    // its sender. Bounded, so a wedged shutdown can't hold the process open forever.
    let drain = async {
        while let Some(event) = handle.events.recv().await {
            print_event(&event);
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), drain).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle.join_handle).await;
    println!("Shutdown complete.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_config::BridgeConfigPatch;

    #[test]
    fn no_file_patch_and_no_env_overrides_yields_plain_defaults() {
        let config = build_startup_config(None, None, None);
        let defaults = RuntimeConfig::default();
        assert_eq!(config.mixing_station_ws_url, defaults.mixing_station_ws_url);
        assert_eq!(config.log_json, defaults.log_json);
    }

    #[test]
    fn file_patch_is_applied_when_present() {
        let patch = BridgeConfigPatch {
            mixing_station_ws_url: Some("ws://from-file:9999".to_string()),
            ..Default::default()
        };
        let config = build_startup_config(Some(patch), None, None);
        assert_eq!(config.mixing_station_ws_url, "ws://from-file:9999");
    }

    #[test]
    fn ws_url_env_override_wins_over_file() {
        let patch = BridgeConfigPatch {
            mixing_station_ws_url: Some("ws://from-file:9999".to_string()),
            ..Default::default()
        };
        let config = build_startup_config(
            Some(patch),
            Some("ws://from-env:1234".to_string()),
            None,
        );
        assert_eq!(config.mixing_station_ws_url, "ws://from-env:1234");
    }

    #[test]
    fn blank_ws_url_env_override_is_ignored() {
        let config = build_startup_config(None, Some("   ".to_string()), None);
        assert_eq!(
            config.mixing_station_ws_url,
            RuntimeConfig::default().mixing_station_ws_url
        );
    }

    #[test]
    fn log_json_env_override_true_wins_over_file() {
        let patch = BridgeConfigPatch {
            log_json: Some(false),
            ..Default::default()
        };
        let config = build_startup_config(Some(patch), None, Some("1".to_string()));
        assert!(config.log_json);
    }

    #[test]
    fn log_json_env_override_false_wins_over_file() {
        let patch = BridgeConfigPatch {
            log_json: Some(true),
            ..Default::default()
        };
        let config = build_startup_config(Some(patch), None, Some("false".to_string()));
        assert!(!config.log_json);
    }

    #[test]
    fn invalid_log_json_env_override_is_ignored() {
        let patch = BridgeConfigPatch {
            log_json: Some(true),
            ..Default::default()
        };
        let config = build_startup_config(Some(patch), None, Some("banana".to_string()));
        assert!(config.log_json, "invalid env value should leave the file's value untouched");
    }

    #[test]
    fn env_overrides_win_over_file_which_wins_over_defaults() {
        let patch = BridgeConfigPatch {
            mixing_station_ws_url: Some("ws://from-file:9999".to_string()),
            log_json: Some(true),
            ..Default::default()
        };
        let config = build_startup_config(
            Some(patch),
            Some("ws://from-env:1234".to_string()),
            Some("0".to_string()),
        );
        assert_eq!(config.mixing_station_ws_url, "ws://from-env:1234");
        assert!(!config.log_json);
    }
}
