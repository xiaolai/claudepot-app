import { useCallback, useEffect, useState } from "react";

const STORAGE_KEY = "claudepot.activeSection";

/**
 * Track which top-level section is active. Persists the choice to
 * localStorage so reloads restore the user's last view, and binds
 * ⌘1..⌘9 (or Ctrl+1..9 on non-macOS) to the first nine sections.
 *
 * Callers pass the list of valid ids; unknown ids in localStorage are
 * silently replaced by `defaultId` so a stale key from an older build
 * doesn't wedge the UI.
 */
export function useSection<Id extends string>(
  defaultId: Id,
  ids: readonly Id[]
): { section: Id; setSection: (id: Id) => void } {
  const [section, setSectionState] = useState<Id>(() => {
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      if (stored && (ids as readonly string[]).includes(stored)) {
        return stored as Id;
      }
    } catch {
      // localStorage may throw in private-mode or headless envs — fall
      // through to the default.
    }
    return defaultId;
  });

  const setSection = useCallback(
    (id: Id) => {
      setSectionState(id);
      try {
        localStorage.setItem(STORAGE_KEY, id);
      } catch {
        // Persistence is best-effort; swallow write failures.
      }
    },
    [],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || e.shiftKey || e.altKey) return;
      // "1".."9" maps to ids[0..8]. Anything past the registered list
      // is ignored.
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

  return { section, setSection };
}
