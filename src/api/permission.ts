// Per-project permission management — frontend bindings for the
// `permission_*` Tauri commands. See `src-tauri/src/commands/permission.rs`
// for the Rust side and `dev-docs/permission-and-env-secrets.md` for
// the design.

import { invoke } from "@tauri-apps/api/core";

/** CC's `permissions.defaultMode` wire values. Unknown strings (e.g.
 *  a feature-flagged `auto`) pass through verbatim. */
export type PermissionModeId =
  | "default"
  | "acceptEdits"
  | "plan"
  | "dontAsk"
  | "bypassPermissions"
  | (string & {});

export type PermissionDecisionSource =
  | "local_project_settings"
  | "project_settings"
  | "user_settings"
  | "default";

/** A live, time-boxed Claudepot grant. */
export interface PermissionGrant {
  /** The mode Claudepot set (almost always `bypassPermissions`). */
  grantedMode: PermissionModeId;
  /** What the layer held before the grant; `null` means the key was
   *  absent and revert will clear it. */
  previousMode: PermissionModeId | null;
  grantedAtMs: number;
  expiresAtMs: number;
}

/** One project row in the permission dashboard. */
export interface ProjectPermission {
  /** Canonical project root — the row identity. */
  projectPath: string;
  /** `permissions.defaultMode` CC will actually use. */
  effectiveMode: PermissionModeId;
  decidedBy: PermissionDecisionSource;
  /** True only for `bypassPermissions`. */
  isElevated: boolean;
  /** The active grant, or `null` for an un-elevated project — or one
   *  the user elevated by hand-editing settings (elevated, but not
   *  Claudepot-managed). */
  activeGrant: PermissionGrant | null;
}

/** Event payload for `permission-reverted` (auto-revert on expiry). */
export interface PermissionRevertedEvent {
  projectPath: string;
  revertedTo: PermissionModeId;
  outcome: "reverted" | "skipped_user_changed";
}

/**
 * Event payload for `permission-breaker-tripped` — a grant's
 * auto-revert failed enough consecutive times that the orchestrator's
 * circuit breaker quarantined it. The grant is left in place,
 * un-reverted, until the breaker's cooldown lets a probe retry
 * through.
 */
export interface PermissionBreakerTrippedEvent {
  projectPath: string;
  consecutiveFailures: number;
}

export const permissionApi = {
  permissionList: () => invoke<ProjectPermission[]>("permission_list"),
  permissionGet: (projectPath: string) =>
    invoke<ProjectPermission>("permission_get", { projectPath }),
  permissionGrant: (
    projectPath: string,
    mode: PermissionModeId,
    durationSecs: number,
  ) =>
    invoke<ProjectPermission>("permission_grant", {
      projectPath,
      mode,
      durationSecs,
    }),
  permissionRevert: (projectPath: string) =>
    invoke<ProjectPermission>("permission_revert", { projectPath }),
  permissionExtend: (projectPath: string, durationSecs: number) =>
    invoke<ProjectPermission>("permission_extend", {
      projectPath,
      durationSecs,
    }),
};

/** Human label for a permission mode. Unknown modes render verbatim. */
export const PERMISSION_MODE_LABEL: Record<string, string> = {
  default: "Default",
  acceptEdits: "Accept edits",
  plan: "Plan",
  dontAsk: "Don't ask",
  bypassPermissions: "Bypass permissions",
};

export function permissionModeLabel(mode: PermissionModeId): string {
  return PERMISSION_MODE_LABEL[mode] ?? mode;
}

/** Grant-duration presets the ProjectDetail control offers. */
export const GRANT_DURATION_PRESETS: ReadonlyArray<{
  label: string;
  secs: number;
}> = [
  { label: "30 minutes", secs: 30 * 60 },
  { label: "2 hours", secs: 2 * 60 * 60 },
  { label: "8 hours", secs: 8 * 60 * 60 },
];
