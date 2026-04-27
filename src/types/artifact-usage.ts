// Artifact usage telemetry — counts and outcomes for invocations of
// installed CC artifacts (skills, hooks, agents, slash commands).
//
// Mirrors `src-tauri/src/dto_artifact_usage.rs`. Distinct from
// `pricing.ts` which covers account / rate-limit usage; the two
// share the word "usage" but nothing else.

/** Matches `claudepot_core::artifact_usage::ArtifactKind`. */
export type ArtifactUsageKind = "skill" | "hook" | "agent" | "command";

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
