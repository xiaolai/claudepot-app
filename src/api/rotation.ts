// Auto-rotation — frontend bindings for the `rotation_*` Tauri
// commands. See `src-tauri/src/commands/rotation.rs` for the Rust
// side and `dev-docs/auto-rotation.md` for the design.

import { invoke } from "@tauri-apps/api/core";

export type WindowId =
  | "five_hour"
  | "seven_day"
  | "seven_day_opus"
  | "seven_day_sonnet";

export const WINDOW_LABELS: Record<WindowId, string> = {
  five_hour: "5-hour window",
  seven_day: "7-day window",
  seven_day_opus: "7-day Opus window",
  seven_day_sonnet: "7-day Sonnet window",
};

export type SelectorKind = "least_used" | "round_robin" | "explicit";
export type RotationModeId = "confirm" | "auto";

export interface RotationGuards {
  minIntervalSecs: number;
  maxSwapsPerWindow: number;
  skipWhenCcRunning: boolean;
}

export interface RotationTrigger {
  /** Always `"utilization_threshold"` in v1. */
  kind: string;
  /** Required for utilization triggers. */
  window: WindowId | "";
  pct: number;
}

export interface RotationSelector {
  kind: SelectorKind;
  /** Used by `least_used`. */
  window: WindowId | "";
  /** Used by `least_used` and `round_robin`. */
  candidates: string[];
  /** Used by `explicit`. */
  email: string;
}

export interface RotationAction {
  /** Always `"rotate_to"` in v1. */
  kind: string;
  selector: RotationSelector;
}

export interface RotationRule {
  id: string;
  enabled: boolean;
  trigger: RotationTrigger;
  action: RotationAction;
  mode: RotationModeId;
  guards: RotationGuards;
}

export interface RotationRulesFile {
  schemaVersion: number;
  rules: RotationRule[];
}

export interface RotationDryRun {
  wouldFire: boolean;
  targetEmail: string | null;
  reason: string;
}

export interface RotationTriggerSummary {
  window: WindowId | null;
  utilizationPct: number;
  thresholdPct: number;
  isExtraUsage: boolean;
}

export type RotationOutcomeId =
  | "applied"
  | "suggested"
  | "skipped_guard"
  | "skipped_cc_running"
  | "no_candidate"
  | "failed"
  | "quarantined";

export interface RotationAuditEntry {
  id: number;
  ts: string;
  ruleId: string;
  trigger: RotationTriggerSummary;
  fromEmail: string;
  toEmail: string | null;
  mode: RotationModeId;
  outcome: RotationOutcomeId;
  reason: string;
}

export interface PendingSwap {
  swapId: string;
  ruleId: string;
  fromEmail: string;
  toEmail: string;
  queuedAt: string;
  trigger: RotationTriggerSummary;
}

export const rotationApi = {
  rotationRulesGet: () => invoke<RotationRulesFile>("rotation_rules_get"),
  rotationRulesSet: (file: RotationRulesFile) =>
    invoke<void>("rotation_rules_set", { file }),
  rotationRuleValidate: (rule: RotationRule) =>
    invoke<void>("rotation_rule_validate", { rule }),
  rotationDryRun: (rule: RotationRule) =>
    invoke<RotationDryRun>("rotation_dry_run", { rule }),
  rotationAuditGet: (limit?: number) =>
    invoke<RotationAuditEntry[]>("rotation_audit_get", { limit }),
  rotationPendingList: () => invoke<PendingSwap[]>("rotation_pending_list"),
  rotationApplyPending: (swapId: string) =>
    invoke<void>("rotation_apply_pending", { swapId }),
  rotationDismissPending: (swapId: string) =>
    invoke<void>("rotation_dismiss_pending", { swapId }),
};

/** Build a fresh rule with sensible defaults. Used by the form. */
export function newRule(id: string, candidates: string[]): RotationRule {
  return {
    id,
    enabled: true,
    trigger: {
      kind: "utilization_threshold",
      window: "five_hour",
      pct: 90,
    },
    action: {
      kind: "rotate_to",
      selector: {
        kind: "least_used",
        window: "five_hour",
        candidates,
        email: "",
      },
    },
    mode: "confirm",
    guards: {
      minIntervalSecs: 60,
      maxSwapsPerWindow: 3,
      skipWhenCcRunning: false,
    },
  };
}

export const ROTATION_OUTCOME_LABEL: Record<RotationOutcomeId, string> = {
  applied: "Applied",
  suggested: "Suggested",
  skipped_guard: "Skipped (guard)",
  skipped_cc_running: "Skipped (CC running)",
  no_candidate: "No candidate",
  failed: "Failed",
  quarantined: "Quarantined",
};
