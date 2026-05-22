// Agent DTOs. Mirrors src-tauri/src/dto_agents.rs.

export type PermissionMode =
  | "default"
  | "acceptEdits"
  | "bypassPermissions"
  | "dontAsk"
  | "plan"
  | "auto";

export type OutputFormat = "text" | "json" | "stream-json";

export type AgentBinaryKind = "first_party" | "route";

export type TriggerKind = "scheduled" | "manual";

/** Agent lifecycle. A draft is inert; only the GUI arms an agent. */
export type Lifecycle = "draft" | "installed";

/**
 * Reference to an MCP server the agent attaches via `--mcp-config`.
 * `claudepot_memory` resolves to Claudepot's own memory server;
 * `custom` carries a name + an opaque MCP server config object.
 * Mirrors the Rust `McpServerRef` enum (`#[serde(tag = "kind")]`).
 */
export type McpServerRef =
  | { kind: "claudepot_memory" }
  | { kind: "custom"; name: string; config: unknown };

/** Claudepot-enforced run-frequency limit. Mirrors Rust `RateLimit`. */
export interface RateLimit {
  min_interval_secs: number | null;
  max_per_day: number | null;
}

export type ArtifactKind =
  | "report"
  | "pending_changes"
  | "apply_receipt"
  | "email";

export interface OutputArtifactDto {
  kind: ArtifactKind;
  path: string;
  format: string;
  bytes: number;
}

export type RouteDecisionDto =
  | { kind: "ran"; route_id: string | null }
  | {
      kind: "fallback";
      from: string;
      to: string | null;
      reason: string;
    }
  | { kind: "skipped"; reason: string }
  | { kind: "skipped_alerted"; reason: string };

export type HostPlatform = "macos" | "windows" | "linux" | "other";

export interface PlatformOptionsDto {
  wake_to_run: boolean;
  catch_up_if_missed: boolean;
  run_when_logged_out: boolean;
}

export interface AgentSummaryDto {
  id: string;
  name: string;
  display_name: string | null;
  description: string | null;
  enabled: boolean;
  binary_kind: AgentBinaryKind;
  binary_route_id: string | null;
  model: string | null;
  cwd: string;
  permission_mode: string;
  allowed_tools: string[];
  max_budget_usd: number | null;
  trigger_kind: string;
  cron: string | null;
  timezone: string | null;
  /** "draft" or "installed". Read-only — the GUI arms an agent. */
  lifecycle: string;
  created_at: string;
  updated_at: string;
}

export interface AgentDetailsDto {
  summary: AgentSummaryDto;
  prompt: string;
  system_prompt: string | null;
  append_system_prompt: string | null;
  add_dir: string[];
  fallback_model: string | null;
  output_format: string;
  json_schema: string | null;
  bare: boolean;
  extra_env: Record<string, string>;
  platform_options: PlatformOptionsDto;
  log_retention_runs: number;
  // ---- Agent-spec fields (Phase 1) ----
  disallowed_tools: string[];
  mcp_servers: McpServerRef[];
  run_as: string | null;
  task_budget: number | null;
  rate_limit: RateLimit | null;
  /** Audit field: who drafted this agent. Read-only and free-text. */
  drafted_by: string | null;
  /**
   * Immutable audit signal stamped by the code path that produced
   * this agent. `"gui"` = hand-authored via the GUI Add form;
   * `"cli_draft"` = AI-drafted via the `agent draft` CLI verb;
   * `"template"` = instantiated from a built-in template. Unlike
   * `drafted_by`, this is not caller-supplied and cannot be
   * spoofed — the install review flags every non-`"gui"` record.
   */
  created_via: string;
}

export interface AgentCreateDto {
  name: string;
  display_name: string | null;
  description: string | null;
  binary_kind: AgentBinaryKind;
  binary_route_id: string | null;
  model: string | null;
  cwd: string;
  prompt: string;
  system_prompt: string | null;
  append_system_prompt: string | null;
  permission_mode: PermissionMode;
  allowed_tools: string[];
  add_dir: string[];
  max_budget_usd: number | null;
  fallback_model: string | null;
  output_format: OutputFormat;
  json_schema: string | null;
  bare: boolean;
  extra_env: Record<string, string>;
  cron: string;
  timezone: string | null;
  platform_options: PlatformOptionsDto;
  log_retention_runs: number;
  // ---- Agent-spec fields (Phase 1) ----
  disallowed_tools: string[];
  mcp_servers: McpServerRef[];
  /** Account email; empty string = run as the active account. */
  run_as: string | null;
  /** Per-run token ceiling; 0 / null = no ceiling. */
  task_budget: number | null;
  rate_limit: RateLimit | null;
  /** Audit field; the regular Add Agent flow leaves it null. */
  drafted_by: string | null;
}

/**
 * Patch shape: omit a field (or send `null`) to leave it unchanged;
 * send a value to overwrite. There is no way to explicitly clear an
 * optional field to null via this DTO — to clear, use the appropriate
 * empty-shape value (e.g. empty string for cleared display name).
 * Mirrors the Rust `Option<T>` patch fields in `dto_agents.rs`.
 */
export interface AgentUpdateDto {
  id: string;
  display_name?: string | null;
  description?: string | null;
  enabled?: boolean;
  model?: string | null;
  cwd?: string;
  prompt?: string;
  system_prompt?: string | null;
  append_system_prompt?: string | null;
  permission_mode?: PermissionMode;
  allowed_tools?: string[];
  add_dir?: string[];
  max_budget_usd?: number | null;
  fallback_model?: string | null;
  output_format?: OutputFormat;
  json_schema?: string | null;
  bare?: boolean;
  extra_env?: Record<string, string>;
  cron?: string;
  timezone?: string | null;
  platform_options?: PlatformOptionsDto;
  log_retention_runs?: number;
  // ---- Agent-spec fields (Phase 1) ----
  disallowed_tools?: string[];
  mcp_servers?: McpServerRef[];
  /** Empty string clears `run_as`; a non-empty email pins it. */
  run_as?: string | null;
  /** 0 clears the budget; a positive value sets it. */
  task_budget?: number | null;
  /** A populated value sets the rate limit; all-null clears it. */
  rate_limit?: RateLimit | null;
}

export interface RunResultDto {
  subtype: string | null;
  is_error: boolean | null;
  num_turns: number | null;
  total_cost_usd: number | null;
  stop_reason: string | null;
  session_id: string | null;
  errors: string[];
}

export interface AgentRunDto {
  id: string;
  agent_id: string;
  started_at: string;
  ended_at: string;
  duration_ms: number;
  exit_code: number;
  result: RunResultDto | null;
  session_jsonl_path: string | null;
  stdout_log: string;
  stderr_log: string;
  trigger_kind: TriggerKind;
  host_platform: HostPlatform;
  claudepot_version: string;
  output_artifacts?: OutputArtifactDto[];
  route_decision?: RouteDecisionDto | null;
}

export interface SchedulerCapabilitiesDto {
  wake_to_run: boolean;
  catch_up_if_missed: boolean;
  run_when_logged_out: boolean;
  native_label: string;
  artifact_dir: string | null;
}

export interface CronValidationDto {
  valid: boolean;
  error: string | null;
  next_runs: string[];
}

export interface NameValidationDto {
  valid: boolean;
  error: string | null;
  already_taken: boolean;
}
