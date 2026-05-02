// Automation DTOs. Mirrors src-tauri/src/dto_automations.rs.

export type PermissionMode =
  | "default"
  | "acceptEdits"
  | "bypassPermissions"
  | "dontAsk"
  | "plan"
  | "auto";

export type OutputFormat = "text" | "json" | "stream-json";

export type AutomationBinaryKind = "first_party" | "route";

export type TriggerKind = "scheduled" | "manual";

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

export interface AutomationSummaryDto {
  id: string;
  name: string;
  display_name: string | null;
  description: string | null;
  enabled: boolean;
  binary_kind: AutomationBinaryKind;
  binary_route_id: string | null;
  model: string | null;
  cwd: string;
  permission_mode: string;
  allowed_tools: string[];
  max_budget_usd: number | null;
  trigger_kind: string;
  cron: string | null;
  timezone: string | null;
  created_at: string;
  updated_at: string;
}

export interface AutomationDetailsDto {
  summary: AutomationSummaryDto;
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
}

export interface AutomationCreateDto {
  name: string;
  display_name: string | null;
  description: string | null;
  binary_kind: AutomationBinaryKind;
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
}

/**
 * Patch shape: omit a field (or send `null`) to leave it unchanged;
 * send a value to overwrite. There is no way to explicitly clear an
 * optional field to null via this DTO — to clear, use the appropriate
 * empty-shape value (e.g. empty string for cleared display name).
 * Mirrors the Rust `Option<T>` patch fields in `dto_automations.rs`.
 */
export interface AutomationUpdateDto {
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

export interface AutomationRunDto {
  id: string;
  automation_id: string;
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
