//! Update-check logic: compares the running app's version against the latest GitHub release
//! of the public repo, and picks the right release asset to offer as a download. Pure/testable
//! logic lives here (ported from the pre-migration Electron app's `app/updateChecker.js`); the
//! actual network call and Tauri command wiring live in `lib.rs`.

use serde::Deserialize;

/// Parses a "major.minor.patch" version string (optionally `v`-prefixed, e.g. a GitHub release
/// tag) into a 3-tuple of integers. Returns `None` for anything that doesn't cleanly parse --
/// callers treat that as "not newer" rather than risking a false update prompt.
fn parse_version(version: &str) -> Option<(u32, u32, u32)> {
    let cleaned = version.trim_start_matches(['v', 'V']);
    let parts: Vec<&str> = cleaned.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let nums: Vec<u32> = parts.iter().filter_map(|p| p.parse::<u32>().ok()).collect();
    if nums.len() != 3 {
        return None;
    }
    Some((nums[0], nums[1], nums[2]))
}

/// True only if `latest` is a strictly greater major.minor.patch than `current`. Tuple ordering
/// in Rust compares lexicographically (major first, then minor, then patch), matching the
/// original's explicit per-segment loop exactly.
pub fn is_newer_version(current: &str, latest: &str) -> bool {
    let (Some(a), Some(b)) = (parse_version(current), parse_version(latest)) else {
        return false;
    };
    b > a
}

/// One release asset from GitHub's API response.
#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    #[serde(rename = "browser_download_url")]
    pub browser_download_url: String,
}

/// The relevant fields of GitHub's `GET /repos/{repo}/releases/latest` response.
#[derive(Debug, Clone, Deserialize)]
pub struct LatestRelease {
    #[serde(rename = "tag_name")]
    pub tag_name: String,
    #[serde(rename = "html_url")]
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

/// Picks the release asset matching this platform, by file extension only -- no arch-matching
/// (Tauri's bundler artifact naming isn't pinned down yet; the original's arch-match already
/// fell back to "any same-platform asset" when arch didn't match, so this preserves the same
/// practical behavior). Only macOS (`.dmg`) and Windows (`.msi`/`.exe`) are recognized, matching
/// this app's current macOS+Windows-only supported-platform scope (see README).
pub fn pick_release_asset<'a>(assets: &'a [ReleaseAsset], platform: &str) -> Option<&'a ReleaseAsset> {
    let extensions: &[&str] = match platform {
        "macos" => &["dmg"],
        "windows" => &["msi", "exe"],
        _ => return None,
    };
    assets.iter().find(|asset| {
        let lower = asset.name.to_lowercase();
        extensions.iter().any(|ext| lower.ends_with(&format!(".{ext}")))
    })
}

/// Fetches the latest published release for a `owner/repo` GitHub repository. GitHub's API
/// rejects requests with no `User-Agent` header, so one is set explicitly here -- this is not
/// optional decoration, omitting it makes every request fail with a 403.
pub async fn fetch_latest_release(repo: &str) -> Result<LatestRelease, String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let client = reqwest::Client::builder()
        .user_agent("softube-ms-bridge-public-edition")
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API returned {} for {}", resp.status(), repo));
    }
    resp.json::<LatestRelease>().await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_parses_plain_semver() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
    }

    #[test]
    fn parse_version_strips_leading_v() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("V1.2.3"), Some((1, 2, 3)));
    }

    #[test]
    fn parse_version_rejects_wrong_segment_count() {
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
    }

    #[test]
    fn parse_version_rejects_non_numeric_segments() {
        assert_eq!(parse_version("1.2.x"), None);
    }

    #[test]
    fn is_newer_version_true_when_major_is_greater() {
        assert!(is_newer_version("1.0.0", "2.0.0"));
    }

    #[test]
    fn is_newer_version_true_when_minor_is_greater_and_major_equal() {
        assert!(is_newer_version("1.1.0", "1.2.0"));
    }

    #[test]
    fn is_newer_version_true_when_patch_is_greater_and_major_minor_equal() {
        assert!(is_newer_version("1.1.1", "1.1.2"));
    }

    #[test]
    fn is_newer_version_false_when_equal() {
        assert!(!is_newer_version("1.2.3", "1.2.3"));
    }

    #[test]
    fn is_newer_version_false_when_latest_is_older() {
        assert!(!is_newer_version("2.0.0", "1.9.9"));
    }

    #[test]
    fn is_newer_version_false_when_either_side_fails_to_parse() {
        assert!(!is_newer_version("not-a-version", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "not-a-version"));
    }

    #[test]
    fn is_newer_version_handles_v_prefixed_latest() {
        assert!(is_newer_version("1.0.0", "v1.0.1"));
    }

    fn asset(name: &str) -> ReleaseAsset {
        ReleaseAsset {
            name: name.to_string(),
            browser_download_url: format!("https://github.com/x/{name}"),
        }
    }

    #[test]
    fn pick_release_asset_finds_macos_dmg() {
        let assets = vec![asset("App_2.0.0_x64.dmg"), asset("App_2.0.0_x64-setup.exe")];
        let picked = pick_release_asset(&assets, "macos").unwrap();
        assert!(picked.name.ends_with(".dmg"));
    }

    #[test]
    fn pick_release_asset_finds_windows_msi_or_exe() {
        let assets = vec![asset("App_2.0.0_x64.dmg"), asset("App_2.0.0_x64_en-US.msi")];
        let picked = pick_release_asset(&assets, "windows").unwrap();
        assert!(picked.name.ends_with(".msi"));
    }

    #[test]
    fn pick_release_asset_matches_case_insensitively() {
        let assets = vec![asset("App_2.0.0_X64.DMG")];
        assert!(pick_release_asset(&assets, "macos").is_some());
    }

    #[test]
    fn pick_release_asset_returns_none_for_unrecognized_platform() {
        let assets = vec![asset("App_2.0.0.dmg")];
        assert!(pick_release_asset(&assets, "linux").is_none());
    }

    #[test]
    fn pick_release_asset_returns_none_when_no_asset_matches() {
        let assets = vec![asset("App_2.0.0.deb")];
        assert!(pick_release_asset(&assets, "macos").is_none());
    }

    #[test]
    fn pick_release_asset_returns_none_for_empty_list() {
        assert!(pick_release_asset(&[], "macos").is_none());
    }
}
