import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import type {
  ArtifactUsageKind,
  ArtifactUsageStatsDto,
  ConfigTreeDto,
} from "../types";
import { artifactKeyForFile } from "../sections/config/artifactKey";

/**
 * Fetch artifact-usage stats for every trackable file in a Config
 * tree. Results are keyed by FileNode.id so the renderer can do a
 * direct lookup without re-deriving the artifact key.
 *
 * Strategy:
 *   1. Walk the tree once, derive (kind, artifactKey) for each
 *      trackable file, and remember which fileId each pair belongs to.
 *   2. One round-trip via `artifactUsageBatch` for the whole set.
 *   3. Map results back to fileId so the FileRowButton can do
 *      `usage.get(file.id)` in O(1).
 *
 * Re-fetches when `tree` identity or `refreshKey` changes. Returns
 * `null` for the entire map until the first successful fetch — the
 * UI distinguishes "never used" (`stats.count_30d === 0`) from
 * "haven't fetched yet" (`null`).
 */
export function useArtifactUsage(
  tree: ConfigTreeDto | null,
  /**
   * Pass `true` when the renderer is in global-only mode (no project
   * anchor). The backend reports `tree.project_root` as the user home
   * in that mode, which would alias `~/.claude/skills/*` to project
   * scope; suppressing project-scope detection avoids the mis-key.
   * Defaults to false / scoped lookup.
   */
  globalOnly: boolean = false,
  refreshKey: number = 0,
): {
  usageByFileId: Map<string, ArtifactUsageStatsDto> | null;
  error: string | null;
  loading: boolean;
} {
  const [usageByFileId, setUsageByFileId] = useState<
    Map<string, ArtifactUsageStatsDto> | null
  >(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  // Stable list of (fileId, kind, key) triples derived from the tree.
  // Memoized on tree identity + project_root so applyPatch in
  // useConfigTree doesn't trigger a re-fetch unless the tree or
  // anchoring project actually changed. project_root is what tells
  // `artifactKeyForFile` which non-plugin skills are project-scope
  // (`projectSettings:<name>`) vs user-scope (`userSettings:<name>`).
  const projectRoot = globalOnly ? null : tree?.project_root ?? null;
  const triples = useMemo(() => {
    if (!tree) return [] as Array<[string, ArtifactUsageKind, string]>;
    const out: Array<[string, ArtifactUsageKind, string]> = [];
    for (const scope of tree.scopes) {
      for (const file of scope.files) {
        const r = artifactKeyForFile(file, projectRoot);
        if (r) out.push([file.id, r.kind, r.artifactKey]);
      }
    }
    return out;
  }, [tree, projectRoot]);

  useEffect(() => {
    if (!tree) {
      setUsageByFileId(null);
      setLoading(false);
      setError(null);
      return;
    }
    if (triples.length === 0) {
      setUsageByFileId(new Map());
      setLoading(false);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    const keys = triples.map(
      ([, kind, key]) => [kind, key] as [ArtifactUsageKind, string],
    );
    api
      .artifactUsageBatch(keys)
      .then((rows) => {
        if (cancelled) return;
        // Position-aligned: rows[i] corresponds to triples[i] when the
        // backend filtered nothing. The DTO carries kind+key so we
        // re-key the map by fileId via a join.
        const byKindKey = new Map<string, ArtifactUsageStatsDto>();
        for (const r of rows) {
          byKindKey.set(`${r.kind}|${r.artifact_key}`, r.stats);
        }
        const map = new Map<string, ArtifactUsageStatsDto>();
        for (const [fileId, kind, key] of triples) {
          const stats = byKindKey.get(`${kind}|${key}`);
          if (stats) map.set(fileId, stats);
        }
        setUsageByFileId(map);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [tree, triples, refreshKey]);

  return { usageByFileId, error, loading };
}
