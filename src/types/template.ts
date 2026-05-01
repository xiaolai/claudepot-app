// Template DTOs. Mirrors src-tauri/src/dto_templates.rs.
//
// The wire format uses kebab-case for category, tier, cost class,
// privacy, and recommended-class enums; snake_case for capability
// tags and schedule shapes (matching the Rust serde renames).

export type TemplateCategory =
  | "it-health"
  | "diagnostics"
  | "housekeeping"
  | "audit"
  | "caregiver"
  | "network";

export type TemplateTier =
  | "ambient"
  | "on-demand"
  | "triggered"
  | "periodic";

export type CostClass = "trivial" | "low" | "medium" | "high";

export type PrivacyClass = "local" | "private-cloud" | "any";

export type ModelClass = "local-ok" | "fast" | "frontier";

export type Capability =
  | "tool_use"
  | "long_context"
  | "vision"
  | "structured_output";

export type FallbackPolicy = "skip" | "use_default_route" | "alert";

export type ScheduleShapeName =
  | "daily"
  | "weekdays"
  | "weekly"
  | "hourly"
  | "manual"
  | "custom";

export type PlaceholderTypeName =
  | "path"
  | "text"
  | "boolean"
  | "number"
  | "list";

export interface PlaceholderDto {
  name: string;
  label: string;
  type: PlaceholderTypeName;
  required: boolean;
  default?: unknown;
  help?: string;
}

export interface TemplateScopeDto {
  reads: string;
  writes: string;
  could_change: string;
  network: string;
}

export interface TemplateSummaryDto {
  id: string;
  name: string;
  tagline: string;
  category: TemplateCategory;
  icon: string;
  tier: TemplateTier;
  cost_class: CostClass;
  privacy: PrivacyClass;
  recommended_class: ModelClass;
  consent_required: boolean;
  apply_supported: boolean;
  default_schedule_label: string;
}

export interface TemplateDetailsDto {
  summary: TemplateSummaryDto;
  schema_version: number;
  version: number;
  description: string;
  scope: TemplateScopeDto;
  capabilities_required: Capability[];
  min_context_tokens: number;
  fallback_policy: FallbackPolicy;
  default_schedule_cron: string;
  allowed_schedule_shapes: ScheduleShapeName[];
  output_path_template: string;
  output_format: string;
  placeholders: PlaceholderDto[];
  requires_full_disk_access: boolean;
}

// ----- Install request -----

export type ScheduleDto =
  | { kind: "daily"; time: string }
  | { kind: "weekdays"; time: string }
  | { kind: "weekly"; day: Weekday; time: string }
  | { kind: "hourly"; every_n_hours: number }
  | { kind: "manual" }
  | { kind: "custom"; cron: string };

export type Weekday =
  | "sun"
  | "mon"
  | "tue"
  | "wed"
  | "thu"
  | "fri"
  | "sat";

export interface TemplateInstanceDto {
  blueprint_id: string;
  blueprint_schema_version: number;
  // Wire shape: a Record of placeholder-name → JSON-typed value.
  // The Rust side decodes using the blueprint's placeholder schema.
  placeholder_values?: Record<string, unknown>;
  route_id?: string;
  schedule: ScheduleDto;
  name_override?: string;
}

// ----- Route summary surfaced by templates_capable_routes -----
//
// Distinct from src/types/route.ts's `RouteSummaryDto` because this
// shape carries template-specific compatibility flags
// (is_capable, ineligibility_reason).

export interface TemplateRouteSummaryDto {
  id: string;
  name: string;
  provider: string;
  model: string;
  is_local: boolean;
  is_private_cloud: boolean;
  is_capable: boolean;
  ineligibility_reason: string;
}
