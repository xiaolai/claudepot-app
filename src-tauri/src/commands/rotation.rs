//! Tauri commands for the auto-rotation Settings panel.
//!
//! All async per `commands/mod.rs` threading policy. The pure rule
//! logic lives in `claudepot_core::rotation`; this module only
//! marshalls DTOs and runs blocking I/O off the main thread.

use std::sync::Arc;

use chrono::Utc;
use claudepot_core::rotation::{
    eval::{evaluate, RuleDecision},
    rules::RotationRulesFile,
    store as rotation_store,
};
use claudepot_core::services::usage_snapshot;
use tauri::{AppHandle, State};

use crate::dto_rotation::{
    PendingSwapDto, RotationAuditEntryDto, RotationDryRunDto, RotationRuleDto,
    RotationRulesFileDto,
};
use crate::rotation_orchestrator::RotationOrchestrator;

/// Read rules from disk. Missing file → empty.
#[tauri::command]
pub async fn rotation_rules_get() -> Result<RotationRulesFileDto, String> {
    let file =
        tauri::async_runtime::spawn_blocking(rotation_store::load)
            .await
            .map_err(|e| format!("rotation_rules_get: join failed: {e}"))?;
    Ok(file.into())
}

/// Persist rules to disk. Validates before writing — invalid input is
/// rejected with a structured error.
#[tauri::command]
pub async fn rotation_rules_set(file: RotationRulesFileDto) -> Result<(), String> {
    let core: RotationRulesFile = RotationRulesFile::try_from(file)?;
    tauri::async_runtime::spawn_blocking(move || rotation_store::save(&core))
        .await
        .map_err(|e| format!("rotation_rules_set: join failed: {e}"))?
        .map_err(|e| e.to_string())
}

/// Validate a single rule without persisting. Used by the form to
/// surface errors inline as the user types.
#[tauri::command]
pub async fn rotation_rule_validate(rule: RotationRuleDto) -> Result<(), String> {
    let core = claudepot_core::rotation::rules::RotationRule::try_from(rule)?;
    core.validate().map_err(|e| e.to_string())
}

/// Dry-run a proposed rule against the current usage snapshot. v1
/// answers "would this fire RIGHT NOW?" — historical replay deferred.
#[tauri::command]
pub async fn rotation_dry_run(rule: RotationRuleDto) -> Result<RotationDryRunDto, String> {
    let core_rule = claudepot_core::rotation::rules::RotationRule::try_from(rule)?;
    core_rule.validate().map_err(|e| e.to_string())?;

    // Read the current snapshot off disk. We don't trigger a fresh
    // fetch — the dry-run is "what would happen on the next tick"
    // and the next tick will use whatever the snapshot writer
    // produces.
    let snapshot_path = usage_snapshot::snapshot_path();
    let snapshot_bytes = match std::fs::read(&snapshot_path) {
        Ok(b) => b,
        Err(e) => {
            return Ok(RotationDryRunDto {
                would_fire: false,
                target_email: None,
                reason: format!(
                    "no usage snapshot on disk yet ({e}); rule will be evaluated on the next 5-min tick"
                ),
            })
        }
    };
    let snapshot: usage_snapshot::UsageSnapshot = match serde_json::from_slice(&snapshot_bytes) {
        Ok(s) => s,
        Err(e) => return Err(format!("snapshot parse failed: {e}")),
    };

    // Resolve active CLI uuid from the snapshot itself.
    let active_uuid = snapshot
        .accounts
        .iter()
        .find(|(_, a)| a.cli_active)
        .and_then(|(k, _)| uuid::Uuid::parse_str(k).ok());
    let Some(active_uuid) = active_uuid else {
        return Ok(RotationDryRunDto {
            would_fire: false,
            target_email: None,
            reason: "no CLI-active account; rule cannot evaluate".into(),
        });
    };

    let decisions = evaluate(&[core_rule], &snapshot, active_uuid, &[], Utc::now());
    Ok(match decisions.into_iter().next() {
        Some(RuleDecision::Fire(p)) => RotationDryRunDto {
            would_fire: true,
            target_email: Some(p.to_email),
            reason: format!(
                "would fire — utilization {:.1}% on {} crosses {}%",
                p.trigger.utilization_pct,
                p.trigger
                    .window
                    .map(|w| w.label())
                    .unwrap_or("the trigger window"),
                p.trigger.threshold_pct
            ),
        },
        Some(RuleDecision::Skip { reason: None, .. }) => RotationDryRunDto {
            would_fire: false,
            target_email: None,
            reason: "would not fire — active account is below threshold".into(),
        },
        Some(RuleDecision::Skip {
            reason: Some(rec), ..
        }) => RotationDryRunDto {
            would_fire: false,
            target_email: rec.to_email,
            reason: format!("would not fire — {:?}", rec.reason),
        },
        None => RotationDryRunDto {
            would_fire: false,
            target_email: None,
            reason: "rule disabled".into(),
        },
    })
}

/// Newest-first audit log entries.
#[tauri::command]
pub async fn rotation_audit_get(
    orchestrator: State<'_, Arc<RotationOrchestrator>>,
    limit: Option<usize>,
) -> Result<Vec<RotationAuditEntryDto>, String> {
    let entries = orchestrator.list_audit(limit.unwrap_or(50));
    Ok(entries.into_iter().map(Into::into).collect())
}

/// Pending confirm-mode swaps. Renderer hydrates this on mount so
/// any toasts queued while the renderer was disconnected are
/// re-surfaced.
#[tauri::command]
pub async fn rotation_pending_list(
    orchestrator: State<'_, Arc<RotationOrchestrator>>,
) -> Result<Vec<PendingSwapDto>, String> {
    Ok(orchestrator
        .pending_list()
        .into_iter()
        .map(|q| PendingSwapDto {
            swap_id: q.swap_id,
            rule_id: q.pending.rule_id,
            from_email: q.pending.from_email,
            to_email: q.pending.to_email,
            queued_at: q.queued_at,
        })
        .collect())
}

/// User confirmed a suggested swap; perform it now. Returns Ok(()) on
/// success or a string error on swap failure (also logged to audit).
/// `Ok(())` with `swap_id` not present in the pending map is treated
/// as a no-op — the entry may have TTL'd or been already-applied by a
/// concurrent click.
#[tauri::command]
pub async fn rotation_apply_pending(
    app: AppHandle,
    orchestrator: State<'_, Arc<RotationOrchestrator>>,
    swap_id: String,
) -> Result<(), String> {
    let queued = match orchestrator.take_pending(&swap_id) {
        Some(q) => q,
        None => return Ok(()),
    };
    orchestrator.apply_confirmed(&app, queued).await
}

/// Drop a pending suggestion without acting on it.
#[tauri::command]
pub async fn rotation_dismiss_pending(
    orchestrator: State<'_, Arc<RotationOrchestrator>>,
    swap_id: String,
) -> Result<(), String> {
    orchestrator.take_pending(&swap_id);
    Ok(())
}
