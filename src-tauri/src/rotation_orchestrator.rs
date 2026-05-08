//! Rotation orchestrator — bridges `claudepot-core::rotation::eval`
//! to the Tauri runtime.
//!
//! Owns:
//! - The persistent audit log (`Arc<RotationAuditLog>`).
//! - A pending-swap stash for `confirm` mode — keyed by `swap_id`,
//!   the front-end's confirm callback resolves a pending entry by id.
//!
//! Used from two sites:
//! - `usage_snapshot::run_tick` calls [`tick`] after writing the
//!   per-account snapshot.
//! - `commands::rotation` exposes the audit-list / apply-pending
//!   entry points to the renderer.
//!
//! The pure rule logic lives in core; this module only wires it to
//! Tauri (event emission, account-store I/O, the cli_backend swap).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use claudepot_core::cli_backend;
use claudepot_core::rotation::{
    audit::{
        entry_for, AuditMode, RotationAuditEntry, RotationAuditLog, RotationOutcome,
    },
    eval::{evaluate, NoCandidateReason, PendingSwap, RuleDecision, SkipReason, SkipReasonRecord},
    rules::RotationRulesFile,
    store as rotation_store,
};
use claudepot_core::services::usage_snapshot::UsageSnapshot;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

/// One queued confirm-mode swap awaiting user confirmation.
#[derive(Debug, Clone)]
pub struct QueuedSwap {
    pub swap_id: String,
    pub pending: PendingSwap,
    pub queued_at: chrono::DateTime<Utc>,
}

/// Pending swaps older than this are evicted on tick. The user has
/// almost certainly missed the toast; re-evaluating fresh is safer
/// than acting on a stale 30-minute-old fire.
const PENDING_TTL_SECS: i64 = 1800;

#[derive(Default)]
struct Inner {
    pending: HashMap<String, QueuedSwap>,
    /// Per-rule deferral state for `skip_when_cc_running`. Keyed by
    /// rule_id; the value is the time we first observed CC running
    /// for this rule. Once set, subsequent ticks see the entry and
    /// suppress the audit-log spam — they don't re-log
    /// `SkippedCcRunning` every 5 minutes for the same wait. The
    /// entry is cleared the first time the rule is dispatched OR
    /// after an idle CC tick where CC isn't running.
    cc_deferred: HashMap<String, chrono::DateTime<Utc>>,
}

/// Orchestrator state — `manage()`'d by the Tauri app, reachable via
/// `app.state::<RotationOrchestrator>()`.
pub struct RotationOrchestrator {
    inner: Mutex<Inner>,
    audit: Arc<RotationAuditLog>,
}

impl RotationOrchestrator {
    pub fn new(audit: Arc<RotationAuditLog>) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            audit,
        }
    }

    /// Drive one rotation evaluation cycle. Called from
    /// `usage_snapshot::run_tick` after the snapshot has been written
    /// to disk. Loading the rules file on every tick is cheap (small
    /// file, infrequent edits) — saves wiring a file watcher.
    pub async fn tick(
        &self,
        app: &AppHandle,
        snapshot: &UsageSnapshot,
        active_uuid: Uuid,
    ) {
        // Real I/O failures (permission denied, transient FS error)
        // skip this tick rather than silently treating the failure
        // as "no rules" — silent default would be invisible to the
        // user. NotFound + corruption are still recovered to empty.
        let rules: RotationRulesFile = match rotation_store::load() {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(error = %e, "rotation_orchestrator: rules load failed; skipping tick");
                return;
            }
        };
        if rules.rules.iter().all(|r| !r.enabled) {
            return;
        }

        // Evict stale pending entries before evaluation so the
        // confirm-mode dedupe below is accurate.
        self.evict_stale();

        let audit_snapshot = self.audit.snapshot();
        let now = Utc::now();
        let decisions = evaluate(&rules.rules, snapshot, active_uuid, &audit_snapshot, now);

        for decision in decisions {
            match decision {
                RuleDecision::Skip { reason: None, .. } => {}
                RuleDecision::Skip {
                    rule_id,
                    reason: Some(rec),
                } => {
                    self.log_skip(&rule_id, &rec);
                }
                RuleDecision::Fire(pending) => {
                    self.dispatch(app, pending).await;
                }
            }
        }
    }

    fn log_skip(&self, rule_id: &str, rec: &SkipReasonRecord) {
        let (outcome, reason_text) = skip_reason_to_outcome(&rec.reason);
        let entry = entry_for(
            rule_id,
            rec.trigger.clone(),
            rec.from_email.clone(),
            rec.to_email.clone(),
            // Carry the rule's actual mode so the audit log records
            // the truth — auto-mode skips look like auto-mode skips.
            rec.mode,
            outcome,
            reason_text,
        );
        if let Err(e) = self.audit.append(entry) {
            tracing::warn!(error = %e, rule_id, "rotation_audit: append failed");
        }
    }

    async fn dispatch(&self, app: &AppHandle, pending: PendingSwap) {
        match pending.mode {
            AuditMode::Auto => {
                self.dispatch_auto(app, pending).await;
            }
            AuditMode::Confirm => {
                self.queue_confirm(app, pending);
            }
        }
    }

    async fn dispatch_auto(&self, app: &AppHandle, pending: PendingSwap) {
        // Honor `skip_when_cc_running` — the orchestrator only checks
        // this for auto mode; in confirm mode the user gets the toast
        // and decides whether to wait. To avoid re-logging
        // SkippedCcRunning every tick for the same wait, we record
        // the first deferral and only audit-log the transition (and
        // the eventual resolve), not every check.
        if pending.skip_when_cc_running
            && cli_backend::swap::is_cc_process_running_public().await
        {
            let mut g = match self.inner.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            let was_already_deferred = g.cc_deferred.contains_key(&pending.rule_id);
            if !was_already_deferred {
                g.cc_deferred.insert(pending.rule_id.clone(), Utc::now());
            }
            drop(g);
            if !was_already_deferred {
                let entry = entry_for(
                    &pending.rule_id,
                    pending.trigger.clone(),
                    pending.from_email.clone(),
                    Some(pending.to_email.clone()),
                    AuditMode::Auto,
                    RotationOutcome::SkippedCcRunning,
                    "cli is currently running; waiting until idle",
                );
                let _ = self.audit.append(entry);
            }
            return;
        }
        // CC is not running (or the rule didn't ask to defer). Drop
        // any prior deferral entry for this rule so the next CC-busy
        // window will log a fresh "deferred" audit entry instead of
        // staying silent.
        {
            let mut g = match self.inner.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            g.cc_deferred.remove(&pending.rule_id);
        }

        match perform_swap(pending.from_uuid, pending.to_uuid).await {
            Ok(()) => {
                let entry = entry_for(
                    &pending.rule_id,
                    pending.trigger.clone(),
                    pending.from_email.clone(),
                    Some(pending.to_email.clone()),
                    AuditMode::Auto,
                    RotationOutcome::Applied,
                    "",
                );
                let _ = self.audit.append(entry);
                emit_applied(app, &pending);
            }
            Err(e) => {
                let entry = entry_for(
                    &pending.rule_id,
                    pending.trigger.clone(),
                    pending.from_email.clone(),
                    Some(pending.to_email.clone()),
                    AuditMode::Auto,
                    RotationOutcome::Failed,
                    e.clone(),
                );
                let _ = self.audit.append(entry);
                emit_failed(app, &pending, &e);
            }
        }
    }

    fn queue_confirm(&self, app: &AppHandle, pending: PendingSwap) {
        // Dedupe key: (rule_id, from_uuid, to_uuid). Including
        // from_uuid lets a second suggestion through when the active
        // account changed mid-toast — same rule_id + same target
        // from a different starting point is a legitimate distinct
        // event the user should see.
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if g.pending.values().any(|q| {
            q.pending.rule_id == pending.rule_id
                && q.pending.from_uuid == pending.from_uuid
                && q.pending.to_uuid == pending.to_uuid
        }) {
            return;
        }
        let swap_id = Uuid::new_v4().to_string();
        let queued = QueuedSwap {
            swap_id: swap_id.clone(),
            pending: pending.clone(),
            queued_at: Utc::now(),
        };
        g.pending.insert(swap_id.clone(), queued);
        drop(g);

        let entry = entry_for(
            &pending.rule_id,
            pending.trigger.clone(),
            pending.from_email.clone(),
            Some(pending.to_email.clone()),
            AuditMode::Confirm,
            RotationOutcome::Suggested,
            "",
        );
        let _ = self.audit.append(entry);
        emit_suggested(app, &swap_id, &pending);
    }

    fn evict_stale(&self) {
        let now = Utc::now();
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.pending
            .retain(|_, q| (now - q.queued_at).num_seconds() < PENDING_TTL_SECS);
    }

    /// Look up + remove a pending swap. Used by
    /// `commands::rotation::rotation_apply_pending`. Evicts stale
    /// entries first so a swap_id whose entry has TTL'd between
    /// the toast click and this call returns `None` rather than
    /// re-applying a stale 30-minute-old fire.
    pub fn take_pending(&self, swap_id: &str) -> Option<QueuedSwap> {
        self.evict_stale();
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.pending.remove(swap_id)
    }

    /// Like [`take_pending`] but only peeks — does not remove.
    /// Used by `apply_pending` so a transient swap failure leaves
    /// the entry available for retry.
    pub fn peek_pending(&self, swap_id: &str) -> Option<QueuedSwap> {
        self.evict_stale();
        let g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.pending.get(swap_id).cloned()
    }

    /// Remove a pending swap by id. Used after a confirmed swap
    /// succeeds (peek-then-apply-then-remove) and as the
    /// `dismiss-pending` command's effect. Evicting first keeps the
    /// stash bounded.
    pub fn remove_pending(&self, swap_id: &str) {
        self.evict_stale();
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.pending.remove(swap_id);
    }

    /// Snapshot of currently queued confirm-mode swaps. Used by the
    /// front-end on mount to re-hydrate any toasts that were
    /// suggested while the renderer was disconnected (e.g. between
    /// reloads). Evicts stale entries first so the renderer never
    /// hydrates a TTL-expired suggestion.
    pub fn pending_list(&self) -> Vec<QueuedSwap> {
        self.evict_stale();
        let g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.pending.values().cloned().collect()
    }

    /// Apply a swap by uuid pair, recording an audit entry. Used by
    /// the apply-pending command after the user confirms.
    ///
    /// On success: removes the pending entry. On failure: leaves the
    /// pending entry in the stash so the user can retry — a
    /// transient error (token refresh hiccup, identity-gate flap)
    /// shouldn't silently lose the suggestion.
    pub async fn apply_confirmed(
        &self,
        app: &AppHandle,
        queued: QueuedSwap,
    ) -> Result<(), String> {
        match perform_swap(queued.pending.from_uuid, queued.pending.to_uuid).await {
            Ok(()) => {
                let entry = entry_for(
                    &queued.pending.rule_id,
                    queued.pending.trigger.clone(),
                    queued.pending.from_email.clone(),
                    Some(queued.pending.to_email.clone()),
                    AuditMode::Confirm,
                    RotationOutcome::Applied,
                    "user confirmed",
                );
                let _ = self.audit.append(entry);
                emit_applied(app, &queued.pending);
                self.remove_pending(&queued.swap_id);
                Ok(())
            }
            Err(e) => {
                let entry = entry_for(
                    &queued.pending.rule_id,
                    queued.pending.trigger.clone(),
                    queued.pending.from_email.clone(),
                    Some(queued.pending.to_email.clone()),
                    AuditMode::Confirm,
                    RotationOutcome::Failed,
                    e.clone(),
                );
                let _ = self.audit.append(entry);
                emit_failed(app, &queued.pending, &e);
                // Do NOT remove the pending entry — let the user
                // retry. The 30-min TTL still bounds blast radius.
                Err(e)
            }
        }
    }

    /// Read-only rule list for the Settings panel.
    pub fn list_audit(&self, limit: usize) -> Vec<RotationAuditEntry> {
        self.audit.list(limit)
    }

    /// Full audit snapshot — used by `dry_run` so its evaluator
    /// honors the same guards a live tick would.
    pub fn audit_snapshot(&self) -> Vec<RotationAuditEntry> {
        self.audit.snapshot()
    }
}

fn skip_reason_to_outcome(r: &SkipReason) -> (RotationOutcome, String) {
    match r {
        SkipReason::NoActiveSnapshot => (
            RotationOutcome::SkippedGuard,
            "active account had no usage snapshot".into(),
        ),
        SkipReason::NoWindowData => (
            RotationOutcome::SkippedGuard,
            "trigger window had no data".into(),
        ),
        SkipReason::MinIntervalNotElapsed { secs_since_last } => (
            RotationOutcome::SkippedGuard,
            format!("min_interval_secs not elapsed (last swap {secs_since_last}s ago)"),
        ),
        SkipReason::MaxSwapsHit { swaps_in_cycle } => (
            RotationOutcome::SkippedGuard,
            format!("max_swaps_per_window reached ({swaps_in_cycle})"),
        ),
        SkipReason::NoCandidate(reason) => (
            RotationOutcome::NoCandidate,
            no_candidate_reason_text(reason).into(),
        ),
    }
}

fn no_candidate_reason_text(r: &NoCandidateReason) -> &'static str {
    match r {
        NoCandidateReason::OnlyActive => "only the active account matched the candidate list",
        NoCandidateReason::AllAboveThreshold => {
            "every alternate candidate was also at or above the threshold"
        }
        NoCandidateReason::UnknownEmails => "no candidate email matched a registered account",
        NoCandidateReason::ActiveNotInList => {
            "active account is not in the round-robin candidate list"
        }
    }
}

async fn perform_swap(from: Uuid, to: Uuid) -> Result<(), String> {
    let store = crate::commands::open_store()?;
    let platform = cli_backend::create_platform();
    let refresher = cli_backend::swap::DefaultRefresher;
    let fetcher = cli_backend::swap::DefaultProfileFetcher;
    cli_backend::swap::switch_force(
        &store,
        Some(from),
        to,
        platform.as_ref(),
        true,
        &refresher,
        &fetcher,
    )
    .await
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Frontend events
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationSuggestedPayload {
    pub swap_id: String,
    pub rule_id: String,
    pub from_email: String,
    pub to_email: String,
    pub from_uuid: String,
    pub to_uuid: String,
    pub window: Option<String>,
    pub utilization_pct: f64,
    pub threshold_pct: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationAppliedPayload {
    pub rule_id: String,
    pub from_email: String,
    pub to_email: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationFailedPayload {
    pub rule_id: String,
    pub from_email: String,
    pub to_email: String,
    pub error: String,
}

fn emit_suggested(app: &AppHandle, swap_id: &str, p: &PendingSwap) {
    let payload = RotationSuggestedPayload {
        swap_id: swap_id.to_string(),
        rule_id: p.rule_id.clone(),
        from_email: p.from_email.clone(),
        to_email: p.to_email.clone(),
        from_uuid: p.from_uuid.to_string(),
        to_uuid: p.to_uuid.to_string(),
        window: p.trigger.window.map(window_kind_str),
        utilization_pct: p.trigger.utilization_pct,
        threshold_pct: p.trigger.threshold_pct,
    };
    if let Err(e) = app.emit("rotation-suggested", payload) {
        tracing::warn!(error = %e, "rotation_orchestrator: emit suggested failed");
    }
}

fn emit_applied(app: &AppHandle, p: &PendingSwap) {
    let payload = RotationAppliedPayload {
        rule_id: p.rule_id.clone(),
        from_email: p.from_email.clone(),
        to_email: p.to_email.clone(),
    };
    if let Err(e) = app.emit("rotation-applied", payload) {
        tracing::warn!(error = %e, "rotation_orchestrator: emit applied failed");
    }
}

fn emit_failed(app: &AppHandle, p: &PendingSwap, error: &str) {
    let payload = RotationFailedPayload {
        rule_id: p.rule_id.clone(),
        from_email: p.from_email.clone(),
        to_email: p.to_email.clone(),
        error: error.to_string(),
    };
    if let Err(e) = app.emit("rotation-failed", payload) {
        tracing::warn!(error = %e, "rotation_orchestrator: emit failed failed");
    }
}

fn window_kind_str(k: claudepot_core::services::usage_alerts::UsageWindowKind) -> String {
    use claudepot_core::services::usage_alerts::UsageWindowKind as W;
    match k {
        W::FiveHour => "five_hour".into(),
        W::SevenDay => "seven_day".into(),
        W::SevenDayOpus => "seven_day_opus".into(),
        W::SevenDaySonnet => "seven_day_sonnet".into(),
    }
}

