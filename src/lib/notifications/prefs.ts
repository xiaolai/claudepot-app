// Renderer-side cache of CategoryPrefs.
//
// emit() (`dispatch.ts`) reads from this cache synchronously when
// computing the effective `SurfaceSet`. The cache is hydrated on
// mount via `preferencesCategoryPrefsGet` and refreshed on every
// setter call. Same pattern as `useDismissedIssues` — keep the
// dispatch path fast; tolerate one extra IPC on Settings changes.
//
// A category not yet in the cache (cold start, IPC pending) is
// treated as "enabled with priority-default OS" — matches the
// Rust `CategoryPrefs::default()` so the cold-start window doesn't
// drop notifications.

import { settingsApi, type CategoryPrefs } from "../../api/settings";
import type { Category } from "./types";

let cache: Partial<Record<Category, CategoryPrefs>> = {};
let hydrated = false;
const subscribers = new Set<() => void>();

function notify() {
  for (const fn of subscribers) {
    try {
      fn();
    } catch {
      /* swallow — one bad subscriber must not poison the rest */
    }
  }
}

/** Read the effective preference for `category`. Falls back to the
 *  cold-start default when the cache hasn't hydrated yet — the
 *  default is "enabled, OS follows priority", which matches the
 *  Rust side. */
export function getCategoryPref(category: Category): CategoryPrefs {
  return cache[category] ?? { enabled: true, osOverride: null };
}

/** Subscribe to cache changes. Returns an unsubscribe. */
export function subscribeCategoryPrefs(fn: () => void): () => void {
  subscribers.add(fn);
  return () => {
    subscribers.delete(fn);
  };
}

/** Hydrate the cache from the Rust side. Idempotent — repeated
 *  calls re-fetch and update subscribers. Settings pane (Phase 4)
 *  invokes this on mount and after each setter. */
export async function hydrateCategoryPrefs(): Promise<void> {
  try {
    const map = await settingsApi.preferencesCategoryPrefsGet();
    cache = map;
    hydrated = true;
    notify();
  } catch {
    // Non-Tauri env or IPC error. Leave cache as-is; the
    // cold-start defaults already produce sensible behavior.
  }
}

/** Optimistically update one category in the cache. The Settings
 *  pane (Phase 4) calls this before the setter IPC resolves so
 *  the UI feels instant; the setter's reply re-syncs the canonical
 *  value when it arrives. */
export function setCategoryPrefLocal(
  category: Category,
  prefs: CategoryPrefs,
): void {
  cache[category] = prefs;
  notify();
}

/** Update a category preference via IPC + optimistic cache update.
 *  Returns the canonical value the backend confirmed. */
export async function updateCategoryPref(
  category: Category,
  prefs: CategoryPrefs,
): Promise<CategoryPrefs> {
  setCategoryPrefLocal(category, prefs);
  const confirmed = await settingsApi.preferencesCategoryPrefSet(
    category,
    prefs,
  );
  setCategoryPrefLocal(category, confirmed);
  return confirmed;
}

/** Test-only: introspect the cache hydration state. */
export function __isHydratedForTests(): boolean {
  return hydrated;
}

/** Test-only: reset the cache. Vitest needs this between tests
 *  because the singleton outlives any one render tree. */
export function __resetForTests(): void {
  cache = {};
  hydrated = false;
  subscribers.clear();
}
