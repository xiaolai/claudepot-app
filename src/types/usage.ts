// Local cost report — wire types mirroring
// `src-tauri/src/commands_usage_local::LocalUsageReportDto`. Field
// names are byte-for-byte the same as the Rust DTO's serialized
// output (snake_case, matching the project's existing DTO
// convention).

export interface ReportWindow {
  from_ms: number | null;
  to_ms: number | null;
}

export interface ProjectUsageRow {
  project_path: string;
  session_count: number;
  first_active_ms: number | null;
  last_active_ms: number | null;
  tokens_input: number;
  tokens_output: number;
  tokens_cache_creation: number;
  tokens_cache_read: number;
  /**
   * Dollar cost across all sessions in this row, computed against
   * the bundled price table. `null` when *every* contributing
   * session lacked a model the price table could resolve. One
   * unmatched session does NOT zero out the total — UI compares
   * `tokens_*` against `cost_usd` to detect partial matches.
   */
  cost_usd: number | null;
  /** Sessions whose models couldn't be priced. Drives the row's
   *  warning glyph + the footer note. */
  unpriced_sessions: number;
  /** Session-count breakdown by model id. A session that mixed
   *  Opus + Sonnet contributes 1 to each bucket, so the sum of
   *  values is ≥ `session_count`. Sessions with no recorded models
   *  contribute nothing. Used by the GUI to render the model-mix
   *  badge column on each project row. */
  models_by_session: Record<string, number>;
}

export interface UsageTotals {
  session_count: number;
  first_active_ms: number | null;
  last_active_ms: number | null;
  tokens_input: number;
  tokens_output: number;
  tokens_cache_creation: number;
  tokens_cache_read: number;
  cost_usd: number | null;
  unpriced_sessions: number;
  /** Install-wide model-mix; mirrors `ProjectUsageRow.models_by_session`. */
  models_by_session: Record<string, number>;
}

/** Wire form of `claudepot_core::pricing::PriceTier`. Lowercase
 *  snake_case matches the Rust enum's `serde(rename_all)`. */
export type PriceTierId =
  | "anthropic_api"
  | "vertex_global"
  | "vertex_regional"
  | "aws_bedrock";

export interface LocalUsageReport {
  window: ReportWindow;
  rows: ProjectUsageRow[];
  totals: UsageTotals;
  /** Short human-readable summary — "bundled · verified
   *  2026-01-15", "live · 2h ago", etc. — for a small pill near
   *  the window selector. */
  pricing_source: string;
  /** Non-null when the most recent pricing-refresh attempt failed.
   *  GUI surfaces it as a tooltip on the pricing-source pill. */
  pricing_error: string | null;
  /** Wire-form pricing tier the cost figures were computed against.
   *  Drives the active option in the tier picker and the platform
   *  label rendered alongside the source pill. */
  pricing_tier: PriceTierId;
}

/**
 * Wire shape for the time window argument to `localUsageAggregate`.
 *
 * Two variants:
 *   - `{ kind: "all" }` → open-ended on both sides.
 *   - `{ kind: "lastDays", days: 7 }` → last N days, anchored at
 *     "now" (server side). `days = 0` is interpreted as "all time"
 *     defensively.
 *
 * The shape is sent across IPC as `{ kind, days? }`. The TS-side
 * `lastDays` discriminator gets translated to the Rust-side
 * `last_days` literal in the api binding so both sides stay in
 * their idiomatic naming.
 */
export type UsageWindowSpec =
  | { kind: "all" }
  | { kind: "lastDays"; days: number };
