// Memory-pane bindings for the Projects → Memory surface and the
// Settings → Auto-memory toggle. Shape mirrors `dto_memory.rs`.

import { invoke } from "@tauri-apps/api/core";

export type MemoryFileRole =
  | "claude_md_project"
  | "claude_md_project_local"
  | "auto_memory_index"
  | "auto_memory_topic"
  | "kairos_log"
  | "claude_md_global";

export type MemoryChangeType = "created" | "modified" | "deleted";

export type DiffOmitReason = "too_large" | "binary" | "endpoint" | "baseline";

export type AutoMemoryDecisionSource =
  | "env_disable"
  | "env_simple"
  | "local_project_settings"
  | "project_settings"
  | "user_settings"
  | "default";

export interface ProjectMemoryAnchor {
  project_root: string;
  auto_memory_anchor: string;
  slug: string;
  auto_memory_dir: string;
}

export interface MemoryFileSummary {
  abs_path: string;
  role: MemoryFileRole;
  scope_label: string;
  size_bytes: number;
  mtime_unix_ns: number;
  line_count: number;
  lines_past_cutoff: number | null;
  last_change_unix_ns: number | null;
  change_count_30d: number;
}

export interface MemoryEnumerate {
  anchor: ProjectMemoryAnchor;
  files: MemoryFileSummary[];
}

export interface MemoryChange {
  id: number;
  project_slug: string | null;
  abs_path: string;
  role: MemoryFileRole;
  change_type: MemoryChangeType;
  detected_at_ns: number;
  mtime_ns: number;
  size_before: number | null;
  size_after: number | null;
  hash_before: string | null;
  hash_after: string | null;
  diff_text: string | null;
  diff_omitted: boolean;
  diff_omit_reason: DiffOmitReason | null;
}

export interface AutoMemoryStateDto {
  project_root: string;
  effective: boolean;
  decided_by: AutoMemoryDecisionSource;
  decided_label: string;
  user_writable: boolean;
  user_settings_value: boolean | null;
  project_settings_value: boolean | null;
  local_project_settings_value: boolean | null;
  env_disable_set: boolean;
  env_simple_set: boolean;
  local_settings_gitignored: boolean | null;
}

export type AutoMemoryScope = "user" | "local_project";

export const memoryApi = {
  memoryListForProject: (projectRoot: string) =>
    invoke<MemoryEnumerate>("memory_list_for_project", {
      projectRoot,
    }),

  memoryReadFile: (projectRoot: string, absPath: string) =>
    invoke<string>("memory_read_file", {
      projectRoot,
      absPath,
    }),

  memoryChangeLog: (
    projectRoot: string,
    filePath?: string,
    limit?: number,
  ) =>
    invoke<MemoryChange[]>("memory_change_log", {
      projectRoot,
      filePath: filePath ?? null,
      limit: limit ?? null,
    }),

  autoMemoryState: (projectRoot: string) =>
    invoke<AutoMemoryStateDto>("auto_memory_state", {
      projectRoot,
    }),

  /**
   * Read env + `~/.claude/settings.json` only — no project layers.
   * Use this for the Settings → General global toggle so the row
   * doesn't conflate user-settings and project-settings (audit #3).
   */
  autoMemoryStateGlobal: () =>
    invoke<AutoMemoryStateDto>("auto_memory_state_global"),

  autoMemorySet: (
    projectRoot: string,
    scope: AutoMemoryScope,
    value: boolean | null,
  ) =>
    invoke<AutoMemoryStateDto>("auto_memory_set", {
      projectRoot,
      scope,
      value,
    }),
};
