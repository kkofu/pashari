//! Checks GitHub Releases for the latest version (update-check feature).
//!
//! Doesn't actually update (download/replace). Distribution is an
//! installer-less portable exe (see `.github/workflows/release.yml`), and
//! self-updating an unsigned binary isn't worth the risk of botched
//! cleanup on failure or AV false positives, so this just shows a link to
//! the release page when a newer version exists (download/extract/
//! replace stays a manual step for the user, as before).

use serde_json::Value;

/// This build's version (`Cargo.toml`'s `version`).
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const REPO_API: &str = "https://api.github.com/repos/yozba/pashari/releases/latest";

/// Info about a newer release that was found.
#[derive(Clone)]
pub struct ReleaseInfo {
    /// The version string with the tag's leading `v` stripped (e.g. "0.6.0").
    pub version: String,
    /// The release page URL (opened via `shell::open_url`).
    pub url: String,
}

/// Fetches the latest release, returning it if newer than the current version.
pub fn check_latest() -> Result<Option<ReleaseInfo>, String> {
    let value: Value = ureq::get(REPO_API)
        .header("User-Agent", "pashari-update-check")
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_json()
        .map_err(|e| e.to_string())?;

    let tag = value
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or("tag_name が取得できません")?;
    let version = tag.trim_start_matches('v').to_string();
    let url = value
        .get("html_url")
        .and_then(Value::as_str)
        .unwrap_or("https://github.com/yozba/pashari/releases")
        .to_string();

    if is_newer(&version, CURRENT_VERSION) {
        Ok(Some(ReleaseInfo { version, url }))
    } else {
        Ok(None)
    }
}

/// A simple numeric "X.Y.Z" comparison (no pre-release identifier
/// support; sufficient since this project's tags are always `vX.Y.Z`).
/// Missing/invalid components are treated as `0`; never panics.
pub fn is_newer(latest: &str, current: &str) -> bool {
    parse_version(latest) > parse_version(current)
}

fn parse_version(v: &str) -> (u32, u32, u32) {
    let mut parts = v.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_compares_major_minor_patch_numerically() {
        assert!(is_newer("0.6.0", "0.5.0"));
        assert!(is_newer("0.5.10", "0.5.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.5.0", "0.5.0"));
        assert!(!is_newer("0.5.0", "0.6.0"));
    }

    #[test]
    fn is_newer_treats_malformed_or_short_versions_as_zero_without_panicking() {
        assert!(!is_newer("garbage", "0.0.0"));
        assert!(is_newer("1", "0.9.9"));
        assert!(!is_newer("0.5", "0.5.1"));
    }
}
