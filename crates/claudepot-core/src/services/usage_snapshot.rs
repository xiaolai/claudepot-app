//! Disk snapshot of per-account Anthropic usage so non-GUI processes
//! (cron jobs, CC bash subprocesses, third-party bots) can read
//! utilization without needing keychain access.
//!
//! The Tauri app owns the writer (`src-tauri/src/usage_snapshot.rs`).
//! This module is the schema + atomic-write helper; it stays
//! Tauri-free so any caller (CLI, future daemon, tests) can use it.
//!
//! Path: `~/.claudepot/usage-snapshot.json` (override via
//! `CLAUDEPOT_DATA_DIR`). Mode 0600 — the email map reveals which
//! accounts are signed in, which is private even though no token
//! material crosses the disk.
//!
//! Consumer contract:
//! - Trust `fetched_at` if you're within `ttl_secs * 2` of it.
//! - Stale but usable if older.
//! - Skip the account if `status != "ok"`.
//! - Treat `written_at` older than 5 minutes as "Claudepot GUI is
//!   not running" — the snapshot is stale-but-historical.

use crate::account::Account;
use crate::fs_utils::atomic_write;
use crate::oauth::usage::{UsageResponse, UsageWindow};
use crate::services::usage_cache::UsageOutcome;
use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Standard filename inside `claudepot_data_dir()`.
pub const SNAPSHOT_FILENAME: &str = "usage-snapshot.json";

/// Default TTL the GUI's `UsageCache` uses (60s). Mirrored here so
/// consumers don't have to import the cache module.
pub const DEFAULT_TTL_SECS: u64 = 60;

/// `~/.claudepot/usage-snapshot.json` (or `$CLAUDEPOT_DATA_DIR`'d).
pub fn snapshot_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(SNAPSHOT_FILENAME)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub schema_version: u32,
    pub written_at: DateTime<Utc>,
    /// Keyed by account uuid (string form). BTreeMap so the on-disk
    /// JSON has stable key ordering for diff-friendly reads.
    pub accounts: BTreeMap<String, AccountSnapshot>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AccountSnapshot {
    pub email: String,
    /// "max", "pro", "free" — whatever the server reported. Mirrors
    /// `Account::subscription_type`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_type: Option<String>,
    pub cli_active: bool,
    pub desktop_active: bool,
    pub status: AccountStatus,
    /// When the data was fetched. For `ok` entries this is the
    /// real fetch time (computed back from `age_secs`); for non-ok
    /// entries it's the snapshot's `written_at` (the attempt time).
    pub fetched_at: DateTime<Utc>,
    pub ttl_secs: u64,
    /// Only present when `status == "ok"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageWindows>,
    /// For `rate_limited`: server-suggested retry delay in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
    /// For `error`: short error text for diagnostics. Never includes
    /// secrets (the underlying `UsageOutcome::Error` is already
    /// audited for this).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Ok,
    NoCredentials,
    Expired,
    RateLimited,
    Error,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UsageWindows {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<WindowSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<WindowSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seven_day_opus: Option<WindowSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seven_day_sonnet: Option<WindowSnapshot>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WindowSnapshot {
    pub utilization: f64,
    /// Preserved verbatim from Anthropic's response so consumers see
    /// the same offset the server sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<DateTime<FixedOffset>>,
}

impl WindowSnapshot {
    fn from_window(w: &UsageWindow) -> Self {
        Self {
            utilization: w.utilization,
            resets_at: w.resets_at,
        }
    }
}

impl UsageWindows {
    fn from_response(r: &UsageResponse) -> Self {
        Self {
            five_hour: r.five_hour.as_ref().map(WindowSnapshot::from_window),
            seven_day: r.seven_day.as_ref().map(WindowSnapshot::from_window),
            seven_day_opus: r.seven_day_opus.as_ref().map(WindowSnapshot::from_window),
            seven_day_sonnet: r.seven_day_sonnet.as_ref().map(WindowSnapshot::from_window),
        }
    }
}

/// Build a snapshot from the current account list and the most recent
/// fetch outcomes. Accounts without an outcome are skipped — including
/// them with a synthesized `status` would imply we know something we
/// don't.
pub fn build(accounts: &[Account], outcomes: &HashMap<Uuid, UsageOutcome>) -> UsageSnapshot {
    let now = Utc::now();
    let mut entries = BTreeMap::new();
    for a in accounts {
        let Some(outcome) = outcomes.get(&a.uuid) else {
            continue;
        };
        entries.insert(
            a.uuid.to_string(),
            AccountSnapshot::from_outcome(a, outcome, now),
        );
    }
    UsageSnapshot {
        schema_version: 1,
        written_at: now,
        accounts: entries,
    }
}

impl AccountSnapshot {
    fn from_outcome(a: &Account, outcome: &UsageOutcome, now: DateTime<Utc>) -> Self {
        let mut entry = Self {
            email: a.email.clone(),
            subscription_type: a.subscription_type.clone(),
            cli_active: a.is_cli_active,
            desktop_active: a.is_desktop_active,
            status: AccountStatus::Error,
            fetched_at: now,
            ttl_secs: DEFAULT_TTL_SECS,
            usage: None,
            retry_after_secs: None,
            error: None,
        };
        match outcome {
            UsageOutcome::Fresh { response, age_secs }
            | UsageOutcome::Stale { response, age_secs } => {
                entry.status = AccountStatus::Ok;
                entry.fetched_at = now - chrono::Duration::seconds(*age_secs as i64);
                entry.usage = Some(UsageWindows::from_response(response));
            }
            UsageOutcome::NoCredentials => entry.status = AccountStatus::NoCredentials,
            UsageOutcome::Expired => entry.status = AccountStatus::Expired,
            UsageOutcome::RateLimited { retry_after_secs } => {
                entry.status = AccountStatus::RateLimited;
                entry.retry_after_secs = Some(*retry_after_secs);
            }
            UsageOutcome::Error(msg) => {
                entry.status = AccountStatus::Error;
                entry.error = Some(msg.clone());
            }
        }
        entry
    }
}

/// Atomic write to `path` via `fs_utils::atomic_write`. Mode 0600 on
/// Unix, parent directory created if missing.
pub fn write(path: &Path, snapshot: &UsageSnapshot) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(snapshot)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    atomic_write(path, &json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::usage::UsageWindow;

    fn account(uuid: Uuid, email: &str, cli_active: bool) -> Account {
        Account {
            uuid,
            email: email.into(),
            org_uuid: None,
            org_name: None,
            subscription_type: Some("max".into()),
            rate_limit_tier: None,
            created_at: Utc::now(),
            last_cli_switch: None,
            last_desktop_switch: None,
            has_cli_credentials: true,
            has_desktop_profile: false,
            is_cli_active: cli_active,
            is_desktop_active: false,
            verified_email: None,
            verified_at: None,
            verify_status: "ok".into(),
        }
    }

    fn ok_response(util: f64) -> UsageResponse {
        UsageResponse {
            five_hour: Some(UsageWindow {
                utilization: util,
                resets_at: None,
            }),
            seven_day: Some(UsageWindow {
                utilization: util * 0.5,
                resets_at: None,
            }),
            seven_day_oauth_apps: None,
            seven_day_opus: None,
            seven_day_sonnet: None,
            seven_day_cowork: None,
            iguana_necktie: None,
            extra_usage: None,
            unknown: Default::default(),
        }
    }

    #[test]
    fn build_skips_accounts_without_outcomes() {
        let a = account(Uuid::new_v4(), "a@example.com", true);
        let b = account(Uuid::new_v4(), "b@example.com", false);
        let mut outcomes = HashMap::new();
        outcomes.insert(
            a.uuid,
            UsageOutcome::Fresh {
                response: ok_response(42.0),
                age_secs: 3,
            },
        );
        let snap = build(&[a.clone(), b], &outcomes);
        assert_eq!(snap.accounts.len(), 1);
        let entry = snap.accounts.get(&a.uuid.to_string()).unwrap();
        assert_eq!(entry.status, AccountStatus::Ok);
        assert_eq!(entry.email, "a@example.com");
        assert!(entry.usage.is_some());
    }

    #[test]
    fn build_maps_each_outcome_variant() {
        let a = account(Uuid::new_v4(), "a@example.com", true);
        let b = account(Uuid::new_v4(), "b@example.com", false);
        let c = account(Uuid::new_v4(), "c@example.com", false);
        let d = account(Uuid::new_v4(), "d@example.com", false);
        let e = account(Uuid::new_v4(), "e@example.com", false);
        let mut outcomes = HashMap::new();
        outcomes.insert(a.uuid, UsageOutcome::NoCredentials);
        outcomes.insert(b.uuid, UsageOutcome::Expired);
        outcomes.insert(
            c.uuid,
            UsageOutcome::RateLimited {
                retry_after_secs: 30,
            },
        );
        outcomes.insert(d.uuid, UsageOutcome::Error("network down".into()));
        outcomes.insert(
            e.uuid,
            UsageOutcome::Stale {
                response: ok_response(10.0),
                age_secs: 120,
            },
        );
        let snap = build(
            &[a.clone(), b.clone(), c.clone(), d.clone(), e.clone()],
            &outcomes,
        );
        assert_eq!(
            snap.accounts.get(&a.uuid.to_string()).unwrap().status,
            AccountStatus::NoCredentials
        );
        assert_eq!(
            snap.accounts.get(&b.uuid.to_string()).unwrap().status,
            AccountStatus::Expired
        );
        let c_entry = snap.accounts.get(&c.uuid.to_string()).unwrap();
        assert_eq!(c_entry.status, AccountStatus::RateLimited);
        assert_eq!(c_entry.retry_after_secs, Some(30));
        let d_entry = snap.accounts.get(&d.uuid.to_string()).unwrap();
        assert_eq!(d_entry.status, AccountStatus::Error);
        assert_eq!(d_entry.error.as_deref(), Some("network down"));
        let e_entry = snap.accounts.get(&e.uuid.to_string()).unwrap();
        assert_eq!(e_entry.status, AccountStatus::Ok);
        assert!(e_entry.usage.is_some());
    }

    #[test]
    fn write_then_read_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SNAPSHOT_FILENAME);
        let a = account(Uuid::new_v4(), "round@example.com", true);
        let mut outcomes = HashMap::new();
        outcomes.insert(
            a.uuid,
            UsageOutcome::Fresh {
                response: ok_response(81.0),
                age_secs: 5,
            },
        );
        let snap = build(std::slice::from_ref(&a), &outcomes);
        write(&path, &snap).unwrap();
        let read = std::fs::read_to_string(&path).unwrap();
        let parsed: UsageSnapshot = serde_json::from_str(&read).unwrap();
        assert_eq!(parsed.schema_version, 1);
        assert_eq!(parsed.accounts.len(), 1);
        let entry = parsed.accounts.get(&a.uuid.to_string()).unwrap();
        assert_eq!(entry.email, "round@example.com");
        assert_eq!(entry.status, AccountStatus::Ok);
        assert_eq!(
            entry
                .usage
                .as_ref()
                .unwrap()
                .five_hour
                .as_ref()
                .unwrap()
                .utilization,
            81.0
        );
    }
}
