// Artifact usage telemetry — Tauri bridge for the four
// `artifact_usage_*` commands.
//
// Distinct from `pricing.ts` which serves account/rate-limit data.
// Naming: every method is prefixed `artifactUsage*` so search and
// auto-complete don't conflate the two.

import { invoke } from "@tauri-apps/api/core";
import type {
  ArtifactEverFiredDto,
  ArtifactUsageBatchEntryDto,
  ArtifactUsageRowDto,
  ArtifactUsageStatsDto,
  ArtifactUsageKind,
  UnusedReportDto,
} from "../types";

export const artifactUsageApi = {
  artifactUsageFor: (kind: ArtifactUsageKind, artifactKey: string) =>
    invoke<ArtifactUsageStatsDto>("artifact_usage_for", {
      kind,
      artifactKey,
    }),
  artifactUsageBatch: (keys: ReadonlyArray<[ArtifactUsageKind, string]>) =>
    invoke<ArtifactUsageBatchEntryDto[]>("artifact_usage_batch", {
      // Tauri serializes tuple-as-array; the backend deserializes the
      // same way (Vec<(String, String)>).
      keys,
    }),
  artifactUsageTop: (kind: ArtifactUsageKind | null, limit: number) =>
    invoke<ArtifactUsageRowDto[]>("artifact_usage_top", { kind, limit }),
  /**
   * Every artifact ever observed firing. One query; subtract from the
   * installed inventory to get the unused set. Do not reach for
   * `artifactUsageBatch` here — it costs six SQL statements per key.
   */
  artifactUsageEverFired: () =>
    invoke<ArtifactEverFiredDto[]>("artifact_usage_ever_fired"),
  /**
   * The Unused view's rows, computed in core. The renderer does no
   * filtering — identity, dedup, ledger subtraction, grace window and
   * the disabled-plugin exclusion all happen backend-side.
   */
  artifactUsageUnused: () => invoke<UnusedReportDto>("artifact_usage_unused"),
};
