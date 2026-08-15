import {
  startTransition,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";

import {
  SECTION_ACTIVE_KEY as STORAGE_KEY,
  SECTION_START_KEY as START_KEY,
  SECTION_SUBROUTE_KEY_PREFIX as SUBROUTE_KEY_PREFIX,
} from "../lib/storageKeys";
import { requestIdle, cancelIdle } from "../lib/idle";
import { isShortcutContextBlocked } from "./useGlobalShortcuts";
import { canonicalSectionId } from "../lib/destination";

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
 * ⌘1..⌘9 binds to position in `numbered` — the FULL registry order —
 * not to position in `ids`. The two are different lists and conflating
 * them made a section's number depend on its neighbours' visibility:
 * Boards ships off and sits ninth, so enabling it moved Settings off
 * ⌘9 and handed the key to Boards. The registry comment justified
 * Boards' position by saying an earlier slot "would renumber sections
 * people already have in muscle memory" — which the toggle did anyway,
 * just conditionally.
 *
 * Numbering by registry position makes every section's number a fixed
 * property of the section. The cost is that ⌘9 is inert while Boards
 * is off, and Settings (tenth) has no number at all — it keeps ⌘,
 * which is the standard binding users reach for anyway. A key that
 * does nothing is better than a key that does something different
 * depending on a setting elsewhere.
 *
 * `ids` still governs *validity*: a number pointing at a disabled
 * section navigates nowhere.
 */
export function useSection<Id extends string>(
  defaultId: Id,
  ids: readonly Id[],
  /** Full registry order. Defaults to `ids` so tests that only care
   *  about validity need not pass it. */
  numbered: readonly Id[] = ids,
): {
  section: Id;
  subRoute: string | null;
  setSection: (id: Id, subRoute?: string | null) => void;
  setSubRoute: (subRoute: string | null) => void;
} {
  // Always mount on `defaultId` (Accounts) for the first paint —
  // Accounts is bundled into the main chunk so the shell lands with
  // real content in one round-trip, without waiting for a lazy chunk
  // fetch (Projects / Sessions / Settings) to resolve. The persisted
  // "Open on launch" / last-active section is restored on the next
  // idle tick, after first paint has committed.
  const [section, setSectionState] = useState<Id>(defaultId);
  const [subRoute, setSubRouteState] = useState<string | null>(() =>
    safeGet(SUBROUTE_KEY_PREFIX + defaultId),
  );

  // Mirror `section` into a ref so the keydown handler can read the
  // current value without forcing the listener effect to re-run on
  // every navigation, and so the deferred startup restore below can
  // check whether the user has already navigated.
  const sectionRef = useRef(section);
  useEffect(() => {
    sectionRef.current = section;
  }, [section]);

  // Same idea for `ids`, which is no longer static: the startup restore
  // must validate against the list as it is when the idle callback
  // fires, not as it was when the effect was scheduled.
  const idsRef = useRef(ids);
  useEffect(() => {
    idsRef.current = ids;
  }, [ids]);

  const numberedRef = useRef(numbered);
  useEffect(() => {
    numberedRef.current = numbered;
  }, [numbered]);

  // Set by any explicit `setSection`. Comparing `sectionRef` against
  // `defaultId` was not enough: navigating away and back to the
  // default before the idle restore fired left it indistinguishable
  // from "never navigated", so the saved section still overwrote a
  // deliberate choice.
  const userNavigated = useRef(false);

  useEffect(() => {
    // Startup resolution (deferred to post-paint):
    //   1. `claudepot.startSection` if user chose an explicit "Open on
    //      launch" preference in Settings — authoritative.
    //   2. `claudepot.activeSection` — the section last navigated to.
    //   3. defaultId (already active, nothing to do).
    // Through `canonicalSectionId` so a RENAMED section restores to its
    // new id instead of failing the `includes` check and dropping the
    // user to Accounts. A stale id from an older build still falls back
    // — the two cases look identical from here, and only the map can
    // tell them apart.
    const start = canonicalSectionId(safeGet(START_KEY));
    const stored = canonicalSectionId(safeGet(STORAGE_KEY));
    const target: Id | null =
      start && (ids as readonly string[]).includes(start)
        ? (start as Id)
        : stored && (ids as readonly string[]).includes(stored)
          ? (stored as Id)
          : null;
    if (!target || target === defaultId) return;

    const handle = requestIdle(() => {
      // Re-validate at fire time, not at schedule time. Two things can
      // change during the idle wait:
      //
      //   - the user navigates, and restoring a saved section on top of
      //     a deliberate click is a stolen navigation;
      //   - an optional section is switched off, and `target` is now a
      //     section that no longer exists.
      //
      // Both were possible because `target` was computed once when the
      // effect ran.
      if (userNavigated.current) return;
      if (sectionRef.current !== defaultId) return;
      if (!(idsRef.current as readonly string[]).includes(target)) return;

      // Wrap in startTransition so the still-mounted default-section
      // tree stays on screen while the saved-section's lazy chunk
      // resolves — eliminates the blank Suspense fallback flash.
      startTransition(() => {
        setSectionState(target);
        setSubRouteState(safeGet(SUBROUTE_KEY_PREFIX + target));
      });
    });
    return () => cancelIdle(handle);
    // Startup resolution runs once per mount. `ids` CAN change at
    // runtime now (optional sections) — that case is handled by the
    // reconciliation effect below, not by re-running startup.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Reconcile when the active section stops existing.
  //
  // `ids` used to be a compile-time constant. Optional sections make it
  // shrink at runtime, and without this the app keeps rendering a
  // section that is gone from the sidebar, the palette, and the ⌘
  // bindings — visible but unreachable, the mirror of the bug the
  // filter exists to prevent.
  //
  // Centralized here on purpose: every caller that can hide a section
  // would otherwise need its own copy of this check, which is the
  // duplicate-list failure `AGENTS.md` calls a review finding.
  useEffect(() => {
    if ((ids as readonly string[]).includes(section)) return;
    setSectionState(defaultId);
    setSubRouteState(safeGet(SUBROUTE_KEY_PREFIX + defaultId));
    safeSet(STORAGE_KEY, defaultId);
  }, [ids, section, defaultId]);

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
      // Any explicit navigation cancels the deferred startup restore,
      // including one that lands back on the default section.
      userNavigated.current = true;
      // Persist synchronously — localStorage writes are cheap and the
      // user's choice survives even if the transition is interrupted.
      safeSet(STORAGE_KEY, id);
      const resolved =
        nextSubRoute !== undefined
          ? nextSubRoute
          : safeGet(SUBROUTE_KEY_PREFIX + id);
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
      // Wrap the React state updates in a transition. When the target
      // section is a React.lazy chunk that hasn't loaded yet, React
      // keeps rendering the current section's tree instead of falling
      // back to `null` — no blank flash between sections. Persistence
      // above runs eagerly because we want the saved-state to reflect
      // the user's intent even if they navigate again before the
      // transition commits.
      startTransition(() => {
        setSectionState(id);
        setSubRouteState(resolved);
      });
    },
    [],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || e.shiftKey || e.altKey) return;
      // rules/design.md: shortcuts never fire while an input is
      // focused or a modal is open — via the shared predicate, not a
      // local re-derivation. This was a second copy: behaviourally
      // equivalent at the time it was written, which is exactly how
      // the copies drift apart later.
      if (isShortcutContextBlocked()) return;
      const n = Number.parseInt(e.key, 10);
      if (!Number.isInteger(n) || n < 1 || n > 9) return;
      // Position in the full registry, then validity against the
      // enabled list. A number whose section is switched off simply
      // does nothing rather than sliding to the next visible section.
      const target = numberedRef.current[n - 1];
      if (!target || target === sectionRef.current) return;
      if (!(idsRef.current as readonly string[]).includes(target)) return;
      e.preventDefault();
      setSection(target);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setSection]);

  return { section, subRoute, setSection, setSubRoute };
}
