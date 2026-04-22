import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { ProjectInfo, RepositoryGroup, SessionRow } from "../types";

/**
 * Data lifecycle for the Sessions tab. Owns the three parallel
 * fetches (`sessionListAll`, `projectList`, `sessionWorktreeGroups`),
 * the loading/error state, and the per-call cancellation token that
 * keeps a stale refresh from clobbering a fresher one.
 *
 * Resilience contract (`Promise.allSettled`):
 *   - `sessionListAll` is mandatory. A rejection sets `error` and
 *     leaves the table empty until the next refresh.
 *   - `projectList` failure falls back to `[]` and reports via
 *     `onSecondaryError` so the caller can surface a toast — the
 *     section still renders, just without project targets in the
 *     Move modal.
 *   - `sessionWorktreeGroups` failure falls back to `null` (the
 *     RepoFilterStrip already handles a null `groups` prop by not
 *     rendering); no user signal because the missing strip is the
 *     visible signal.
 */
export interface UseSessionsDataOptions {
  /** Bridge for surfacing a partial-failure message as a toast. */
  onSecondaryError: (message: string) => void;
}

export interface UseSessionsDataResult {
  sessions: SessionRow[];
  projects: ProjectInfo[];
  repoGroups: RepositoryGroup[] | null;
  loading: boolean;
  error: string | null;
  /** Refetch all three. Each call bumps a monotonic token; older
   * in-flight responses bail out on arrival. */
  refresh: () => void;
  /** Setters exposed so the section can prune selection state when
   * the dataset changes. */
  setSessions: React.Dispatch<React.SetStateAction<SessionRow[]>>;
  setRepoGroups: React.Dispatch<
    React.SetStateAction<RepositoryGroup[] | null>
  >;
}

export function useSessionsData(
  opts: UseSessionsDataOptions,
): UseSessionsDataResult {
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [repoGroups, setRepoGroups] = useState<RepositoryGroup[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const tokenRef = useRef(0);
  const mountedRef = useRef(true);
  const onSecondaryErrorRef = useRef(opts.onSecondaryError);
  // Keep the latest callback in a ref so `refresh` stays referentially
  // stable (its `useCallback` deps stay empty). Without this the
  // effect that triggers refresh-on-mount would re-run whenever the
  // caller passed a fresh inline arrow.
  useEffect(() => {
    onSecondaryErrorRef.current = opts.onSecondaryError;
  }, [opts.onSecondaryError]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const refresh = useCallback(() => {
    const myToken = ++tokenRef.current;
    setLoading(true);
    setError(null);
    Promise.allSettled([
      api.sessionListAll(),
      api.projectList(),
      api.sessionWorktreeGroups(),
    ]).then(([ssRes, psRes, groupsRes]) => {
      if (!mountedRef.current || myToken !== tokenRef.current) return;

      // Mandatory: session list. Failure aborts the render.
      if (ssRes.status === "rejected") {
        setError(String(ssRes.reason));
        setLoading(false);
        return;
      }
      setSessions(ssRes.value);

      // Secondary: projects.
      if (psRes.status === "fulfilled") {
        setProjects(psRes.value);
      } else {
        setProjects([]);
        onSecondaryErrorRef.current(
          `Couldn't load projects: ${String(psRes.reason)}`,
        );
        if (import.meta.env.DEV) {
          // eslint-disable-next-line no-console
          console.warn("[useSessionsData] projectList failed", psRes.reason);
        }
      }

      // Tertiary: worktree groups.
      if (groupsRes.status === "fulfilled") {
        setRepoGroups(groupsRes.value);
      } else {
        setRepoGroups(null);
        if (import.meta.env.DEV) {
          // eslint-disable-next-line no-console
          console.warn(
            "[useSessionsData] sessionWorktreeGroups failed; " +
              "RepoFilterStrip will not render",
            groupsRes.reason,
          );
        }
      }

      setLoading(false);
    });
  }, []);

  // Initial fetch.
  useEffect(() => {
    refresh();
  }, [refresh]);

  return {
    sessions,
    projects,
    repoGroups,
    loading,
    error,
    refresh,
    setSessions,
    setRepoGroups,
  };
}
