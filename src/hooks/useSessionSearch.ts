import { useEffect, useRef, useState } from "react";
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
 *
 * Tauri's `invoke` has no cancellation primitive — we can't tell the
 * backend to stop scanning once we've submitted a query. Instead we
 * use a monotonic `requestSeqRef` counter: every new query bumps it,
 * and any in-flight response that isn't tagged with the *current* seq
 * is discarded on arrival. Combined with the debounce, this bounds
 * the number of concurrent scans to at most one in-flight plus one
 * scheduled.
 */
export function useSessionSearch(
  query: string,
  limit = 6,
): { hits: SearchHit[]; loading: boolean; error: string | null } {
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestSeqRef = useRef(0);

  useEffect(() => {
    const trimmed = query.trim();
    if (trimmed.length < 2) {
      // Reset immediately on short queries so stale hits from a longer
      // previous query don't flash on screen.
      setHits([]);
      setLoading(false);
      setError(null);
      return;
    }
    // Clear hits up-front so the user sees the loading state rather
    // than stale results for the old query while they type.
    setHits([]);
    setLoading(true);
    setError(null);
    const mySeq = ++requestSeqRef.current;
    const handle = setTimeout(() => {
      api
        .sessionSearch(trimmed, limit)
        .then((results) => {
          // Ignore responses from any query superseded by a newer one.
          if (mySeq !== requestSeqRef.current) return;
          setHits(results);
          setLoading(false);
        })
        .catch((e) => {
          if (mySeq !== requestSeqRef.current) return;
          setError(String(e));
          setLoading(false);
        });
    }, 250);
    return () => {
      // Bumping the seq guarantees any in-flight response for `mySeq`
      // will see a newer value in the ref and bail out. `clearTimeout`
      // avoids firing the request at all if the debounce timer hasn't
      // elapsed yet.
      requestSeqRef.current++;
      clearTimeout(handle);
    };
  }, [query, limit]);

  return { hits, loading, error };
}
