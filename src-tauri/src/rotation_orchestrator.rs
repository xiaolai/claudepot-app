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

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use claudepot_core::breaker;
use claudepot_core::cli_backend;
use claudepot_core::rotation::{
    audit::{entry_for, AuditMode, RotationAuditEntry, RotationAuditLog, RotationOutcome},
    breaker_store::{self, BreakerFile},
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

// `breaker_gated_rules` (the pure breaker pre-filter) lives in
// `claudepot_core::rotation::gating` with its tests — the rotation
// pattern: policy in core, wiring here.
use claudepot_core::rotation::breaker_gated_rules;

#[derive(Default)]
struct Inner {
    pending: HashMap<String, QueuedSwap>,
    /// Swap ids currently inside `apply_confirmed`'s multi-second
    /// swap window (token refresh + profile fetch + keychain
    /// writes). `begin_apply` compare-and-sets an id in here so a
    /// second concurrent apply of the same suggestion (toast press
    /// racing a reload-hydrated toast press) is a no-op instead of
    /// two interleaved `switch_force` runs. Cleared by
    /// `finish_apply` on both outcomes.
    in_flight: HashSet<String>,
    /// Per-rule deferral state for `skip_when_cc_running`. Keyed by
    /// rule_id; the value is the time we first observed CC running
    /// for this rule. Once set, subsequent ticks see the entry and
    /// suppress the audit-log spam — they don't re-log
    /// `SkippedCcRunning` every 5 minutes for the same wait. At the
    /// end of every tick the map is retained down to the rules that
    /// actually deferred *this* tick (see `retain_cc_deferred`), which
    /// both prunes deleted rules and clears the flag for a rule that
    /// stopped deferring — so a later deferral logs a fresh entry
    /// instead of being suppressed by a stale flag.
    cc_deferred: HashMap<String, chrono::DateTime<Utc>>,
    /// Rule ids currently in the "no safe target" stalled state
    /// (every alternate candidate is also above threshold). Emitting
    /// `rotation-stalled` on *entry* into the set (not every tick)
    /// keeps the notification to one per stall episode. Retained down
    /// to the rules still stalled each tick, so a rule that recovers
    /// and later re-stalls notifies again.
    stalled_notified: HashSet<String>,
}

/// What `dispatch_auto` did with one auto-mode `Fire` this tick. The
/// tick loop reads this to enforce "at most one applied swap per tick"
/// and to track which rules deferred for `cc_deferred` upkeep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FireOutcome {
    /// The swap was applied — the active account changed, so every
    /// later decision this tick is stale and must be superseded.
    Swapped,
    /// The swap was deferred because CC is running and the rule set
    /// `skip_when_cc_running`.
    Deferred,
    /// The swap was attempted but failed (active unchanged).
    Failed,
}

/// Orchestrator state — `manage()`'d by the Tauri app, reachable via
/// `app.state::<RotationOrchestrator>()`.
pub struct RotationOrchestrator {
    inner: Mutex<Inner>,
    audit: Arc<RotationAuditLog>,
    /// Serializes breaker-file read-modify-write sections. `tick`
    /// (background) and `apply_confirmed` (a Tauri command) both
    /// record breaker outcomes and can run concurrently; without
    /// this lock their separate load→modify→save cycles could
    /// interleave and lose a ledger update (last-writer-wins).
    breaker_io: Mutex<()>,
}

impl RotationOrchestrator {
    pub fn new(audit: Arc<RotationAuditLog>) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            audit,
            breaker_io: Mutex::new(()),
        }
    }

    /// Acquire the breaker-file critical-section lock. Every
    /// breaker-file read-modify-write must hold this so a
    /// confirm-mode apply cannot interleave with the background
    /// tick and clobber a ledger update. Poison-tolerant — the
    /// guarded unit carries no data, so a panic mid-section cannot
    /// corrupt anything. All breaker file ops are synchronous, so
    /// the guard never crosses an `await`.
    fn breaker_guard(&self) -> std::sync::MutexGuard<'_, ()> {
        match self.breaker_io.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    /// Drive one rotation evaluation cycle. Called from
    /// `usage_snapshot::run_tick` after the snapshot has been written
    /// to disk. Loading the rules file on every tick is cheap (small
    /// file, infrequent edits) — saves wiring a file watcher.
    pub async fn tick(&self, app: &AppHandle, snapshot: &UsageSnapshot, active_uuid: Uuid) {
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
            // No enabled rule can defer or stall, so the correct state
            // is empty maps. Clear them before returning — otherwise a
            // lingering `cc_deferred` entry would suppress a fresh
            // deferral audit (and a lingering `stalled_notified` entry a
            // fresh stalled toast) if the rule is re-enabled later. This
            // is the same stale-suppression the end-of-tick retain
            // prevents on the normal path; the early return must not
            // skip it. (Breaker ledgers are deliberately left intact so
            // a rule re-enabled while still tripped stays gated.)
            self.retain_cc_deferred(&HashSet::new());
            self.retain_stalled(&HashSet::new());
            return;
        }

        // Evict stale pending entries before evaluation so the
        // confirm-mode dedupe below is accurate.
        self.evict_stale();

        let now = Utc::now();

        // Consecutive-failure circuit breaker: load the per-rule
        // failure ledgers and filter out any rule whose breaker is
        // tripped *before* the evaluator runs — a quarantined rule
        // has no pending entry to skip, so it must be excluded up
        // front. A real I/O failure on load skips the breaker gate
        // (fail open: a missing breaker file must not silence
        // rotation), but corruption recovers to empty. The breaker
        // *logic* is pure `claudepot_core::breaker`; this is wiring.
        let breaker_file = breaker_store::load_or_default();
        let live_rule_ids: HashSet<String> = rules.rules.iter().map(|r| r.id.clone()).collect();
        let active_rules = breaker_gated_rules(&rules.rules, &breaker_file, now);

        let audit_snapshot = self.audit.snapshot();
        let decisions = evaluate(&active_rules, snapshot, active_uuid, &audit_snapshot, now);

        // At most ONE swap is applied per tick. Once a swap lands, the
        // active account (and the usage the remaining decisions were
        // computed against) is stale, so later `Fire`s are superseded
        // and left for the next tick's fresh evaluation. This is what
        // makes `min_interval_secs`' "no cascade from a single tick"
        // guarantee real: within one `evaluate` call every rule sees
        // the same pre-tick audit, so the interval guard alone cannot
        // stop two rules from both firing here.
        let mut swapped_this_tick = false;
        // Rule ids that deferred (CC running) / stalled this tick, used
        // to retain the matching orchestrator state below.
        let mut deferred_ids: HashSet<String> = HashSet::new();
        let mut stalled_ids: HashSet<String> = HashSet::new();
        // Confirm-mode fires are stashed, not emitted inline: a confirm
        // suggestion emitted *before* an auto swap lands later this tick
        // would be stale (its `from_uuid` is the pre-swap active). We
        // emit them in a second pass, and only if no swap happened.
        let mut confirm_fires: Vec<PendingSwap> = Vec::new();

        for decision in decisions {
            match decision {
                RuleDecision::Skip { reason: None, .. } => {}
                RuleDecision::Skip {
                    rule_id,
                    reason: Some(rec),
                } => {
                    self.log_skip(&rule_id, &rec);
                    if matches!(
                        rec.reason,
                        SkipReason::NoCandidate(NoCandidateReason::AllAboveThreshold)
                    ) {
                        stalled_ids.insert(rule_id.clone());
                        self.note_stalled(app, &rule_id, &rec);
                    }
                }
                // Confirm fires are stashed for the second pass so their
                // freshness can be judged against the whole tick.
                RuleDecision::Fire(pending) if matches!(pending.mode, AuditMode::Confirm) => {
                    confirm_fires.push(pending);
                }
                // Auto fires apply immediately, at most one per tick.
                RuleDecision::Fire(pending) => {
                    if swapped_this_tick {
                        self.log_superseded(&pending);
                        continue;
                    }
                    let rule_id = pending.rule_id.clone();
                    match self.dispatch_auto(app, pending).await {
                        FireOutcome::Swapped => swapped_this_tick = true,
                        FireOutcome::Deferred => {
                            deferred_ids.insert(rule_id);
                        }
                        FireOutcome::Failed => {}
                    }
                }
            }
        }

        // Second pass: a swap this tick makes every confirm suggestion
        // stale (computed against the pre-swap active account), so
        // supersede them and let the next tick re-suggest from the new
        // active. With no swap, the active is unchanged and the
        // suggestions are valid — queue + emit them.
        for pending in confirm_fires {
            if swapped_this_tick {
                self.log_superseded(&pending);
            } else {
                self.queue_confirm(app, pending);
            }
        }

        // Retain the per-tick state to only the rules that produced it
        // this tick — prunes deleted rules and clears stale flags.
        self.retain_cc_deferred(&deferred_ids);
        self.retain_stalled(&stalled_ids);

        // Prune breaker ledgers for rules the user has since deleted
        // so `rotation-breaker.json` doesn't accumulate stale state
        // for rule ids that no longer exist. Cheap, runs last so it
        // doesn't race the failure/success recording above.
        self.prune_breaker_ledgers(&live_rule_ids);
    }

    /// Drop breaker ledgers whose `rule_id` is no longer in the rules
    /// file. Best-effort — a load/save failure is logged and
    /// swallowed; a stale ledger is harmless (the rule is gone, so it
    /// is never evaluated).
    fn prune_breaker_ledgers(&self, live_rule_ids: &HashSet<String>) {
        let _io = self.breaker_guard();
        let mut file = match breaker_store::load() {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(error = %e, "rotation_breaker: prune load failed");
                return;
            }
        };
        let before = file.ledgers.len();
        file.retain_rules(live_rule_ids);
        if file.ledgers.len() != before {
            if let Err(e) = breaker_store::save(&file) {
                tracing::warn!(error = %e, "rotation_breaker: prune save failed");
            }
        }
    }

    /// Record the outcome of a swap attempt against `rule_id`'s
    /// circuit breaker. `success = true` resets the ledger; `false`
    /// advances it. Returns `true` iff this failure was the one that
    /// newly tripped the breaker (crossed the threshold) — the
    /// caller fires the `rotation-breaker-tripped` event exactly once
    /// on that transition. The breaker arithmetic is pure
    /// `claudepot_core::breaker`; this only loads → mutates → saves
    /// the persisted ledger.
    fn record_breaker_outcome(&self, rule_id: &str, success: bool) -> bool {
        let _io = self.breaker_guard();
        let mut file: BreakerFile = match breaker_store::load() {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(error = %e, rule_id, "rotation_breaker: outcome load failed");
                return false;
            }
        };
        let ledger = file.ledger_for(rule_id);
        let (next, newly_tripped) = if success {
            (breaker::record_success(&ledger), false)
        } else {
            let newly_tripped = breaker::trips_on_next_failure(&ledger);
            (breaker::record_failure(&ledger, Utc::now()), newly_tripped)
        };
        file.set_ledger(rule_id, next);
        if let Err(e) = breaker_store::save(&file) {
            tracing::warn!(error = %e, rule_id, "rotation_breaker: outcome save failed");
        }
        newly_tripped
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

    async fn dispatch_auto(&self, app: &AppHandle, pending: PendingSwap) -> FireOutcome {
        // Honor `skip_when_cc_running` — the orchestrator only checks
        // this for auto mode; in confirm mode the user gets the toast
        // and decides whether to wait. To avoid re-logging
        // SkippedCcRunning every tick for the same wait, we record
        // the first deferral and only audit-log the transition. The
        // tick's end-of-loop `retain_cc_deferred` clears the flag once
        // the rule stops deferring, so the next episode logs afresh.
        if pending.skip_when_cc_running && cli_backend::swap::is_cc_process_running_public().await {
            let was_already_deferred = {
                let mut g = match self.inner.lock() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                if g.cc_deferred.contains_key(&pending.rule_id) {
                    true
                } else {
                    // Record the first-observed time; the value is only
                    // read for presence, but keep it truthful.
                    g.cc_deferred.insert(pending.rule_id.clone(), Utc::now());
                    false
                }
            };
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
            return FireOutcome::Deferred;
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
                // A successful swap clears any prior failure run.
                self.record_breaker_outcome(&pending.rule_id, true);
                let cc_running = cli_backend::swap::is_cc_process_running_public().await;
                emit_applied(app, &pending, cc_running);
                FireOutcome::Swapped
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
                // Advance the circuit breaker; quarantine + surface
                // when this failure crosses the threshold.
                if self.record_breaker_outcome(&pending.rule_id, false) {
                    self.note_breaker_tripped(app, &pending);
                }
                FireOutcome::Failed
            }
        }
    }

    /// Log a `Fire` that was superseded by an earlier swap this tick.
    /// Reuses the `SkippedGuard` outcome — a tick-level guard (one swap
    /// per tick) prevented it — with a reason the audit reader can act
    /// on. The rule is re-evaluated fresh on the next tick.
    fn log_superseded(&self, pending: &PendingSwap) {
        let entry = entry_for(
            &pending.rule_id,
            pending.trigger.clone(),
            pending.from_email.clone(),
            Some(pending.to_email.clone()),
            pending.mode,
            RotationOutcome::SkippedGuard,
            "another rule already swapped this tick; deferred to the next evaluation",
        );
        let _ = self.audit.append(entry);
    }

    /// Emit `rotation-stalled` the first time a rule enters the "no
    /// safe target" state. `note_stalled` is idempotent within a stall
    /// episode: `HashSet::insert` returns `true` only on the
    /// transition, so the event (and its toast) fires once, not every
    /// tick the stall persists.
    fn note_stalled(&self, app: &AppHandle, rule_id: &str, rec: &SkipReasonRecord) {
        let newly = {
            let mut g = match self.inner.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            g.stalled_notified.insert(rule_id.to_string())
        };
        if newly {
            emit_stalled(app, rule_id, rec);
        }
    }

    /// Retain `cc_deferred` down to the rules that deferred this tick.
    fn retain_cc_deferred(&self, deferred_ids: &HashSet<String>) {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.cc_deferred.retain(|id, _| deferred_ids.contains(id));
    }

    /// Retain `stalled_notified` down to the rules still stalled this
    /// tick, so a rule that recovers can notify again if it re-stalls.
    fn retain_stalled(&self, stalled_ids: &HashSet<String>) {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.stalled_notified.retain(|id| stalled_ids.contains(id));
    }

    /// Log the `Quarantined` audit entry and emit the
    /// `rotation-breaker-tripped` event. Called once, on the failure
    /// that trips the breaker — subsequent failed ticks for the same
    /// rule are silently skipped at the `tick` gate.
    fn note_breaker_tripped(&self, app: &AppHandle, pending: &PendingSwap) {
        let entry = entry_for(
            &pending.rule_id,
            pending.trigger.clone(),
            pending.from_email.clone(),
            Some(pending.to_email.clone()),
            pending.mode,
            RotationOutcome::Quarantined,
            format!(
                "paused after {} consecutive failures; will probe again after the cooldown",
                breaker::THRESHOLD
            ),
        );
        let _ = self.audit.append(entry);
        emit_breaker_tripped(app, pending);
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
        let Inner {
            pending, in_flight, ..
        } = &mut *g;
        pending.retain(|_, q| (now - q.queued_at).num_seconds() < PENDING_TTL_SECS);
        // An in-flight marker whose pending entry is gone (TTL'd
        // mid-apply, or the apply future was dropped) must not block
        // a future suggestion forever.
        in_flight.retain(|id| pending.contains_key(id));
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

    /// Claim a pending swap for application: returns the entry and
    /// marks it in-flight, in one critical section. A second call
    /// with the same id while the first apply is still running
    /// returns `None` — the peek-then-apply shape this replaces let
    /// two concurrent confirms both observe the entry and run
    /// `switch_force` twice, interleaving keychain writes. The entry is
    /// removed by [`Self::finish_apply`] on both outcomes; a failed
    /// apply is retried via the next tick's fresh re-suggestion, not by
    /// re-claiming this swap_id.
    pub fn begin_apply(&self, swap_id: &str) -> Option<QueuedSwap> {
        self.evict_stale();
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if g.in_flight.contains(swap_id) {
            return None;
        }
        let queued = g.pending.get(swap_id).cloned()?;
        g.in_flight.insert(swap_id.to_string());
        Some(queued)
    }

    /// Release a [`Self::begin_apply`] claim and consume the pending
    /// entry, regardless of swap outcome. A *failed* confirm apply is
    /// re-surfaced by the next tick's fresh suggestion (a new toast
    /// with a new swap_id), NOT by a lingering stash entry: keeping the
    /// entry on failure created a dead-zone — the toast was already
    /// gone (the "Switch" press dismisses it) and `queue_confirm`'s
    /// dedupe on `(rule_id, from, to)` suppressed the next tick's
    /// re-suggestion, so there was no retry affordance for up to the
    /// 30-minute TTL. Consuming the entry lets the rule re-suggest on
    /// the next tick until it succeeds or the breaker trips.
    fn finish_apply(&self, swap_id: &str) {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        g.in_flight.remove(swap_id);
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
    /// the apply-pending command after the user confirms. `queued`
    /// must come from [`Self::begin_apply`] — the in-flight claim is
    /// what makes a concurrent second confirm a no-op.
    ///
    /// Both outcomes consume the pending entry and release the
    /// in-flight claim. On failure the rule re-suggests on the next
    /// tick (a fresh toast) until it succeeds or its breaker trips —
    /// see [`Self::finish_apply`] for why keeping the entry on failure
    /// was a dead-zone rather than a retry.
    pub async fn apply_confirmed(&self, app: &AppHandle, queued: QueuedSwap) -> Result<(), String> {
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
                // A successful swap clears any prior failure run.
                self.record_breaker_outcome(&queued.pending.rule_id, true);
                let cc_running = cli_backend::swap::is_cc_process_running_public().await;
                emit_applied(app, &queued.pending, cc_running);
                self.finish_apply(&queued.swap_id);
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
                // Advance the circuit breaker; quarantine + surface
                // when this failure crosses the threshold. A tripped
                // rule is filtered out of the next `tick`'s
                // evaluation, so it stops re-suggesting.
                if self.record_breaker_outcome(&queued.pending.rule_id, false) {
                    self.note_breaker_tripped(app, &queued.pending);
                }
                // Consume the entry — the next tick re-suggests a fresh
                // toast; the breaker bounds runaway failures.
                self.finish_apply(&queued.swap_id);
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
        NoCandidateReason::TargetNotReady => {
            "the chosen target account is not swap-ready (unverified or erroring)"
        }
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
    /// Whether a CC process was running when the swap landed. A swap
    /// applied mid-session only takes effect after CC restarts (CC
    /// holds the old creds in memory), so the toast tells the user to
    /// restart. `false` → no restart hint (the swap is already live).
    pub cc_running: bool,
}

/// `rotation-stalled` payload — a rule matched but every alternate
/// candidate is also at or above the threshold, so there is no safe
/// target. Emitted once per stall episode.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationStalledPayload {
    pub rule_id: String,
    pub from_email: String,
    pub window: Option<String>,
    pub utilization_pct: f64,
    pub threshold_pct: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationFailedPayload {
    pub rule_id: String,
    pub from_email: String,
    pub to_email: String,
    pub error: String,
}

/// `rotation-breaker-tripped` payload — a rule's swap kept failing,
/// so its circuit breaker quarantined it. Emitted once, on the
/// failure that crosses the threshold.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationBreakerTrippedPayload {
    pub rule_id: String,
    pub from_email: String,
    pub to_email: String,
    /// Number of consecutive failures that tripped the breaker.
    pub consecutive_failures: u32,
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
    if let Err(e) = app.emit(crate::events::ROTATION_SUGGESTED, payload) {
        tracing::warn!(error = %e, "rotation_orchestrator: emit suggested failed");
    }
}

fn emit_applied(app: &AppHandle, p: &PendingSwap, cc_running: bool) {
    let payload = RotationAppliedPayload {
        rule_id: p.rule_id.clone(),
        from_email: p.from_email.clone(),
        to_email: p.to_email.clone(),
        cc_running,
    };
    if let Err(e) = app.emit(crate::events::ROTATION_APPLIED, payload) {
        tracing::warn!(error = %e, "rotation_orchestrator: emit applied failed");
    }
}

fn emit_stalled(app: &AppHandle, rule_id: &str, rec: &SkipReasonRecord) {
    let payload = RotationStalledPayload {
        rule_id: rule_id.to_string(),
        from_email: rec.from_email.clone(),
        window: rec.trigger.window.map(window_kind_str),
        utilization_pct: rec.trigger.utilization_pct,
        threshold_pct: rec.trigger.threshold_pct,
    };
    if let Err(e) = app.emit(crate::events::ROTATION_STALLED, payload) {
        tracing::warn!(error = %e, "rotation_orchestrator: emit stalled failed");
    }
}

fn emit_failed(app: &AppHandle, p: &PendingSwap, error: &str) {
    let payload = RotationFailedPayload {
        rule_id: p.rule_id.clone(),
        from_email: p.from_email.clone(),
        to_email: p.to_email.clone(),
        error: error.to_string(),
    };
    if let Err(e) = app.emit(crate::events::ROTATION_FAILED, payload) {
        tracing::warn!(error = %e, "rotation_orchestrator: emit failed failed");
    }
}

fn emit_breaker_tripped(app: &AppHandle, p: &PendingSwap) {
    let payload = RotationBreakerTrippedPayload {
        rule_id: p.rule_id.clone(),
        from_email: p.from_email.clone(),
        to_email: p.to_email.clone(),
        consecutive_failures: breaker::THRESHOLD,
    };
    if let Err(e) = app.emit(crate::events::ROTATION_BREAKER_TRIPPED, payload) {
        tracing::warn!(error = %e, "rotation_orchestrator: emit breaker-tripped failed");
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

// The breaker-gate tests moved to
// `claudepot_core::rotation::gating::tests` with the function.

#[cfg(test)]
mod tests {
    use super::*;
    use claudepot_core::rotation::RotationTriggerSummary;

    fn orchestrator_with_pending(swap_id: &str) -> RotationOrchestrator {
        let orch = RotationOrchestrator::new(Arc::new(RotationAuditLog::in_memory_only()));
        let queued = QueuedSwap {
            swap_id: swap_id.to_string(),
            pending: PendingSwap {
                rule_id: "r1".into(),
                from_uuid: uuid::Uuid::nil(),
                from_email: "a@example.com".into(),
                to_uuid: uuid::Uuid::nil(),
                to_email: "b@example.com".into(),
                trigger: RotationTriggerSummary {
                    window: None,
                    utilization_pct: 90.0,
                    threshold_pct: 80,
                    is_extra_usage: false,
                    cycle_resets_at: None,
                    bg_workers: None,
                },
                mode: AuditMode::Confirm,
                skip_when_cc_running: false,
            },
            queued_at: Utc::now(),
        };
        let mut g = orch.inner.lock().unwrap();
        g.pending.insert(swap_id.to_string(), queued);
        drop(g);
        orch
    }

    #[test]
    fn begin_apply_claims_once_and_consumes_on_finish() {
        // The double-apply race this closes: two toast presses both
        // peeked the same entry and both ran switch_force. The second
        // claim must be refused while the first is in flight.
        let orch = orchestrator_with_pending("s1");
        assert!(orch.begin_apply("s1").is_some());
        assert!(
            orch.begin_apply("s1").is_none(),
            "concurrent claim must be refused"
        );

        // `finish_apply` consumes the entry on both outcomes — a failed
        // confirm apply is retried via the next tick's fresh
        // suggestion, not by re-claiming this swap_id.
        orch.finish_apply("s1");
        assert!(
            orch.begin_apply("s1").is_none(),
            "consumed entry must be gone after finish"
        );
    }

    #[test]
    fn begin_apply_unknown_id_is_none() {
        let orch = orchestrator_with_pending("s1");
        assert!(orch.begin_apply("nope").is_none());
    }
}
