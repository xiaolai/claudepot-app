// Session prune / slim / trash / export DTOs + GitHub token status.
// Sharded from src/types.ts to keep each domain's DTOs in its own
// file; src/types/index.ts re-exports them. Mirrors src-tauri/src/dto.rs.


// ---------------------------------------------------------------------------
// Session prune / slim / trash
// ---------------------------------------------------------------------------

export interface PruneFilterInput {
  older_than_secs?: number | null;
  larger_than_bytes?: number | null;
  project: string[];
  has_error?: boolean | null;
  is_sidechain?: boolean | null;
}

export interface PruneEntry {
  session_id: string;
  file_path: string;
  project_path: string;
  size_bytes: number;
  last_ts_ms: number | null;
  has_error: boolean;
  is_sidechain: boolean;
}

export interface PrunePlan {
  entries: PruneEntry[];
  total_bytes: number;
}

export interface PruneReport {
  moved: string[];
  failed: [string, string][];
  freed_bytes: number;
}

export interface SlimOptsInput {
  drop_tool_results_over_bytes: number;
  exclude_tools: string[];
  /** Replace base64 image blocks with `[image]` text stubs. */
  strip_images?: boolean;
  /** Replace base64 document blocks with `[document]` text stubs. */
  strip_documents?: boolean;
}

export interface SlimPlan {
  original_bytes: number;
  projected_bytes: number;
  redact_count: number;
  image_redact_count: number;
  document_redact_count: number;
  tools_affected: string[];
  bytes_saved: number;
}

export interface SlimReport {
  original_bytes: number;
  final_bytes: number;
  redact_count: number;
  image_redact_count: number;
  document_redact_count: number;
  trashed_original: string;
  bytes_saved: number;
}

export interface BulkSlimEntry {
  session_id: string;
  file_path: string;
  project_path: string;
  plan: SlimPlan;
}

export interface BulkSlimPlan {
  entries: BulkSlimEntry[];
  /** Matched rows whose plan_slim() call errored. Surfaced so
   *  unreadable sessions don't silently disappear from the preview. */
  failed_to_plan: [string, string][];
  total_bytes_saved: number;
  total_image_redacts: number;
  total_document_redacts: number;
  total_tool_result_redacts: number;
}

export interface TrashEntry {
  id: string;
  kind: "prune" | "slim";
  orig_path: string;
  size: number;
  ts_ms: number;
  cwd: string | null;
  reason: string | null;
}

export interface TrashListing {
  entries: TrashEntry[];
  total_bytes: number;
}

export type ExportFormatInput =
  | { kind: "markdown" }
  | { kind: "markdown_slim" }
  | { kind: "json" }
  | { kind: "html"; no_js?: boolean };

export type PathStrategyInput =
  | { kind: "off" }
  | { kind: "relative"; root: string }
  | { kind: "hash" };

export interface RedactionPolicyInput {
  anthropic_keys?: boolean;
  paths?: PathStrategyInput;
  emails?: boolean;
  env_assignments?: boolean;
  custom_regex?: string[];
}

export interface GithubTokenStatus {
  present: boolean;
  last4: string | null;
  /**
   * When true, the GITHUB_TOKEN env var is set and overrides the
   * keychain slot for gist uploads. Surface this so users know why
   * Clear may not take effect for an upload.
   */
  env_override: boolean;
}
