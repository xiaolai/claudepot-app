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
  /** `true` when the visible rows came from the sessionStorage
   * snapshot rather than a fresh IPC round. The UI surfaces an
   * "Updating…" overlay while this holds alongside `loading=true`. */
  servedFromCache: boolean;
  /** Timestamp (ms) when the current fetch started. `null` when idle.
   * The UI ticks off "Updating… Ns" from this. */
  fetchStartedAt: number | null;
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

const CACHE_KEY = "claudepot.sessionsCache.v1";

function loadSessionCache(): SessionRow[] | null {
  try {
    const raw = sessionStorage.getItem(CACHE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return null;
    return parsed as SessionRow[];
  } catch {
    return null;
  }
}

function saveSessionCache(rows: SessionRow[]): void {
  try {
    sessionStorage.setItem(CACHE_KEY, JSON.stringify(rows));
  } catch {
    // Quota exceeded or sessionStorage disabled — drop silently; the
    // cache is an optimization, not a correctness boundary.
  }
}

export function useSessionsData(
  opts: UseSessionsDataOptions,
): UseSessionsDataResult {
  // Seed from sessionStorage so the table paints immediately on
  // re-entry, instead of rendering a blank "Loading…" state while the
  // backend parses cold. Stale rows beat a blank table — the fetch
  // still runs in parallel and replaces the data.
  const cachedOnMount = useRef<SessionRow[] | null>(loadSessionCache());
  const [sessions, setSessions] = useState<SessionRow[]>(
    () => cachedOnMount.current ?? [],
  );
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [repoGroups, setRepoGroups] = useState<RepositoryGroup[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [servedFromCache, setServedFromCache] = useState<boolean>(
    () => cachedOnMount.current !== null,
  );
  const [fetchStartedAt, setFetchStartedAt] = useState<number | null>(null);

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
    setFetchStartedAt(Date.now());
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
        setFetchStartedAt(null);
        return;
      }
      setSessions(ssRes.value);
      setServedFromCache(false);
      saveSessionCache(ssRes.value);

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
      setFetchStartedAt(null);
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
    servedFromCache,
    fetchStartedAt,
    refresh,
    setSessions,
    setRepoGroups,
  };
}
