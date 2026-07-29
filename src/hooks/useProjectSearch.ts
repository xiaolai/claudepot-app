import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import { scoreFields } from "../lib/paletteScore";
import type { ProjectInfo } from "../types";

/**
 * Project lookup for the ⌘K palette.
 *
 * Unlike session search — which needs the backend's FTS index because
 * the corpus is unbounded — the project list is a bounded array, so
 * this fetches it once and scores locally. That makes typing
 * instantaneous with no debounce and no per-keystroke IPC.
 *
 * `project_list` stats every project directory to compute sizes, so
 * it is not free. The result is cached at module scope for
 * `CACHE_TTL_MS` and shared by every palette open, and the fetch is
 * deferred until the user actually types — opening ⌘K and hitting
 * Enter on "Open Projects" must not pay for a filesystem walk.
 */
const CACHE_TTL_MS = 30_000;
const MIN_QUERY = 2;

let cache: { at: number; projects: ProjectInfo[] } | null = null;
let inFlight: Promise<ProjectInfo[]> | null = null;
/**
 * Bumped by every invalidation. A request that was already in flight
 * when the cache was dropped must not write its result back — without
 * this, a deliberately-invalidated cache gets resurrected by a
 * response for the state that was thrown away.
 */
let generation = 0;

/** Exposed for tests — drops the cache and disowns any in-flight load. */
export function __resetProjectCache(): void {
  cache = null;
  inFlight = null;
  generation++;
}

function loadProjects(now: number): Promise<ProjectInfo[]> {
  if (cache && now - cache.at < CACHE_TTL_MS) {
    return Promise.resolve(cache.projects);
  }
  // Collapse concurrent callers onto one request; a palette that
  // remounts mid-flight must not start a second filesystem walk.
  const myGeneration = generation;
  inFlight ??= api
    .projectList()
    .then((projects) => {
      if (myGeneration === generation) {
        cache = { at: Date.now(), projects };
      }
      return projects;
    })
    .finally(() => {
      if (myGeneration === generation) inFlight = null;
    });
  return inFlight;
}

export function useProjectSearch(
  query: string,
  limit = 5,
): { hits: ProjectInfo[]; loading: boolean } {
  const [projects, setProjects] = useState<ProjectInfo[] | null>(null);
  const [loading, setLoading] = useState(false);
  const aliveRef = useRef(true);

  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
    };
  }, []);

  const trimmed = query.trim();
  const wanted = trimmed.length >= MIN_QUERY;

  useEffect(() => {
    if (!wanted || projects !== null) return;
    setLoading(true);
    loadProjects(Date.now())
      .then((list) => {
        if (!aliveRef.current) return;
        setProjects(list);
        setLoading(false);
      })
      .catch(() => {
        if (!aliveRef.current) return;
        // A failed project list is not worth a toast from the palette —
        // the section itself surfaces the error properly. Degrade to
        // "no project matches" rather than blocking the whole palette.
        setProjects([]);
        setLoading(false);
      });
  }, [wanted, projects]);

  if (!wanted) return { hits: [], loading: false };
  if (projects === null) return { hits: [], loading };

  const scored: { project: ProjectInfo; score: number }[] = [];
  for (const p of projects) {
    // Match on the basename first — that's the project's identity —
    // then fall back to the full path so "github/foo" still works.
    const score = scoreFields(trimmed, basename(p.original_path), [
      p.original_path,
    ]);
    if (score !== null) scored.push({ project: p, score });
  }
  scored.sort((a, b) => b.score - a.score);
  return { hits: scored.slice(0, limit).map((s) => s.project), loading: false };
}

/** Last path segment, separator-agnostic (CC stores native paths). */
export function basename(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}
