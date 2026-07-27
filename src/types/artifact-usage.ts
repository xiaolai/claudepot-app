// Artifact usage telemetry — counts and outcomes for invocations of
// installed CC artifacts (skills, hooks, agents, slash commands).
//
// Mirrors `src-tauri/src/dto_artifact_usage.rs`. Distinct from
// `pricing.ts` which covers account / rate-limit usage; the two
// share the word "usage" but nothing else.

/**
 * Matches `claudepot_core::artifact_usage::ArtifactKind`.
 *
 * `mcp` keys are the FULL wire name (`mcp__<server>__<tool>`) — server
 * names can contain underscores and hyphens, so the pair is never
 * re-joined from parts.
 */
export type ArtifactUsageKind =
  | "skill"
  | "hook"
  | "agent"
  | "command"
  | "mcp";

export interface ArtifactUsageStatsDto {
  count_24h: number;
  count_7d: number;
  count_30d: number;
  error_count_30d: number;
  /** Wall-clock ms-since-epoch of the last recorded event. */
  last_seen_ms: number | null;
  /** p50 in ms over the 24h raw-event window. Only hooks have durations today. */
  p50_ms_24h: number | null;
  /** Average duration in ms over the 30d daily rollup. */
  avg_ms_30d: number | null;
}

export interface ArtifactUsageRowDto {
  kind: string; // ArtifactUsageKind, but the DTO uses raw string for forward-compat
  artifact_key: string;
  plugin_id: string | null;
  stats: ArtifactUsageStatsDto;
}

export interface ArtifactUsageBatchEntryDto {
  kind: string;
  artifact_key: string;
  stats: ArtifactUsageStatsDto;
}

/**
 * One row of the durable "ever observed" ledger (`artifact_first_last`,
 * schema v6). Backs the Usage tab's Unused view.
 *
 * Coverage boundary: this is "ever observed **by Claudepot**", not
 * "ever run". Invocations predating the ledger, or living only in
 * transcripts deleted before its backfill, are absent. Surface it as
 * "no invocation on record" — never "never used".
 */
export interface ArtifactEverFiredDto {
  kind: string;
  artifact_key: string;
  first_seen_ms: number;
  last_seen_ms: number;
}

/** One row of the Unused view. Mirrors `UnusedArtifactDto` in Rust. */
export interface UnusedArtifactDto {
  kind: string;
  /** Config-tree node id — deep-link via Global -> Config, subRoute "node:<id>". */
  node_id: string;
  artifact_key: string;
  plugin_id: string | null;
  label: string;
  abs_path: string;
  /** Filesystem **modification** time in ms — not install time. */
  modified_ms: number;
}

/**
 * Unused-view payload. Carries suppression counts so the pane's summary
 * reconciles instead of silently dropping rows, and `grace_days` so the
 * UI states the number core actually applied rather than keeping a
 * second copy of the constant.
 */
export interface UnusedReportDto {
  rows: UnusedArtifactDto[];
  installed_count: number;
  suppressed_recent: number;
  suppressed_disabled: number;
  grace_days: number;
}
