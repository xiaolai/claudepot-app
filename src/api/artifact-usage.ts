// Artifact usage telemetry — Tauri bridge for the four
// `artifact_usage_*` commands.
//
// Distinct from `pricing.ts` which serves account/rate-limit data.
// Naming: every method is prefixed `artifactUsage*` so search and
// auto-complete don't conflate the two.

import { invoke } from "@tauri-apps/api/core";
import type {
  ArtifactUsageBatchEntryDto,
  ArtifactUsageRowDto,
  ArtifactUsageStatsDto,
  ArtifactUsageKind,
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
};
