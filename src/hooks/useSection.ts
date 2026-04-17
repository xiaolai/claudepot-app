import { useCallback, useEffect, useState } from "react";

const STORAGE_KEY = "claudepot.activeSection";
const START_KEY = "claudepot.startSection";
const SUBROUTE_KEY_PREFIX = "claudepot.subRoute.";

function safeGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function safeSet(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Persistence is best-effort.
  }
}

/**
 * Track which top-level section is active, plus an optional per-section
 * sub-route (e.g. `projects` has a `repair` subview). Both values are
 * persisted to localStorage — the section under a single key, sub-routes
 * under `claudepot.subRoute.<sectionId>` so switching sections doesn't
 * trample another section's state.
 *
 * `setSection(id, subRoute)` can atomically set both — useful when
 * deep-linking from a banner ("open Projects → repair" in one call).
 *
 * Callers pass the list of valid ids; unknown ids in localStorage are
 * silently replaced by `defaultId` so a stale key from an older build
 * doesn't wedge the UI. Sub-route values are NOT validated — each
 * section owns its own sub-route vocabulary.
 *
 * ⌘1..⌘9 binds to the first nine sections (ignored when other
 * modifiers are present).
 */
export function useSection<Id extends string>(
  defaultId: Id,
  ids: readonly Id[]
): {
  section: Id;
  subRoute: string | null;
  setSection: (id: Id, subRoute?: string | null) => void;
  setSubRoute: (subRoute: string | null) => void;
} {
  const [section, setSectionState] = useState<Id>(() => {
    // Startup resolution:
    //   1. `claudepot.startSection` if user chose an explicit "Open on launch"
    //      preference in Settings — this is the authoritative startup value.
    //   2. `claudepot.activeSection` — the section last navigated to in the
    //      previous session. Used when the user has no explicit startup
    //      preference (legacy behavior).
    //   3. defaultId.
    //
    // `activeSection` is still written on every navigation so it stays
    // accurate as a secondary fallback, but it NEVER overwrites the
    // explicit startSection preference.
    const start = safeGet(START_KEY);
    if (start && (ids as readonly string[]).includes(start)) {
      return start as Id;
    }
    const stored = safeGet(STORAGE_KEY);
    if (stored && (ids as readonly string[]).includes(stored)) {
      return stored as Id;
    }
    return defaultId;
  });

  const [subRoute, setSubRouteState] = useState<string | null>(() =>
    safeGet(SUBROUTE_KEY_PREFIX + section),
  );

  const setSubRoute = useCallback(
    (next: string | null) => {
      setSubRouteState(next);
      if (next === null) {
        try {
          localStorage.removeItem(SUBROUTE_KEY_PREFIX + section);
        } catch {
          // ignore
        }
      } else {
        safeSet(SUBROUTE_KEY_PREFIX + section, next);
      }
    },
    [section],
  );

  const setSection = useCallback(
    (id: Id, nextSubRoute?: string | null) => {
      setSectionState(id);
      safeSet(STORAGE_KEY, id);
      // Load the per-section subroute from storage (caller override wins).
      const resolved =
        nextSubRoute !== undefined
          ? nextSubRoute
          : safeGet(SUBROUTE_KEY_PREFIX + id);
      setSubRouteState(resolved);
      if (nextSubRoute !== undefined) {
        if (nextSubRoute === null) {
          try {
            localStorage.removeItem(SUBROUTE_KEY_PREFIX + id);
          } catch {
            // ignore
          }
        } else {
          safeSet(SUBROUTE_KEY_PREFIX + id, nextSubRoute);
        }
      }
    },
    [],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || e.shiftKey || e.altKey) return;
      const n = Number.parseInt(e.key, 10);
      if (!Number.isInteger(n) || n < 1 || n > 9) return;
      const target = ids[n - 1];
      if (!target || target === section) return;
      e.preventDefault();
      setSection(target);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [ids, section, setSection]);

  return { section, subRoute, setSection, setSubRoute };
}
