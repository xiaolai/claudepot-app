import type { SessionFilter } from "./sessionsTable.shared";

/**
 * Module-scoped mirror of the Sessions section's filter state.
 *
 * Motivation. SessionsSection is lazily mounted and navigating away
 * from it (e.g. via ⌘K or a cross-section deep-link) unmounts the
 * component, taking its local `useState` values with it. Users who
 * had a search query typed or a repo scope chosen would return to a
 * blank filter — which M-10 of the UX audit flagged.
 *
 * Same pattern used by `useSessionLive`: a module-scope value that
 * survives component lifecycles. We intentionally don't use
 * `localStorage` here — filter state should survive a tab hop but
 * not a full relaunch. Use `sessionStorage`-lite semantics:
 * in-memory, reset on reload.
 *
 * `selectedPath` lives here too because a user drilling into a
 * transcript and hopping to Accounts to check an anomaly should
 * land back on the same row.
 *
 * NOT reactive. Consumers seed their local state from this object
 * on mount and call `writeSessionsFilter` on change. There is no
 * subscription mechanism — the snapshot is only read once per
 * mount, so cross-mount updates are irrelevant.
 */
export interface SessionsFilterSnapshot {
  query: string;
  filter: SessionFilter;
  activeRepo: string | null;
  selectedPath: string | null;
  tab: "sessions" | "cleanup";
}

const store: SessionsFilterSnapshot = {
  query: "",
  filter: "all",
  activeRepo: null,
  selectedPath: null,
  tab: "sessions",
};

export function readSessionsFilter(): SessionsFilterSnapshot {
  return { ...store };
}

export function writeSessionsFilter(patch: Partial<SessionsFilterSnapshot>): void {
  Object.assign(store, patch);
}

/** Test hook — reset so each test starts from a clean slate. */
export function resetSessionsFilterForTest(): void {
  store.query = "";
  store.filter = "all";
  store.activeRepo = null;
  store.selectedPath = null;
  store.tab = "sessions";
}
