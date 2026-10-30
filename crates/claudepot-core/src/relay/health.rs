//! `GET /relay/health` (plan §7.1).
//!
//! Two answers, decided by the bearer:
//!
//! - **No valid relay token** — a missing, malformed, revoked or foreign
//!   (remote-control) token — gets `{"ok":true}` and nothing else.
//!   Liveness is all an unpaired caller may learn; which accounts or
//!   routes a server holds is not public.
//! - **A valid token** gets the full report, for the upstreams **that
//!   device is granted** only. Fingerprints are keyed by the client salt
//!   the request carries (`x-relay-client-salt`), so one client sees equal
//!   values across servers and two clients cannot correlate theirs.
//!
//! Readiness is per upstream: a subscription in `owner_conflict` says so
//! in its own entry and does not mark a healthy route beside it unready.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use super::devices::DeviceRecord;
use super::identity::{self, TargetKind};
use super::ledger::LatencySummary;
use super::server::RelayState;
use super::upstream::UpstreamError;
use crate::services::usage_snapshot::{self, AccountStatus, UsageSnapshot, WindowSnapshot};

/// Where the client salt travels on a health request.
pub const SALT_HEADER: &str = "x-relay-client-salt";

/// A usage snapshot older than this means the GUI that writes it is not
/// running, and its numbers say nothing about now.
pub const SNAPSHOT_MAX_AGE: Duration = Duration::from_secs(5 * 60);

/// Whether Claude Code on the serving machine is signed into this
/// upstream's account. Subscription upstreams only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerState {
    /// Not CC's login, as of the last check.
    Clear,
    /// CC signed into it; the lease stopped and stays stopped.
    Conflict,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Window {
    /// Percent, 0–100, as the usage endpoint reports it.
    pub utilization: f64,
    pub resets_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Utilization {
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
}

/// Reads the subscription's current utilization; `None` is unknown.
pub type UtilizationSource = Arc<dyn Fn(DateTime<Utc>) -> Option<Utilization> + Send + Sync>;

/// A window is known only while its reset is in the future: a value
/// whose reset has passed is `unknown`, not high (plan §7.2).
fn window(w: Option<&WindowSnapshot>, now: DateTime<Utc>) -> Option<Window> {
    let w = w?;
    let resets_at = w.resets_at?.with_timezone(&Utc);
    (resets_at > now && w.utilization.is_finite()).then_some(Window {
        utilization: w.utilization,
        resets_at,
    })
}

/// The account's utilization from the GUI's usage snapshot, or `None`
/// when the snapshot is missing, stale, or has no entry for it.
pub fn utilization_from_snapshot(
    snap: Option<&UsageSnapshot>,
    account_id: &str,
    now: DateTime<Utc>,
) -> Option<Utilization> {
    let snap = snap?;
    let age = now.signed_duration_since(snap.written_at);
    if age.num_seconds() < 0 || age.to_std().ok()? > SNAPSHOT_MAX_AGE {
        return None;
    }
    // The snapshot file is rewritten every few minutes, but each entry may
    // be a cached reading: its own `fetched_at` must be within
    // `ttl_secs * 2` and its status `ok` (the consumer contract in
    // `services::usage_snapshot`), or an old value reads as current.
    let entry = snap.accounts.get(account_id)?;
    let fetched = now.signed_duration_since(entry.fetched_at);
    let max_fetch_age = Duration::from_secs(entry.ttl_secs.saturating_mul(2));
    if entry.status != AccountStatus::Ok
        || fetched.num_seconds() < 0
        || fetched.to_std().ok()? > max_fetch_age
    {
        return None;
    }
    let usage = entry.usage.as_ref()?;
    let u = Utilization {
        five_hour: window(usage.five_hour.as_ref(), now),
        seven_day: window(usage.seven_day.as_ref(), now),
    };
    (u.five_hour.is_some() || u.seven_day.is_some()).then_some(u)
}

/// The production source for a Claudepot account id: the snapshot file,
/// read on each call — health is polled, not streamed, and the file is
/// small.
pub fn snapshot_source(account_id: uuid::Uuid) -> UtilizationSource {
    let id = account_id.to_string();
    Arc::new(move |now| {
        let snap = usage_snapshot::read(&usage_snapshot::snapshot_path());
        utilization_from_snapshot(snap.as_ref(), &id, now)
    })
}

/// What one upstream reports about itself. A handler that does not
/// implement it reports not-ready: unknown readiness must not be scored
/// as ready.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UpstreamStatus {
    pub credential_ready: bool,
    pub credential_error: Option<UpstreamError>,
    pub owner_state: Option<OwnerState>,
    pub utilization: Option<Utilization>,
    pub cooldown: Option<Duration>,
    /// The provider identity `(account_uuid, org_uuid)`, for the account
    /// fingerprint. Subscription only.
    pub account: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UpstreamHealth {
    pub id: String,
    pub kind: TargetKind,
    /// Is this the upstream bare `/v1/…` reaches?
    pub default: bool,
    pub target_fingerprint: String,
    pub accepted_generation: u64,
    pub current_generation: u64,
    /// The device's grant matches the current generation and epoch; a
    /// request now would not be refused `acceptance_required`.
    pub accepted: bool,
    pub credential_ready: bool,
    pub credential_error: Option<String>,
    pub owner_state: Option<OwnerState>,
    pub utilization: Option<Utilization>,
    pub cooldown_secs: Option<u64>,
    /// Requests this device could start now: the smaller of its own and
    /// the server's free slots.
    pub capacity_free: usize,
    pub latency: Vec<LatencySummary>,
    pub account_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HealthReport {
    pub ok: bool,
    pub server_id: String,
    pub version: String,
    pub upstreams: Vec<UpstreamHealth>,
}

pub(super) async fn report(s: &RelayState, device: &DeviceRecord, salt: &[u8]) -> HealthReport {
    let now = Instant::now();
    let capacity_free = s.capacity_free(&device.id);
    let mut upstreams = Vec::new();
    for handler in s.handlers() {
        let target = handler.target().clone();
        let Some(grant) = device.grant_for(&target.id) else {
            continue;
        };
        let st = handler.status().await;
        upstreams.push(UpstreamHealth {
            id: target.id.as_str().to_owned(),
            kind: target.kind,
            default: s.default_upstream() == Some(&target.id),
            target_fingerprint: identity::client_fingerprint(salt, &target.fingerprint),
            accepted_generation: grant.accepted_generation,
            current_generation: target.generation,
            accepted: grant.covers(target.generation, s.epoch()),
            credential_ready: st.credential_ready,
            credential_error: st.credential_error.map(|e| e.to_string()),
            owner_state: st.owner_state,
            utilization: st.utilization,
            cooldown_secs: st.cooldown.map(|d| d.as_secs().max(1)),
            capacity_free,
            latency: s.stats().summary(&target.id, now),
            account_fingerprint: st
                .account
                .as_ref()
                .map(|(a, o)| identity::account_fingerprint(salt, a, o)),
        });
    }
    upstreams.sort_by(|a, b| a.id.cmp(&b.id));
    HealthReport {
        ok: true,
        server_id: s.server_id().to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        upstreams,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type W = (f64, Option<&'static str>);

    fn snap(written: &str, five: W, seven: W) -> UsageSnapshot {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "written_at": written,
            "accounts": { "acct": {
                "email": "a@example.com",
                "cli_active": false,
                "desktop_active": false,
                "status": "ok",
                "fetched_at": written,
                "ttl_secs": 300,
                "usage": {
                    "five_hour": { "utilization": five.0, "resets_at": five.1 },
                    "seven_day": { "utilization": seven.0, "resets_at": seven.1 },
                },
            }},
        }))
        .unwrap()
    }

    fn at(t: &str) -> DateTime<Utc> {
        t.parse().unwrap()
    }

    #[test]
    fn a_fresh_snapshot_reports_both_windows_with_reset_times() {
        let s = snap(
            "2026-10-29T12:00:00Z",
            (42.0, Some("2026-10-29T15:00:00Z")),
            (80.5, Some("2026-11-02T00:00:00Z")),
        );
        let u = utilization_from_snapshot(Some(&s), "acct", at("2026-10-29T12:01:00Z")).unwrap();
        assert_eq!(u.five_hour.unwrap().utilization, 42.0);
        assert_eq!(u.seven_day.unwrap().resets_at, at("2026-11-02T00:00:00Z"));
    }

    #[test]
    fn a_passed_reset_is_unknown_not_high() {
        let s = snap(
            "2026-10-29T12:00:00Z",
            (99.0, Some("2026-10-29T12:00:30Z")),
            (10.0, Some("2026-11-02T00:00:00Z")),
        );
        let u = utilization_from_snapshot(Some(&s), "acct", at("2026-10-29T12:01:00Z")).unwrap();
        assert!(u.five_hour.is_none());
        assert!(u.seven_day.is_some());
    }

    #[test]
    fn a_window_without_a_reset_time_is_unknown() {
        let s = snap("2026-10-29T12:00:00Z", (99.0, None), (99.0, None));
        assert!(utilization_from_snapshot(Some(&s), "acct", at("2026-10-29T12:01:00Z")).is_none());
    }

    #[test]
    fn stale_missing_or_unlisted_is_unknown() {
        let s = snap(
            "2026-10-29T12:00:00Z",
            (1.0, Some("2026-10-29T15:00:00Z")),
            (1.0, Some("2026-11-02T00:00:00Z")),
        );
        assert!(utilization_from_snapshot(Some(&s), "acct", at("2026-10-29T12:05:01Z")).is_none());
        assert!(utilization_from_snapshot(Some(&s), "other", at("2026-10-29T12:01:00Z")).is_none());
        assert!(utilization_from_snapshot(None, "acct", at("2026-10-29T12:01:00Z")).is_none());
        // A snapshot from the future is a clock problem, not a reading.
        assert!(utilization_from_snapshot(Some(&s), "acct", at("2026-10-29T11:00:00Z")).is_none());
    }

    #[test]
    fn a_fresh_write_of_a_stale_or_failed_reading_is_unknown() {
        let mut s = snap(
            "2026-10-29T12:00:00Z",
            (1.0, Some("2026-10-29T15:00:00Z")),
            (1.0, Some("2026-11-02T00:00:00Z")),
        );
        let now = at("2026-10-29T12:01:00Z");
        let entry = s.accounts.get_mut("acct").unwrap();
        // Fetched eleven minutes ago with a five-minute TTL: past 2×TTL.
        entry.fetched_at = at("2026-10-29T11:50:00Z");
        assert!(utilization_from_snapshot(Some(&s), "acct", now).is_none());
        // Within 2×TTL it is a reading.
        s.accounts.get_mut("acct").unwrap().fetched_at = at("2026-10-29T11:52:00Z");
        assert!(utilization_from_snapshot(Some(&s), "acct", now).is_some());
        // A rate-limited refresh keeps the old windows; they are not current.
        s.accounts.get_mut("acct").unwrap().status = AccountStatus::RateLimited;
        assert!(utilization_from_snapshot(Some(&s), "acct", now).is_none());
    }

    #[test]
    fn the_default_status_is_not_ready() {
        assert!(!UpstreamStatus::default().credential_ready);
    }
}
