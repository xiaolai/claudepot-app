import { useEffect, useState } from "react";
import { api } from "../api";
import type { SearchHit } from "../types";

/**
 * Debounced cross-session text search. Feeds the Command palette.
 *
 * Flow:
 *  - `query` shorter than 2 chars → empty result, no network call.
 *  - Typing resets a 250ms timer; when it elapses, we call
 *    `api.sessionSearch(query, limit)`.
 *  - A stale result (query changed before the request returns) is
 *    discarded so the older response can't overwrite the newer one.
 */
export function useSessionSearch(
  query: string,
  limit = 6,
): { hits: SearchHit[]; loading: boolean; error: string | null } {
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const trimmed = query.trim();
    if (trimmed.length < 2) {
      setHits([]);
      setLoading(false);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    const handle = setTimeout(() => {
      api
        .sessionSearch(trimmed, limit)
        .then((results) => {
          if (cancelled) return;
          setHits(results);
          setLoading(false);
        })
        .catch((e) => {
          if (cancelled) return;
          setError(String(e));
          setLoading(false);
        });
    }, 250);
    return () => {
      cancelled = true;
      clearTimeout(handle);
    };
  }, [query, limit]);

  return { hits, loading, error };
}
