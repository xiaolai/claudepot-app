//! Ring-buffer audit log of rotation activity.
//!
//! One entry per rotation attempt — applied, suggested, skipped (with
//! reason), failed. The 500-entry cap, atomic write-through,
//! mutex-poison recovery, and corrupt-file rename-aside all come from
//! the shared [`crate::json_store::CappedJsonLog`] engine (also used
//! by `notification_log`); this module keeps the entry schema and
//! the rotation-specific queries. Independent file
//! (`~/.claudepot/rotation-audit.json`) so notification spam doesn't
//! compete with rotation forensics.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::json_store::{CappedJsonLog, HasId, LogConfig};
use crate::services::usage_alerts::UsageWindowKind;

/// Hard ring-buffer cap. Mirrors `notification_log::MAX_ENTRIES`. At
/// ~300 B/entry (rule_id + emails + trigger summary), 500 entries is
/// ~150 KB — small enough to read into memory, big enough to cover
/// weeks of typical activity.
pub const MAX_AUDIT_ENTRIES: usize = 500;

/// Standard filename inside `claudepot_data_dir()`.
pub const AUDIT_FILENAME: &str = "rotation-audit.json";

/// `~/.claudepot/rotation-audit.json` (or `$CLAUDEPOT_DATA_DIR`'d).
pub fn audit_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(AUDIT_FILENAME)
}

/// What happened on a single rotation evaluation. Kept narrow on
/// purpose — every variant is a state the orchestrator can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationOutcome {
    /// Rule fired, swap was applied (auto mode or confirm-then-confirmed).
    Applied,
    /// Rule fired in confirm mode; awaiting user input. No swap yet.
    Suggested,
    /// Rule fired but a guard prevented the swap (min_interval,
    /// max_swaps). Reason in `skip_reason`.
    SkippedGuard,
    /// Rule fired but CC was running and `skip_when_cc_running` is
    /// set on the rule.
    SkippedCcRunning,
    /// Rule fired but no candidate could be selected (all alternates
    /// were also above the threshold, or none had usable creds).
    NoCandidate,
    /// Rule fired and the swap was attempted but failed. Error in
    /// `error`.
    Failed,
    /// Rule was *not* evaluated because its consecutive-failure
    /// circuit breaker is tripped — the swap kept failing, so the
    /// orchestrator quarantined the rule instead of retrying it every
    /// tick. Logged once on the trip transition, then the rule is
    /// silently skipped until the breaker's cooldown probe. See
    /// `claudepot_core::breaker`.
    Quarantined,
}

/// Trigger snapshot frozen at fire time so the audit log is self-
/// describing — even if the rule is later edited, the log reads the
/// way it happened.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RotationTriggerSummary {
    /// Window if the trigger was utilization-based. `None` for
    /// extra-usage triggers and any future shape that doesn't carry a
    /// window.
    pub window: Option<UsageWindowKind>,
    /// Server-reported utilization (0..=100) at fire time. The percent
    /// the user actually crossed, not the threshold.
    pub utilization_pct: f64,
    /// Threshold from the rule (1..=100).
    pub threshold_pct: u32,
    /// `true` when the trigger was `extra_usage_threshold` (monthly
    /// budget) rather than a window utilization.
    #[serde(default)]
    pub is_extra_usage: bool,
    /// `resets_at` of the window's current cycle at fire time. The
    /// guard math (`max_swaps_per_window`) compares against this so a
    /// swap from the previous cycle that happens to fall within the
    /// raw lookback length doesn't count toward the new cycle's cap.
    /// `None` for triggers without a cycle (extra-usage, or windows
    /// the server returned with `resets_at: null`).
    #[serde(default)]
    pub cycle_resets_at: Option<DateTime<chrono::FixedOffset>>,
    /// Background-worker count from `claude daemon status` at fire
    /// time. Frozen into the trigger so the audit entry remains
    /// self-describing even if the daemon state later changes. The
    /// React `AuditTable` renders this as a "5 bg workers active"
    /// chip when > 0 — surfaces the "5 detached agents have been
    /// chewing tokens" context that's otherwise invisible.
    /// `None` when the snapshot was written before bg-worker tracking
    /// shipped, or the scrape failed.
    #[serde(default)]
    pub bg_workers: Option<u32>,
}

/// One audit entry. Self-describing — the orchestrator may rotate
/// later under different rules, and the user may rename or delete
/// rules; the entry must still read on its own.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RotationAuditEntry {
    /// Monotonic per-process id. Reset to (max+1) on load. Powers
    /// stable React keys and the "since last seen" cursor.
    pub id: u64,
    pub ts: DateTime<Utc>,
    pub rule_id: String,
    pub trigger: RotationTriggerSummary,
    /// Email of the account that was active before the swap.
    pub from_email: String,
    /// Email of the rotation target — `None` when no candidate was
    /// found (`outcome = no_candidate`) or when the rule can't pick
    /// (degenerate selectors). The orchestrator fills this in when it
    /// resolves a candidate, even for skipped/suggested outcomes.
    #[serde(default)]
    pub to_email: Option<String>,
    pub mode: AuditMode,
    pub outcome: RotationOutcome,
    /// Free-form reason for skipped/failed/no_candidate outcomes.
    /// Empty when not applicable.
    #[serde(default)]
    pub reason: String,
}

/// Audit-log mirror of `RotationMode`. Separate type so we can
/// serialize without taking a coupling on the rules module's enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditMode {
    Confirm,
    Auto,
}

impl From<crate::rotation::rules::RotationMode> for AuditMode {
    fn from(m: crate::rotation::rules::RotationMode) -> Self {
        match m {
            crate::rotation::rules::RotationMode::Confirm => Self::Confirm,
            crate::rotation::rules::RotationMode::Auto => Self::Auto,
        }
    }
}

impl HasId for RotationAuditEntry {
    fn id(&self) -> u64 {
        self.id
    }
    fn set_id(&mut self, id: u64) {
        self.id = id;
    }
}

/// Engine config: the name lands in log messages and poison-recovery
/// warns; compact output matches the historical on-disk format.
const LOG_CFG: LogConfig = LogConfig {
    name: "rotation_audit",
    cap: MAX_AUDIT_ENTRIES,
    pretty: false,
};

/// Persistent ring-buffer log of rotation activity.
pub struct RotationAuditLog {
    log: CappedJsonLog<RotationAuditEntry>,
}

impl RotationAuditLog {
    /// In-memory log — appends and reads work but disk is never
    /// touched. Used when the on-disk path can't be opened, and by
    /// tests that don't want filesystem dependencies.
    pub fn in_memory_only() -> Self {
        Self {
            log: CappedJsonLog::in_memory_only(LOG_CFG),
        }
    }

    /// Open at `path`. Missing → empty. Corrupt → renamed aside
    /// (timestamped, see [`crate::json_store::move_aside`]), empty.
    pub fn open(path: PathBuf) -> std::io::Result<Self> {
        Ok(Self {
            log: CappedJsonLog::open(path, LOG_CFG)?,
        })
    }

    /// Open the canonical path. Falls back to `in_memory_only` if the
    /// file exists but can't be opened — never fatal at boot.
    pub fn open_default() -> Self {
        match Self::open(audit_path()) {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "rotation_audit: open failed; using volatile in-memory log"
                );
                Self::in_memory_only()
            }
        }
    }

    /// Append `entry`. Caller fills every field except `id` (assigned
    /// by the engine, monotonic per-process). Returns the assigned id.
    pub fn append(&self, entry: RotationAuditEntry) -> std::io::Result<u64> {
        self.log.append(entry)
    }

    /// Newest-first list of entries, capped at `limit`.
    pub fn list(&self, limit: usize) -> Vec<RotationAuditEntry> {
        self.log
            .with(|s| s.entries.iter().rev().take(limit).cloned().collect())
    }

    /// All entries (cloned). Used by the evaluator for guard math.
    pub fn snapshot(&self) -> Vec<RotationAuditEntry> {
        self.log.with(|s| s.entries.iter().cloned().collect())
    }

    /// Number of entries currently held.
    pub fn len(&self) -> usize {
        self.log.len()
    }

    pub fn is_empty(&self) -> bool {
        self.log.is_empty()
    }

    /// Drop every entry. Used by the Settings → Rotation panel for an
    /// explicit "clear log" action; not exposed automatically.
    pub fn clear(&self) -> std::io::Result<()> {
        self.log.with_mut(|s| {
            s.entries.clear();
            ((), true)
        })
    }
}

/// Convenience builder used by callers that only have a few fields and
/// don't want to construct the full struct each time.
pub fn entry_for(
    rule_id: impl Into<String>,
    trigger: RotationTriggerSummary,
    from_email: impl Into<String>,
    to_email: Option<String>,
    mode: AuditMode,
    outcome: RotationOutcome,
    reason: impl Into<String>,
) -> RotationAuditEntry {
    RotationAuditEntry {
        id: 0,
        ts: Utc::now(),
        rule_id: rule_id.into(),
        trigger,
        from_email: from_email.into(),
        to_email,
        mode,
        outcome,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(rule: &str, outcome: RotationOutcome) -> RotationAuditEntry {
        entry_for(
            rule,
            RotationTriggerSummary {
                window: Some(UsageWindowKind::FiveHour),
                utilization_pct: 91.0,
                threshold_pct: 90,
                is_extra_usage: false,
                cycle_resets_at: None,
                bg_workers: None,
            },
            "a@x.com",
            Some("b@x.com".into()),
            AuditMode::Confirm,
            outcome,
            "",
        )
    }

    #[test]
    fn append_assigns_monotonic_ids() {
        let log = RotationAuditLog::in_memory_only();
        let id1 = log.append(entry("r1", RotationOutcome::Suggested)).unwrap();
        let id2 = log.append(entry("r1", RotationOutcome::Applied)).unwrap();
        assert!(id2 > id1);
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn ring_caps_at_max_entries() {
        let log = RotationAuditLog::in_memory_only();
        // Append 502; the first two should fall off.
        for i in 0..(MAX_AUDIT_ENTRIES + 2) {
            let mut e = entry("r1", RotationOutcome::Applied);
            e.reason = format!("entry-{i}");
            log.append(e).unwrap();
        }
        assert_eq!(log.len(), MAX_AUDIT_ENTRIES);
        // Oldest preserved entry is the third we appended.
        let snap = log.snapshot();
        assert_eq!(snap.first().unwrap().reason, "entry-2");
    }

    #[test]
    fn list_returns_newest_first() {
        let log = RotationAuditLog::in_memory_only();
        let mut a = entry("r1", RotationOutcome::Applied);
        a.reason = "first".into();
        log.append(a).unwrap();
        let mut b = entry("r1", RotationOutcome::Applied);
        b.reason = "second".into();
        log.append(b).unwrap();
        let lst = log.list(10);
        assert_eq!(lst[0].reason, "second");
        assert_eq!(lst[1].reason, "first");
    }

    #[test]
    fn persist_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("audit.json");
        let log = RotationAuditLog::open(p.clone()).unwrap();
        log.append(entry("r1", RotationOutcome::Applied)).unwrap();
        log.append(entry("r1", RotationOutcome::Suggested)).unwrap();
        drop(log);

        let log2 = RotationAuditLog::open(p).unwrap();
        assert_eq!(log2.len(), 2);
        // Next id should be max+1, not collide with persisted ids.
        let new_id = log2.append(entry("r2", RotationOutcome::Applied)).unwrap();
        assert!(new_id > 2);
    }

    #[test]
    fn corrupt_file_starts_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("audit.json");
        std::fs::write(&p, b"not json").unwrap();
        let log = RotationAuditLog::open(p.clone()).unwrap();
        assert!(log.is_empty());
        assert_eq!(
            crate::json_store::corrupt_siblings(&p).len(),
            1,
            "corrupt file should be moved aside (timestamped)"
        );
    }

    #[test]
    fn clear_empties_log() {
        let log = RotationAuditLog::in_memory_only();
        log.append(entry("r1", RotationOutcome::Applied)).unwrap();
        log.append(entry("r1", RotationOutcome::Applied)).unwrap();
        assert_eq!(log.len(), 2);
        log.clear().unwrap();
        assert!(log.is_empty());
    }

    #[test]
    fn audit_mode_from_rotation_mode() {
        assert_eq!(
            AuditMode::from(crate::rotation::rules::RotationMode::Auto),
            AuditMode::Auto
        );
        assert_eq!(
            AuditMode::from(crate::rotation::rules::RotationMode::Confirm),
            AuditMode::Confirm
        );
    }
}
