// Config view: tree, preview, search, effective settings, MCP, editor candidates.
// Sharded from src/types.ts to keep each domain's DTOs in its own
// file; src/types/index.ts re-exports them. Mirrors src-tauri/src/dto.rs.

// ---------- Config section --------------------------------------------

/** Matches `claudepot_core::config_view::model::Kind` serde string form. */
export type ConfigKind =
  | "claude_md"
  | "settings"
  | "settings_local"
  | "managed_settings"
  | "redacted_user_config"
  | "mcp_json"
  | "managed_mcp_json"
  | "agent"
  | "skill"
  | "command"
  | "output_style"
  | "workflow"
  | "rule"
  | "hook"
  | "memory"
  | "memory_index"
  | "plugin"
  | "keybindings"
  | "statusline"
  | "effective_settings"
  | "effective_mcp"
  | "other";

export interface ConfigFileNodeDto {
  id: string;
  kind: string;
  abs_path: string;
  display_path: string;
  size_bytes: number;
  mtime_unix_ns: number;
  summary_title: string | null;
  summary_description: string | null;
  issues: string[];
  /**
   * Absolute path of the memory file that `@include`-pulled this one.
   * `null` for root files. Present only on memory-kind nodes reached
   * through the include resolver.
   */
  included_by: string | null;
  /** Depth in the `@include` chain (0 = root, 1 = direct include). */
  include_depth: number;
}

export interface ConfigScopeNodeDto {
  id: string;
  label: string;
  scope_type: string;
  recursive_count: number;
  files: ConfigFileNodeDto[];
}

/**
 * User-selected anchor for the Config page.
 *
 * `global` — no project selected. Backend runs in global-only mode:
 *   skips Project / Local / MCP-walk / CLAUDE.md-walk / Memory-current
 *   scopes. Effective Settings / MCP are not available.
 * `folder` — specific cwd; the backend walks that project normally.
 *
 * Persisted in localStorage under `claudepot.config.anchor`.
 */
export type ConfigAnchor =
  | { kind: "global" }
  | { kind: "folder"; path: string };

export interface ConfigTreeDto {
  scopes: ConfigScopeNodeDto[];
  cwd: string;
  project_root: string;
  /**
   * Platform-correct path to `<cwd>/.claude`, joined on the backend via
   * `Path::join` so Windows tree rows render `C:\...\.claude` instead
   * of the mixed-separator result of a JS template string. Display-
   * only — never fed back to the backend for path lookups.
   */
  config_home_dir: string;
  memory_slug: string;
  memory_slug_lossy: boolean;
}

export interface ConfigPreviewDto {
  file: ConfigFileNodeDto;
  body_utf8: string;
  truncated: boolean;
}

export interface ConfigSearchHitDto {
  search_id: string;
  node_id: string;
  line_number: number;
  snippet: string;
  match_count_in_file: number;
}

export interface ConfigSearchSummaryDto {
  search_id: string;
  total_hits: number;
  capped: boolean;
  skipped_large: number;
  cancelled: boolean;
}

export interface ConfigProvenanceLeafDto {
  path: string;
  winner: string;
  contributors: string[];
  suppressed: boolean;
}

export interface ConfigPolicyErrorDto {
  origin: string;
  message: string;
}

export interface ConfigEffectiveSettingsDto {
  merged: unknown;
  provenance: ConfigProvenanceLeafDto[];
  policy_winner: string | null;
  policy_errors: ConfigPolicyErrorDto[];
}

export type McpSimulationMode =
  | "interactive"
  | "non_interactive"
  | "skip_permissions";

export interface ConfigEffectiveMcpServerDto {
  name: string;
  source_scope: string;
  contributors: string[];
  approval: "approved" | "rejected" | "pending" | "auto_approved";
  approval_reason: string | null;
  blocked_by: string | null;
  masked: unknown;
}

export interface ConfigEffectiveMcpDto {
  enterprise_lockout: boolean;
  servers: ConfigEffectiveMcpServerDto[];
}

export interface EditorCandidateDto {
  id: string;
  label: string;
  binary_path: string | null;
  bundle_id: string | null;
  launch_kind: "direct" | "macos-open-a" | "env-editor" | "system-handler";
  detected_via:
    | "path-binary"
    | "macos-app"
    | "windows-registry"
    | "linux-desktop-file"
    | "env-var"
    | "system-default"
    | "user-picked";
  supports_kinds: ConfigKind[] | null;
}

export interface EditorDefaultsDto {
  by_kind: Record<string, string>;
  fallback: string;
}
