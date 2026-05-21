//! Claudepot's own desktop self-updater release channel.
//!
//! Distinct from [`crate::updates::version::Channel`] — that enum
//! mirrors *Claude Code's* `autoUpdatesChannel` setting (the CLI tool
//! Claudepot manages). This [`ReleaseChannel`] selects which of
//! Claudepot's own GitHub-hosted updater manifests the in-app updater
//! reads:
//!
//! - [`ReleaseChannel::Stable`] — only stable releases. Resolves to
//!   the `…/releases/latest/download/latest.json` endpoint, which
//!   GitHub always points at the newest *non-prerelease* release.
//! - [`ReleaseChannel::Beta`] — prereleases too. Resolves to a
//!   fixed-tag manifest (`updater-manifest/latest-beta.json`) that CI
//!   `--clobber`-uploads on every `v*-beta.*` tag. GitHub's
//!   `/releases/latest/` URL skips prereleases, so beta needs its own
//!   permanent endpoint.
//!
//! This module is pure: it only owns the enum and the channel →
//! endpoint-URL mapping. It deliberately does *not* depend on the
//! `url` crate — the URLs are returned as `&'static str` and parsed
//! to `url::Url` by the Tauri command that calls
//! `UpdaterBuilder::endpoints`. Keeping `claudepot-core` free of the
//! `url` dependency respects the crate-separation rule
//! (`.claude/rules/architecture.md`).

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// The GitHub repository the updater manifests are hosted on. Both
/// endpoint constants below must live under this base; the tests
/// assert that. Test-only — the endpoints are full literals so they
/// stay greppable, this is just the shared prefix the tests check.
#[cfg(test)]
const REPO_BASE: &str = "https://github.com/xiaolai/claudepot-app";

/// Stable updater manifest. GitHub's `/releases/latest/` always
/// resolves to the newest release that is *not* flagged
/// `prerelease`, so stable users on this endpoint never see a beta.
/// Byte-identical to the single endpoint historically hardcoded in
/// `tauri.conf.json` `plugins.updater.endpoints` — already-installed
/// apps that never touch the channel feature keep updating off this.
pub const STABLE_ENDPOINT: &str =
    "https://github.com/xiaolai/claudepot-app/releases/latest/download/latest.json";

/// Beta updater manifest. Hosted on a permanent fixed-tag release
/// (`updater-manifest`) that CI `--clobber`-uploads `latest-beta.json`
/// to on every `v*-beta.*` tag. A fixed tag is required because
/// GitHub's `/releases/latest/` URL skips prereleases — a beta
/// release's manifest would otherwise have no stable URL.
pub const BETA_ENDPOINT: &str =
    "https://github.com/xiaolai/claudepot-app/releases/download/updater-manifest/latest-beta.json";

/// Which set of Claudepot releases the in-app updater offers.
///
/// Serialized lowercase (`"stable"` / `"beta"`) so it round-trips
/// cleanly through `preferences.json` and across the Tauri IPC
/// boundary as a plain JSON string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseChannel {
    /// Only stable (non-prerelease) releases. The conservative
    /// default — a fresh install never auto-opts into betas.
    #[default]
    Stable,
    /// Stable releases plus prereleases (`vX.Y.Z-beta.N`).
    Beta,
}

impl ReleaseChannel {
    /// The lowercase wire form. Matches the serde representation, so
    /// `as_str()` and a `serde_json` round-trip always agree.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReleaseChannel::Stable => "stable",
            ReleaseChannel::Beta => "beta",
        }
    }

    /// The updater-manifest endpoint URL for this channel.
    ///
    /// `&'static str` rather than `url::Url` on purpose — see the
    /// module docs. The caller (a Tauri command) parses it.
    pub fn endpoint(&self) -> &'static str {
        match self {
            ReleaseChannel::Stable => STABLE_ENDPOINT,
            ReleaseChannel::Beta => BETA_ENDPOINT,
        }
    }

    /// The endpoint list to hand to `UpdaterBuilder::endpoints`.
    ///
    /// One URL today; returned as a `Vec` so the call site matches
    /// the plugin's `endpoints(Vec<Url>)` signature without an extra
    /// wrapping allocation at the call site, and so a future
    /// fallback-mirror endpoint can be added here without touching
    /// the command code.
    pub fn endpoints(&self) -> Vec<&'static str> {
        vec![self.endpoint()]
    }
}

impl fmt::Display for ReleaseChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ReleaseChannel {
    type Err = String;

    /// Case-insensitive, whitespace-trimmed parse. Returns a plain
    /// `String` error — this is consumed at the Tauri command
    /// boundary where errors become user-facing strings anyway, so a
    /// dedicated error enum would buy nothing.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "stable" => Ok(ReleaseChannel::Stable),
            "beta" => Ok(ReleaseChannel::Beta),
            other => Err(format!(
                "unknown release channel {other:?} (expected 'stable' or 'beta')"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_release_channel_default_is_stable() {
        // The conservative default — a fresh install with no
        // persisted preference must land on Stable, never Beta.
        assert_eq!(ReleaseChannel::default(), ReleaseChannel::Stable);
    }

    #[test]
    fn test_release_channel_as_str_round_trips() {
        for c in [ReleaseChannel::Stable, ReleaseChannel::Beta] {
            assert_eq!(c.as_str().parse::<ReleaseChannel>().unwrap(), c);
        }
    }

    #[test]
    fn test_release_channel_from_str_is_case_insensitive() {
        assert_eq!(
            "STABLE".parse::<ReleaseChannel>().unwrap(),
            ReleaseChannel::Stable
        );
        assert_eq!(
            "  Beta  ".parse::<ReleaseChannel>().unwrap(),
            ReleaseChannel::Beta
        );
    }

    #[test]
    fn test_release_channel_from_str_rejects_unknown() {
        let err = "nightly".parse::<ReleaseChannel>().unwrap_err();
        assert!(err.contains("nightly"), "error names the bad input: {err}");
        assert!(
            err.contains("stable") && err.contains("beta"),
            "error lists the valid values: {err}"
        );
    }

    #[test]
    fn test_release_channel_serde_round_trip_is_lowercase() {
        // The wire form must be a bare lowercase string so it
        // round-trips through preferences.json and Tauri IPC.
        let json = serde_json::to_string(&ReleaseChannel::Beta).unwrap();
        assert_eq!(json, "\"beta\"");
        let back: ReleaseChannel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ReleaseChannel::Beta);

        let json = serde_json::to_string(&ReleaseChannel::Stable).unwrap();
        assert_eq!(json, "\"stable\"");
    }

    #[test]
    fn test_stable_endpoint_uses_latest_download_path() {
        // The stable endpoint MUST keep using GitHub's
        // `/releases/latest/download/` path — that URL self-tracks
        // the newest non-prerelease release. Changing this would
        // strand every already-installed stable app.
        let url = ReleaseChannel::Stable.endpoint();
        assert!(url.starts_with(REPO_BASE), "endpoint under the repo base");
        assert!(
            url.contains("/releases/latest/download/latest.json"),
            "stable endpoint resolves to latest.json: {url}"
        );
    }

    #[test]
    fn test_beta_endpoint_uses_fixed_manifest_tag() {
        // Beta MUST resolve to the permanent `updater-manifest`
        // fixed-tag release — a prerelease has no `/releases/latest/`
        // URL of its own.
        let url = ReleaseChannel::Beta.endpoint();
        assert!(url.starts_with(REPO_BASE), "endpoint under the repo base");
        assert!(
            url.contains("/releases/download/updater-manifest/latest-beta.json"),
            "beta endpoint resolves to the fixed-tag manifest: {url}"
        );
    }

    #[test]
    fn test_endpoints_returns_the_single_channel_url() {
        assert_eq!(
            ReleaseChannel::Stable.endpoints(),
            vec![STABLE_ENDPOINT]
        );
        assert_eq!(ReleaseChannel::Beta.endpoints(), vec![BETA_ENDPOINT]);
    }

    #[test]
    fn test_display_matches_as_str() {
        assert_eq!(ReleaseChannel::Stable.to_string(), "stable");
        assert_eq!(ReleaseChannel::Beta.to_string(), "beta");
    }
}
