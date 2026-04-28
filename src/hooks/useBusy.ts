import { useCallback, useMemo, useState } from "react";

/**
 * Per-key busy tracking with concurrent-start safety.
 *
 * Audit M19: the original was a `Set<string>` — if two overlapping
 * `withBusy("cli-<uuid>")` calls started before either finished, the
 * Set collapsed them to one entry. The first finally deleted the key
 * while the second call was still running, and the UI re-enabled
 * controls mid-work. The same action is triggerable from several
 * surfaces (Sidebar, AccountActions, CommandPalette, context menus),
 * so duplicate starts are plausible.
 *
 * Now we count per key. addBusy / withBusy increment, removeBusy
 * decrements, and the key is considered busy iff its count > 0. The
 * public `busyKeys` is a derived Set so consumers checking `has(key)`
 * keep working without changes.
 */
export function useBusy() {
  const [counts, setCounts] = useState<Record<string, number>>({});

  // Identities pinned with useCallback — `withBusy` is read as a dep
  // by `useActions`, which is in turn read as a dep by AppStateProvider's
  // context-value memo. A fresh closure per render here propagated all
  // the way up and forced every `useAppState()` consumer to re-render
  // on every parent render.
  const bump = useCallback((key: string, delta: number) => {
    setCounts((prev) => {
      const cur = prev[key] ?? 0;
      const next = cur + delta;
      const copy: Record<string, number> = { ...prev };
      if (next <= 0) delete copy[key];
      else copy[key] = next;
      return copy;
    });
  }, []);

  const withBusy = useCallback(
    async <T>(key: string, fn: () => Promise<T>): Promise<T> => {
      bump(key, 1);
      try {
        return await fn();
      } finally {
        bump(key, -1);
      }
    },
    [bump],
  );

  const addBusy = useCallback((key: string) => bump(key, 1), [bump]);
  const removeBusy = useCallback((key: string) => bump(key, -1), [bump]);

  // Derived Set so existing consumers that call `busyKeys.has(key)`
  // work unchanged. Memoized so identity is stable across renders
  // that didn't change the counts.
  const busyKeys = useMemo(() => new Set(Object.keys(counts)), [counts]);
  const anyBusy = busyKeys.size > 0;

  return { busyKeys, anyBusy, withBusy, addBusy, removeBusy };
}
