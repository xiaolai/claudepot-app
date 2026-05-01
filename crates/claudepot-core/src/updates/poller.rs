//! Background-poll core: runs one probe + auto-install cycle and
//! returns the outcome.
//!
//! This module exists so the Tauri command (`updates_check_now`) and
//! the spawned background loop both call into the same code — the
//! policy that decides "auto-install fires when X" lives in exactly
//! one place. The Tauri side owns side effects (events, tray, OS
//! notifications); core stays Tauri-free per
//! `.claude/rules/architecture.md`.

use crate::updates::cli_driver::run_claude_update;
use crate::updates::desktop_driver::install_desktop_latest;
use crate::updates::detect::{detect_cli_installs, detect_desktop_install, is_desktop_running};
use crate::updates::settings_bridge;
use crate::updates::state::UpdateStateMutex;
use crate::updates::{compare_versions, fetch_cli_latest, fetch_desktop_latest, Channel};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::sync::Mutex;

/// Result of an auto-install attempt for one surface (CLI or Desktop).
///
/// `Disabled` is the inert resting state — toggle is off; nothing
/// to do. `UpToDate` is the toggle-on-but-no-delta path. `Skipped`
/// means we wanted to install but a precondition blocked
/// (DISABLE_UPDATES, Desktop running, no active install). The four
/// non-`Disabled` variants all carry enough text for the UI to
/// surface a one-line banner without re-deriving anything.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AutoInstallOutcome {
    Disabled,
    UpToDate,
    Skipped { reason: String },
    Installed { version: Option<String> },
    Failed { error: String },
}

impl AutoInstallOutcome {
    /// True iff this cycle actually fired an install (succeeded OR
    /// failed). Useful for OS-notification gating: only notify on
    /// state transitions, not on each "still up to date" tick.
    pub fn fired(&self) -> bool {
        matches!(self, Self::Installed { .. } | Self::Failed { .. })
    }
}

/// Snapshot of what one probe-and-maybe-install cycle observed.
/// Owned by the caller; the cache mutation has already been
/// persisted by the time this returns.
#[derive(Debug, Clone)]
pub struct CheckCycleOutcome {
    pub cli_latest: Option<String>,
    pub desktop_latest: Option<String>,
    pub desktop_latest_sha: Option<String>,
    pub cli_update_available: bool,
    pub desktop_update_available: bool,
    pub cli_auto: AutoInstallOutcome,
    pub desktop_auto: AutoInstallOutcome,
}

/// Run one probe + auto-install cycle. Mutates `UpdateState.cache`
/// in-place and persists. Pure of Tauri — callers (the Tauri
/// command and the background spawned loop) own side-effect
/// dispatch.
pub async fn run_one_check_cycle(state: &UpdateStateMutex) -> CheckCycleOutcome {
    let cc = settings_bridge::read().unwrap_or_default();
    let channel = cc
        .auto_updates_channel
        .as_deref()
        .and_then(|s| s.parse::<Channel>().ok())
        .unwrap_or(Channel::Latest);

    // Probe — both endpoints. Network failure on one doesn't block
    // the other.
    let cli_latest = fetch_cli_latest(channel).await.ok();
    let desktop_release = fetch_desktop_latest().await.ok();

    // Snapshot the settings we need; mutex held briefly.
    //
    // Note: we do NOT save here. The caller is responsible for
    // persisting after any post-cycle mutations land (the watcher
    // also writes `last_notified_version` after computing OS-toast
    // signals; doing two separate saves race each other on
    // out-of-order completion). Instead the caller awaits one
    // consolidated save after all mutations finish — see
    // `src-tauri/src/updates_watcher.rs::tick`.
    let (cli_settings, desktop_settings) = {
        let mut guard = match state.0.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let now = chrono::Utc::now().timestamp();
        if let Some(v) = cli_latest.clone() {
            guard.cache.cli.last_check_unix = Some(now);
            guard.cache.cli.last_error = None;
            match channel {
                Channel::Latest => guard.cache.cli.last_known_latest = Some(v),
                Channel::Stable => guard.cache.cli.last_known_stable = Some(v),
            }
        } else {
            guard.cache.cli.last_error = Some("network probe failed".into());
        }
        if let Some(d) = desktop_release.as_ref() {
            guard.cache.desktop.last_check_unix = Some(now);
            guard.cache.desktop.last_error = None;
            guard.cache.desktop.last_known_latest = Some(d.version.clone());
            guard.cache.desktop.last_known_sha = d.commit_sha.clone();
        } else {
            guard.cache.desktop.last_error = Some("network probe failed".into());
        }
        (guard.settings.cli.clone(), guard.settings.desktop.clone())
    };

    // Re-detect post-probe so the comparisons reflect the live state.
    let installs = detect_cli_installs();
    let active = installs.iter().find(|c| c.is_active);
    let cli_update_available = match (
        active.and_then(|a| a.version.as_deref()),
        cli_latest.as_deref(),
    ) {
        (Some(have), Some(want)) => compare_versions(have, want) == Ordering::Less,
        _ => false,
    };

    let desktop_install = detect_desktop_install();
    let desktop_update_available = match (
        desktop_install.as_ref().and_then(|i| i.version.as_deref()),
        desktop_release.as_ref().map(|d| d.version.as_str()),
    ) {
        (Some(have), Some(want)) => compare_versions(have, want) == Ordering::Less,
        _ => false,
    };

    // Auto-install pass — CLI
    let cli_auto = if !cli_settings.force_update_on_check {
        AutoInstallOutcome::Disabled
    } else if active.is_none() {
        AutoInstallOutcome::Skipped {
            reason: "no active CC install detected".into(),
        }
    } else if cli_latest.is_none() {
        AutoInstallOutcome::Skipped {
            reason: "upstream probe failed".into(),
        }
    } else if !cli_update_available {
        AutoInstallOutcome::UpToDate
    } else {
        match run_claude_update().await {
            Ok(out) => AutoInstallOutcome::Installed {
                version: out.installed_after,
            },
            Err(e) => AutoInstallOutcome::Failed {
                error: e.to_string(),
            },
        }
    };

    // Auto-install pass — Desktop
    let desktop_auto = if !desktop_settings.auto_install_when_quit {
        AutoInstallOutcome::Disabled
    } else {
        match desktop_install.as_ref() {
            None => AutoInstallOutcome::Skipped {
                reason: "no Claude Desktop install detected".into(),
            },
            Some(install) if !install.manageable => AutoInstallOutcome::Skipped {
                reason: format!(
                    "Desktop is managed elsewhere ({}) — Claudepot can't drive updates here",
                    install.source.label()
                ),
            },
            Some(_) if is_desktop_running() => AutoInstallOutcome::Skipped {
                reason: "Desktop is currently running — will retry next cycle".into(),
            },
            Some(_) if desktop_release.is_none() => AutoInstallOutcome::Skipped {
                reason: "upstream probe failed".into(),
            },
            Some(_) if !desktop_update_available => AutoInstallOutcome::UpToDate,
            Some(_) => match install_desktop_latest().await {
                Ok(out) => AutoInstallOutcome::Installed {
                    version: out.version_after,
                },
                Err(e) => AutoInstallOutcome::Failed {
                    error: e.to_string(),
                },
            },
        }
    };

    // After-install correction: a successful auto-install MAY have
    // resolved the delta in this same cycle. Re-detect the installed
    // version and recompute against the upstream latest — don't blindly
    // force the flag false, because a "successful" install whose
    // version didn't actually change (broken updater contract) would
    // produce a false negative and silently hide the still-pending
    // update. Re-detecting also catches the rare case where two
    // versions ship within one tick.
    let cli_update_available = if matches!(cli_auto, AutoInstallOutcome::Installed { .. }) {
        let post = detect_cli_installs();
        let post_installed = post
            .iter()
            .find(|c| c.is_active)
            .and_then(|c| c.version.as_deref().map(|s| s.to_string()));
        match (post_installed.as_deref(), cli_latest.as_deref()) {
            (Some(have), Some(want)) => compare_versions(have, want) == Ordering::Less,
            _ => false,
        }
    } else {
        cli_update_available
    };
    let desktop_update_available =
        if matches!(desktop_auto, AutoInstallOutcome::Installed { .. }) {
            let post = detect_desktop_install();
            let post_installed = post.as_ref().and_then(|i| i.version.as_deref().map(|s| s.to_string()));
            match (post_installed.as_deref(), desktop_release.as_ref().map(|d| d.version.as_str())) {
                (Some(have), Some(want)) => compare_versions(have, want) == Ordering::Less,
                _ => false,
            }
        } else {
            desktop_update_available
        };

    CheckCycleOutcome {
        cli_latest,
        desktop_latest: desktop_release.as_ref().map(|d| d.version.clone()),
        desktop_latest_sha: desktop_release.and_then(|d| d.commit_sha),
        cli_update_available,
        desktop_update_available,
        cli_auto,
        desktop_auto,
    }
}

/// Persist the current state snapshot. Awaitable so callers can
/// serialize multiple mutations into a single ordered write.
///
/// **Save-order guarantee**: acquires `state.save_lock()` before
/// snapshotting + writing. All save paths must go through this
/// function — direct calls to `UpdateState::save()` from a Tauri
/// command path can race the watcher and lose dedupe state.
///
/// Errors are logged and swallowed — saves are best-effort and
/// shouldn't fail user-visible operations.
pub async fn save_state(state: &UpdateStateMutex) {
    let _save_guard = state.save_lock().lock().await;
    let snap = match state.0.lock() {
        Ok(g) => g.clone(),
        Err(p) => p.into_inner().clone(),
    };
    let join = tokio::task::spawn_blocking(move || snap.save()).await;
    match join {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!("updates state save failed: {e}"),
        Err(e) => tracing::warn!("updates state save join failed: {e}"),
    }
}

/// Single-flight gate for the background poller. Prevents two cycles
/// running simultaneously when a long auto-install spans the next
/// tick boundary. Held for the full duration of `run_one_check_cycle`.
#[derive(Default)]
pub struct PollerGate(pub Mutex<bool>);

pub struct PollerLease<'a> {
    gate: &'a PollerGate,
}

impl PollerGate {
    /// Try to acquire the gate. Returns `None` if another cycle is
    /// in flight; the caller should skip its tick (the in-flight
    /// cycle will catch up).
    pub fn try_acquire(&self) -> Option<PollerLease<'_>> {
        let mut g = match self.0.lock() {
            Ok(x) => x,
            Err(p) => p.into_inner(),
        };
        if *g {
            None
        } else {
            *g = true;
            Some(PollerLease { gate: self })
        }
    }
}

impl Drop for PollerLease<'_> {
    fn drop(&mut self) {
        let mut g = match self.gate.0.lock() {
            Ok(x) => x,
            Err(p) => p.into_inner(),
        };
        *g = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_install_fired_classifies() {
        assert!(!AutoInstallOutcome::Disabled.fired());
        assert!(!AutoInstallOutcome::UpToDate.fired());
        assert!(!AutoInstallOutcome::Skipped { reason: "x".into() }.fired());
        assert!(AutoInstallOutcome::Installed { version: None }.fired());
        assert!(AutoInstallOutcome::Failed { error: "x".into() }.fired());
    }

    #[test]
    fn poller_gate_serializes() {
        let gate = PollerGate::default();
        let l1 = gate.try_acquire();
        assert!(l1.is_some());
        let l2 = gate.try_acquire();
        assert!(l2.is_none(), "second acquire should fail while first held");
        drop(l1);
        let l3 = gate.try_acquire();
        assert!(l3.is_some(), "release should allow new acquire");
    }
}
