//! Version probing and comparison for CC CLI and Claude Desktop.
//!
//! No `semver` crate dependency — Anthropic's release versions are
//! simple `MAJOR.MINOR.PATCH` strings, and the comparison we need
//! (older / equal / newer) is a numeric component-wise compare.
//! Pre-release suffixes are sorted numerically after the prefix
//! matches, which is good enough for our display + "is an update
//! available" gate.

use crate::updates::errors::{Result, UpdateError};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::time::Duration;

const CC_RELEASES_BASE: &str = "https://downloads.claude.ai/claude-code-releases";
const DESKTOP_FORMULAE_API: &str = "https://formulae.brew.sh/api/cask/claude.json";
const HTTP_TIMEOUT: Duration = Duration::from_secs(8);

/// CC release channel. Mirrors the values CC's own
/// `autoUpdatesChannel` setting accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Latest,
    Stable,
}

impl Channel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Channel::Latest => "latest",
            Channel::Stable => "stable",
        }
    }
}

impl std::str::FromStr for Channel {
    type Err = UpdateError;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "latest" => Ok(Channel::Latest),
            "stable" => Ok(Channel::Stable),
            other => Err(UpdateError::Parse(format!(
                "unknown channel: {other:?} (expected 'latest' or 'stable')"
            ))),
        }
    }
}

/// One row from the Homebrew Cask formulae API for the Claude desktop
/// app. The cask `version` field is shaped `"<semver>,<sha>"`, e.g.
/// `"1.5354.0,9a9e3d5a4..."`. We split on the comma so the UI can
/// show the human version without the build hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopRelease {
    pub version: String,
    pub commit_sha: Option<String>,
    /// Direct .zip download URL, as published by the Homebrew Cask.
    /// Pattern: `https://downloads.claude.ai/releases/darwin/universal/<v>/Claude-<sha>.zip`
    pub download_url: String,
    /// SHA256 of the zip, if the cask carries it. Used to gate the
    /// install path before we even hit `codesign`.
    pub sha256: Option<String>,
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .user_agent(concat!("Claudepot/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("reqwest client builder")
}

/// Fetch the latest CC CLI version for the chosen channel.
///
/// The endpoint returns a plain-text version string, e.g. `"2.1.126\n"`.
/// We trim and validate the shape lightly — an HTML error page would
/// otherwise be silently parsed as a "version".
pub async fn fetch_cli_latest(channel: Channel) -> Result<String> {
    let url = format!("{CC_RELEASES_BASE}/{}", channel.as_str());
    let body = http_client()
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let trimmed = body.trim();
    if !looks_like_version(trimmed) {
        return Err(UpdateError::Parse(format!(
            "endpoint {url} returned non-version: {:?}",
            trimmed.chars().take(80).collect::<String>()
        )));
    }
    Ok(trimmed.to_string())
}

/// Fetch the latest Claude Desktop release info via the Homebrew
/// formulae API. We use this instead of the canonical
/// `claude.ai/api/desktop/.../redirect` endpoint because the latter
/// is Cloudflare-protected and 403s every non-browser UA.
///
/// Brew's autobump runs within hours of a release, so the lag is small.
pub async fn fetch_desktop_latest() -> Result<DesktopRelease> {
    #[derive(Deserialize)]
    struct CaskJson {
        version: String,
        url: String,
        sha256: Option<String>,
    }
    let body: CaskJson = http_client()
        .get(DESKTOP_FORMULAE_API)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let (ver, sha) = match body.version.split_once(',') {
        Some((v, s)) => (v.to_string(), Some(s.to_string())),
        None => (body.version.clone(), None),
    };
    if !looks_like_version(&ver) {
        return Err(UpdateError::Parse(format!(
            "formulae returned non-version: {:?}",
            body.version
        )));
    }
    Ok(DesktopRelease {
        version: ver,
        commit_sha: sha,
        download_url: body.url,
        sha256: body.sha256,
    })
}

fn looks_like_version(s: &str) -> bool {
    if s.is_empty() || s.len() > 64 {
        return false;
    }
    let mut had_digit = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            had_digit = true;
        } else if c != '.' && c != '-' && !c.is_ascii_alphabetic() {
            return false;
        }
    }
    had_digit
}

/// Compare two version strings component-by-component, numerically
/// where possible. Returns `installed.cmp(latest)` semantics:
///
/// - `Less`    → installed is older; an update is available
/// - `Equal`   → installed matches latest
/// - `Greater` → installed is newer (e.g., on `latest` channel ahead
///   of `stable` floor)
///
/// Pre-release suffixes (e.g., `2.1.89-beta`) are split as separate
/// numeric components after the dotted prefix; non-numeric tokens
/// parse as zero, so `2.1.89` compares Equal to `2.1.89-rc1`. The UI
/// should not rely on this comparator for pre-release ordering.
pub fn compare_versions(installed: &str, latest: &str) -> Ordering {
    let installed_parts = parse_version_components(installed);
    let latest_parts = parse_version_components(latest);
    installed_parts.cmp(&latest_parts)
}

fn parse_version_components(s: &str) -> Vec<u64> {
    s.split(|c: char| c == '.' || c == '-')
        .map(|p| p.parse::<u64>().unwrap_or(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_channel() {
        assert_eq!("latest".parse::<Channel>().unwrap(), Channel::Latest);
        assert_eq!("STABLE".parse::<Channel>().unwrap(), Channel::Stable);
        assert_eq!("  Stable ".parse::<Channel>().unwrap(), Channel::Stable);
        assert!("nightly".parse::<Channel>().is_err());
    }

    #[test]
    fn version_compare_orderings() {
        assert_eq!(compare_versions("2.1.126", "2.1.126"), Ordering::Equal);
        assert_eq!(compare_versions("2.1.118", "2.1.126"), Ordering::Less);
        assert_eq!(compare_versions("2.1.200", "2.1.126"), Ordering::Greater);
        assert_eq!(compare_versions("1.5354.0", "1.5354.1"), Ordering::Less);
        assert_eq!(compare_versions("2.0.0", "2.0"), Ordering::Greater);
        assert_eq!(compare_versions("3.0.0", "2.999.999"), Ordering::Greater);
    }

    #[test]
    fn looks_like_version_filters_html() {
        assert!(looks_like_version("2.1.126"));
        assert!(looks_like_version("1.5354.0"));
        assert!(looks_like_version("2.1.89-beta"));
        assert!(!looks_like_version(""));
        assert!(!looks_like_version("<html>"));
        assert!(!looks_like_version("Cloudflare error 403"));
        assert!(!looks_like_version("not.a.version!"));
    }

    #[test]
    fn channel_as_str_roundtrips() {
        assert_eq!(Channel::Latest.as_str(), "latest");
        assert_eq!(Channel::Stable.as_str(), "stable");
        assert_eq!(
            Channel::Latest.as_str().parse::<Channel>().unwrap(),
            Channel::Latest
        );
        assert_eq!(
            Channel::Stable.as_str().parse::<Channel>().unwrap(),
            Channel::Stable
        );
    }
}
