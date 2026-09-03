// Per-project permission management — frontend bindings for the
// `permission_*` Tauri commands. See `src-tauri/src/commands/permission.rs`
// for the Rust side and `dev-docs/permission-and-env-secrets.md` for
// the design.
//
// A grant no longer writes `bypassPermissions` into the project's
// settings file — Claude Code ignores that from project scope since
// 2.1.257 — it installs Claudepot's `PreToolUse` hook, which answers
// `allow` for tool calls inside the granted project until the grant
// lapses. See `claudepot_core::permission::hook`.

import { invoke } from "@tauri-apps/api/core";
import { i18n } from "../lib/i18n";

/** CC's `permissions.defaultMode` wire values. Unknown strings (a
 *  future CC mode) pass through verbatim.
 *
 *  `manual` is CC v2.1.200+'s alias for `default` — same mode, and
 *  the spelling CC's own UI now uses. Both are preserved verbatim so
 *  a revert never rewrites one spelling into the other. */
export type PermissionModeId =
  | "default"
  | "manual"
  | "acceptEdits"
  | "plan"
  | "auto"
  | "dontAsk"
  | "bypassPermissions"
  | (string & {});

export type PermissionDecisionSource =
  | "local_project_settings"
  | "project_settings"
  | "user_settings"
  | "default"
  /** A project-scope file holds `bypassPermissions` / `auto`, which CC
   *  ignores from that scope; the session starts in its built-in
   *  default. See `ProjectPermission.ignoredValue`. */
  | "project_scope_ignored";

/** A live Claudepot grant — time-boxed or sticky. */
export interface PermissionGrant {
  grantedAtMs: number;
  /** When the grant lapses. `null` means the grant is **sticky** —
   *  never auto-expired; the user removes it explicitly. */
  expiresAtMs: number | null;
}

/** A `defaultMode` value on disk that CC will not honour because of
 *  the file it is in. */
export interface IgnoredPermissionValue {
  /** `local_project` (Claudepot may remove it) or `project` (the
   *  repository's file; edit by hand). */
  layer: "local_project" | "project" | (string & {});
  mode: PermissionModeId;
}

/** One project row in the permission dashboard. */
export interface ProjectPermission {
  /** Canonical project root — the row identity. */
  projectPath: string;
  /** `permissions.defaultMode` CC will actually use. */
  effectiveMode: PermissionModeId;
  decidedBy: PermissionDecisionSource;
  /** True only for `bypassPermissions`, which since CC 2.1.257 only
   *  user or managed settings can produce. */
  isElevated: boolean;
  /** A stale project-scope value CC ignores, or `null`. */
  ignoredValue: IgnoredPermissionValue | null;
  /** The active grant, or `null`. */
  activeGrant: PermissionGrant | null;
  /** Whether Claude Code's `PreToolUse` hook entry is present and
   *  points at this binary. `false` with an active grant means the
   *  grant answers nothing — the pane must say so. */
  hookInstalled: boolean;
  /** First CC release that ignores `bypassPermissions` from project
   *  files, quoted in the pane rather than hardcoded. */
  projectScopeIgnoresSince: string;
}

/** Event payload for `permission-reverted` (a grant lapsed). */
export interface PermissionRevertedEvent {
  projectPath: string;
  outcome: "expired";
}

export const permissionApi = {
  permissionList: () => invoke<ProjectPermission[]>("permission_list"),
  permissionGet: (projectPath: string) =>
    invoke<ProjectPermission>("permission_get", { projectPath }),
  /**
   * Create a grant. Pass `durationSecs: null` for a sticky grant
   * (no auto-expiry); pass a positive number for a time-boxed grant.
   */
  permissionGrant: (projectPath: string, durationSecs: number | null) =>
    invoke<ProjectPermission>("permission_grant", {
      projectPath,
      durationSecs,
    }),
  permissionRevert: (projectPath: string) =>
    invoke<ProjectPermission>("permission_revert", { projectPath }),
  /**
   * Update an existing grant's deadline. Pass `durationSecs: null`
   * to convert a time-boxed grant to sticky.
   */
  permissionExtend: (projectPath: string, durationSecs: number | null) =>
    invoke<ProjectPermission>("permission_extend", {
      projectPath,
      durationSecs,
    }),
  /** Remove an ignored `defaultMode` from `.claude/settings.local.json`. */
  permissionClearIgnored: (projectPath: string) =>
    invoke<ProjectPermission>("permission_clear_ignored", { projectPath }),
};

/** Human label for a permission mode. Unknown modes render verbatim.
 *
 *  `default` and `manual` are one mode with two on-disk spellings, so
 *  they share a label. It reads "Manual" rather than "Default" to match
 *  what CC v2.1.200+ shows in its own UI — a control center that named
 *  the same state differently would look like it disagreed with CC.
 *
 *  Every entry is a getter: the catalog is read when the label is
 *  looked up, so a language switch reaches a row already on screen.
 *  A plain table evaluated at module load would pin the boot language.
 *  Indexing an unlisted mode still yields `undefined`, which is what
 *  `permissionModeLabel`'s `?? mode` fallback depends on. */
export const PERMISSION_MODE_LABEL: Record<string, string> = {
  get default() { return i18n.t("permission.mode.default"); },
  get manual() { return i18n.t("permission.mode.manual"); },
  get acceptEdits() { return i18n.t("permission.mode.acceptEdits"); },
  get plan() { return i18n.t("permission.mode.plan"); },
  get auto() { return i18n.t("permission.mode.auto"); },
  get dontAsk() { return i18n.t("permission.mode.dontAsk"); },
  get bypassPermissions() {
    return i18n.t("permission.mode.bypassPermissions");
  },
};

export function permissionModeLabel(mode: PermissionModeId): string {
  return PERMISSION_MODE_LABEL[mode] ?? mode;
}

/** Grant-duration presets the ProjectDetail control offers.
 *  `secs: null` is the "Never" / sticky option — the grant never
 *  lapses on its own; the user removes it via the same Revert action.
 *
 *  `label` is a getter (see `PERMISSION_MODE_LABEL`) so the option list
 *  re-reads the catalog on every render rather than freezing the boot
 *  language into a module-level constant. */
export const GRANT_DURATION_PRESETS: ReadonlyArray<{
  label: string;
  secs: number | null;
}> = [
  {
    get label() { return i18n.t("permission.duration.minutes30"); },
    secs: 30 * 60,
  },
  {
    get label() { return i18n.t("permission.duration.hours2"); },
    secs: 2 * 60 * 60,
  },
  {
    get label() { return i18n.t("permission.duration.hours8"); },
    secs: 8 * 60 * 60,
  },
  {
    get label() { return i18n.t("permission.duration.never"); },
    secs: null,
  },
];
