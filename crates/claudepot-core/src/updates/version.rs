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

/// A CC release channel Claudepot **has a published version feed for**
/// — i.e. one where `{CC_RELEASES_BASE}/{channel}` answers. That is the
/// whole meaning of this type: `as_str` builds a URL.
///
/// It is deliberately *narrower* than the set of values CC accepts. CC
/// 2.1.241's schema is `["latest","stable","rc"]`, but there is no
/// `rc` feed — `GET /claude-code-releases/rc` returns 404 NoSuchKey
/// (measured 2026-08-24, alongside 200s for `latest` and `stable`).
/// Adding an `Rc` variant here would create a channel whose only
/// possible fetch is a 404, so the third value is modelled by
/// [`CcChannel::Untracked`] instead.
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

/// What CC's `autoUpdatesChannel` key actually says, read as three
/// states rather than coerced to two.
///
/// Every reader used to be `raw.parse::<Channel>().ok().unwrap_or(
/// Channel::Latest)`, which silently answers "latest" for a user who
/// is genuinely on `rc`: the panel lit the `latest` button and the
/// version comparison ran against the `latest` baseline. A misreport,
/// not a crash — so nothing exercising only `latest`/`stable` could
/// catch it.
///
/// The distinction that matters is *not* valid-vs-invalid. `rc` is a
/// perfectly valid setting; Claudepot simply has no feed for it. So
/// `Untracked` carries the raw string and is reported, never repaired
/// — same reasoning as the retention pane's three-state
/// `SettingValue`, where collapsing "absent" into "present but
/// unusable" pointed the user at the wrong remedy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CcChannel {
    /// Key absent. CC's own default is `latest`, so that is what the
    /// probe uses — but "unset" is kept distinct from an explicit
    /// `"latest"` because only one of the two is the user's choice.
    Unset,
    /// A channel with a feed. Probe it, cache it, compare against it.
    Tracked(Channel),
    /// A value CC accepts and Claudepot cannot track — `rc` on
    /// 2.1.241, which CC's own `/status` and `/config` display as
    /// **"slow"**. Also covers any future value, so the next channel
    /// CC adds degrades to "we can't tell you" rather than to a
    /// confident wrong answer.
    Untracked(String),
}

impl CcChannel {
    /// Read the raw `autoUpdatesChannel` value.
    pub fn read(raw: Option<&str>) -> Self {
        match raw.map(str::trim) {
            None | Some("") => CcChannel::Unset,
            Some(v) => match v.parse::<Channel>() {
                Ok(c) => CcChannel::Tracked(c),
                Err(_) => CcChannel::Untracked(v.to_string()),
            },
        }
    }

    /// The channel to probe and cache under, or `None` when there is
    /// no feed. `Unset` probes `latest` because that is CC's default.
    pub fn tracked(&self) -> Option<Channel> {
        match self {
            CcChannel::Unset => Some(Channel::Latest),
            CcChannel::Tracked(c) => Some(*c),
            CcChannel::Untracked(_) => None,
        }
    }

    /// The wire value to show the user — CC's own string, never a
    /// translated or normalized one. `.claude/rules/design.md` keeps
    /// CC setting values in English precisely because the user may
    /// have to type them.
    pub fn label(&self) -> &str {
        match self {
            CcChannel::Unset => Channel::Latest.as_str(),
            CcChannel::Tracked(c) => c.as_str(),
            CcChannel::Untracked(raw) => raw,
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

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .user_agent(concat!("Claudepot/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

/// Fetch the latest CC CLI version for the chosen channel.
///
/// The endpoint returns a plain-text version string, e.g. `"2.1.126\n"`.
/// We trim and validate the shape lightly — an HTML error page would
/// otherwise be silently parsed as a "version".
pub async fn fetch_cli_latest(channel: Channel) -> Result<String> {
    let url = format!("{CC_RELEASES_BASE}/{}", channel.as_str());
    let body = http_client()?
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
    let body: CaskJson = http_client()?
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
    s.split(['.', '-'])
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
    fn test_http_client_build_succeeds() {
        assert!(http_client().is_ok());
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

    // ── CcChannel: the three-state read of `autoUpdatesChannel` ──

    #[test]
    fn rc_is_untracked_not_latest() {
        // The reported defect. CC 2.1.241 accepts `rc` (its own UI
        // calls it "slow"); the old read coerced it to Latest and the
        // panel then claimed the user was on latest.
        let c = CcChannel::read(Some("rc"));
        assert_eq!(c, CcChannel::Untracked("rc".into()));
        assert_eq!(c.tracked(), None, "no feed, so nothing to compare against");
        assert_eq!(c.label(), "rc", "the user sees CC's own value");
    }

    #[test]
    fn a_future_channel_value_also_degrades_honestly() {
        // The point of `Untracked` carrying the raw string: the next
        // value CC adds must not silently become Latest either.
        let c = CcChannel::read(Some("nightly"));
        assert_eq!(c, CcChannel::Untracked("nightly".into()));
        assert_eq!(c.tracked(), None);
        assert_eq!(c.label(), "nightly");
    }

    #[test]
    fn tracked_channels_still_resolve() {
        assert_eq!(
            CcChannel::read(Some("stable")),
            CcChannel::Tracked(Channel::Stable)
        );
        assert_eq!(
            CcChannel::read(Some("stable")).tracked(),
            Some(Channel::Stable)
        );
        assert_eq!(CcChannel::read(Some("stable")).label(), "stable");
        assert_eq!(
            CcChannel::read(Some("  LATEST  ")),
            CcChannel::Tracked(Channel::Latest),
            "CC's parser is case- and whitespace-tolerant; so is ours"
        );
    }

    #[test]
    fn unset_probes_latest_but_is_not_an_explicit_latest() {
        for raw in [None, Some(""), Some("   ")] {
            let c = CcChannel::read(raw);
            assert_eq!(c, CcChannel::Unset, "{raw:?}");
            assert_eq!(
                c.tracked(),
                Some(Channel::Latest),
                "absent means CC's default, which is latest"
            );
            assert_eq!(c.label(), "latest");
        }
        assert_ne!(
            CcChannel::read(None),
            CcChannel::read(Some("latest")),
            "an absent key and a chosen `latest` are different facts"
        );
    }

    #[test]
    fn untracked_never_yields_a_channel_to_fetch() {
        // `fetch_cli_latest` builds `{base}/{channel}`, and there is no
        // `rc` object in that bucket — measured 404 NoSuchKey on
        // 2026-08-24. `tracked()` returning None is what keeps the
        // poller from making that request and filing the answer.
        assert!(CcChannel::Untracked("rc".into()).tracked().is_none());
    }
}
