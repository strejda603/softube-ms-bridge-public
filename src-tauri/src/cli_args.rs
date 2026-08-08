//! Pure CLI argument parsing for the Tauri GUI binary. Mirrors the shape of the Electron app's
//! `app/cliArgs.js` (a plain, dependency-free parser). Every flag Electron's launcher accepted
//! (`--start`/`--stop`/`--preset`/`--ws`/`--interval`/`--log`, plus this app's own `--lang`) is
//! parsed once at launch and applied once by the frontend on mount -- see `get_launch_lang`/
//! `get_launch_args` in `lib.rs`. Unlike Electron, this app has no single-instance forwarding
//! (no `tauri-plugin-single-instance` wired in), so a second launch opens a second window rather
//! than forwarding its args to the first -- these flags only ever affect a fresh launch.

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliArgs {
    pub lang: Option<String>,
    pub start: bool,
    pub stop: bool,
    pub preset: Option<String>,
    pub ws: Option<String>,
    pub interval: Option<i64>,
    pub log: bool,
}

/// Parses already-stripped user args (i.e. `std::env::args().skip(1)`, matching Electron's own
/// `getUserArgv` convention of excluding the executable path). Unrecognized args, a bare
/// trailing value-flag (no following value), and a value-flag followed by an empty or
/// `--`-prefixed token (mirroring `app/cliArgs.js`'s `takeValue` guard, so e.g.
/// `--preset --preset foo` doesn't silently swallow the real value as the flag's own argument)
/// are all silently ignored -- no `warnings` output, unlike Electron's `parseCliArgs`: nothing
/// consumes warnings today, and this binary had zero argv parsing before the `--lang` feature,
/// so there's no existing consumer contract to preserve.
pub fn parse_cli_args(args: &[String]) -> CliArgs {
    let mut result = CliArgs::default();
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--lang" => {
                if let Some(value) = iter.next_if(|v| !v.is_empty() && !v.starts_with("--")) {
                    result.lang = Some(value.clone());
                }
            }
            "--start" => result.start = true,
            "--stop" => result.stop = true,
            "--log" => result.log = true,
            "--preset" => {
                if let Some(value) = iter.next_if(|v| !v.is_empty() && !v.starts_with("--")) {
                    result.preset = Some(value.clone());
                }
            }
            "--ws" => {
                if let Some(value) = iter.next_if(|v| !v.is_empty() && !v.starts_with("--")) {
                    result.ws = Some(value.clone());
                }
            }
            "--interval" => {
                if let Some(value) = iter.next_if(|v| !v.is_empty() && !v.starts_with("--")) {
                    if let Ok(n) = value.parse::<f64>() {
                        if n.is_finite() {
                            result.interval = Some(n.trunc() as i64);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Matches Electron's parseCliArgs: --start and --stop given together is a contradiction,
    // so ignore both rather than guess which one wins.
    if result.start && result.stop {
        result.start = false;
        result.stop = false;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_args_yields_defaults() {
        assert_eq!(parse_cli_args(&args(&[])), CliArgs::default());
    }

    #[test]
    fn lang_flag_with_value_is_parsed() {
        assert_eq!(
            parse_cli_args(&args(&["--lang", "cs"])),
            CliArgs { lang: Some("cs".to_string()), ..CliArgs::default() }
        );
    }

    #[test]
    fn trailing_lang_flag_with_no_value_is_ignored() {
        assert_eq!(parse_cli_args(&args(&["--lang"])), CliArgs::default());
    }

    #[test]
    fn unrecognized_args_are_ignored_without_panicking() {
        assert_eq!(
            parse_cli_args(&args(&["--bogus", "--lang", "cs", "--verbose"])),
            CliArgs { lang: Some("cs".to_string()), ..CliArgs::default() }
        );
    }

    #[test]
    fn last_lang_flag_wins_when_given_twice() {
        assert_eq!(
            parse_cli_args(&args(&["--lang", "en", "--lang", "cs"])),
            CliArgs { lang: Some("cs".to_string()), ..CliArgs::default() }
        );
    }

    #[test]
    fn lang_flag_followed_by_another_flag_is_ignored() {
        assert_eq!(parse_cli_args(&args(&["--lang", "--verbose"])), CliArgs::default());
    }

    #[test]
    fn start_flag_is_parsed() {
        assert_eq!(
            parse_cli_args(&args(&["--start"])),
            CliArgs { start: true, ..CliArgs::default() }
        );
    }

    #[test]
    fn stop_flag_is_parsed() {
        assert_eq!(
            parse_cli_args(&args(&["--stop"])),
            CliArgs { stop: true, ..CliArgs::default() }
        );
    }

    #[test]
    fn start_and_stop_together_cancel_both_out() {
        assert_eq!(
            parse_cli_args(&args(&["--start", "--stop"])),
            CliArgs::default()
        );
    }

    #[test]
    fn log_flag_is_parsed() {
        assert_eq!(parse_cli_args(&args(&["--log"])), CliArgs { log: true, ..CliArgs::default() });
    }

    #[test]
    fn preset_flag_with_value_is_parsed() {
        assert_eq!(
            parse_cli_args(&args(&["--preset", "My Preset"])),
            CliArgs { preset: Some("My Preset".to_string()), ..CliArgs::default() }
        );
    }

    #[test]
    fn preset_flag_with_no_value_is_ignored() {
        assert_eq!(parse_cli_args(&args(&["--preset"])), CliArgs::default());
    }

    #[test]
    fn preset_flag_followed_by_another_flag_is_ignored() {
        assert_eq!(
            parse_cli_args(&args(&["--preset", "--log"])),
            CliArgs { log: true, ..CliArgs::default() }
        );
    }

    #[test]
    fn ws_flag_with_value_is_parsed() {
        assert_eq!(
            parse_cli_args(&args(&["--ws", "ws://example:9000"])),
            CliArgs { ws: Some("ws://example:9000".to_string()), ..CliArgs::default() }
        );
    }

    #[test]
    fn ws_flag_with_no_value_is_ignored() {
        assert_eq!(parse_cli_args(&args(&["--ws"])), CliArgs::default());
    }

    #[test]
    fn interval_flag_with_integer_value_is_parsed() {
        assert_eq!(
            parse_cli_args(&args(&["--interval", "250"])),
            CliArgs { interval: Some(250), ..CliArgs::default() }
        );
    }

    #[test]
    fn interval_flag_truncates_a_fractional_value() {
        assert_eq!(
            parse_cli_args(&args(&["--interval", "250.9"])),
            CliArgs { interval: Some(250), ..CliArgs::default() }
        );
    }

    #[test]
    fn interval_flag_with_non_numeric_value_is_ignored() {
        assert_eq!(parse_cli_args(&args(&["--interval", "fast"])), CliArgs::default());
    }

    #[test]
    fn interval_flag_with_no_value_is_ignored() {
        assert_eq!(parse_cli_args(&args(&["--interval"])), CliArgs::default());
    }

    #[test]
    fn all_flags_together_are_parsed() {
        assert_eq!(
            parse_cli_args(&args(&[
                "--start", "--preset", "Live Set", "--ws", "ws://localhost:9001", "--interval",
                "50", "--log", "--lang", "cs",
            ])),
            CliArgs {
                lang: Some("cs".to_string()),
                start: true,
                stop: false,
                preset: Some("Live Set".to_string()),
                ws: Some("ws://localhost:9001".to_string()),
                interval: Some(50),
                log: true,
            }
        );
    }
}
