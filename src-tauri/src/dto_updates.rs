//! DTOs for the `updates_*` Tauri commands.
//!
//! Re-exports the core types where they cross IPC unchanged, plus a
//! small number of composite DTOs whose only job is to bundle related
//! fields into one round-trip.

use claudepot_core::updates::{
    settings_bridge::CcUpdateSettings, CliInstall, DesktopInstall, UpdateSettings,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliStatusDto {
    /// Channel currently selected — what `autoUpdatesChannel` resolves
    /// to (defaults to "latest" if unset).
    pub channel: String,
    /// Detected installs in PATH order. The one with `is_active: true`
    /// is the binary that runs when the user types `claude`.
    pub installs: Vec<CliInstall>,
    /// Latest version reported by upstream for the active channel.
    /// `None` if the network probe failed.
    pub latest_remote: Option<String>,
    /// Cached value from the last successful probe. May be present
    /// even when `latest_remote` is `None` (offline / probe error).
    pub last_known: Option<String>,
    /// Unix timestamp of the last successful probe.
    pub last_check_unix: Option<i64>,
    /// Last error message from a failed probe, if any.
    pub last_error: Option<String>,
    /// CC's own settings — drives the UI hints around channel,
    /// minimumVersion, DISABLE_UPDATES.
    pub cc_settings: CcUpdateSettings,
    /// Number of currently-running CC processes (per
    /// `~/.local/state/claude/locks/`). Surfaces as "1 process active"
    /// next to the version string.
    pub running_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopStatusDto {
    /// `None` if no Desktop install detected.
    pub install: Option<DesktopInstall>,
    /// True iff Desktop is currently running. Auto-install gating
    /// requires this to be false.
    pub running: bool,
    /// Latest version reported by upstream (Homebrew formulae API).
    pub latest_remote: Option<String>,
    /// Latest release commit SHA, if known.
    pub latest_commit_sha: Option<String>,
    pub last_check_unix: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatesStatusDto {
    pub cli: CliStatusDto,
    pub desktop: DesktopStatusDto,
    pub settings: UpdateSettings,
    /// Result of the auto-install pass (only populated by
    /// `updates_check_now`; `updates_status_get` always reports
    /// `Disabled` here). The UI uses these to show "we just
    /// auto-updated" banners.
    #[serde(default = "auto_install_outcome_default_disabled")]
    pub cli_auto_outcome: AutoInstallOutcome,
    #[serde(default = "auto_install_outcome_default_disabled")]
    pub desktop_auto_outcome: AutoInstallOutcome,
}

/// Module-local helper because we can't add inherent methods to a
/// re-exported type from outside its defining crate. Used as a
/// `serde(default = ...)` callback on `UpdatesStatusDto`.
pub fn auto_install_outcome_default_disabled() -> AutoInstallOutcome {
    AutoInstallOutcome::Disabled
}

/// Result of a `claude update` invocation, marshalled across IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliInstallResultDto {
    pub stdout: String,
    pub stderr: String,
    pub installed_after: Option<String>,
}

/// Result of a Desktop install invocation, marshalled across IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopInstallResultDto {
    /// "brew" or "direct-zip"
    pub method: String,
    pub version_after: Option<String>,
    pub stdout: String,
    pub stderr: String,
}

/// Surface what happened on an auto-install fired from
/// `updates_check_now`. Re-exported from `claudepot_core::updates`
/// so the Tauri DTO and the core poller agree on the wire shape —
/// only one source of truth for the discriminated-union variants.
pub use claudepot_core::updates::AutoInstallOutcome;
