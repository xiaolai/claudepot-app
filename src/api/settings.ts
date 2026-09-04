// Protected paths + General preferences.
// Sharded from src/api.ts; src/api/index.ts merges every
// domain slice into the canonical `api` object.

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import type {
  Preferences,
  ProtectedPath,
} from "../types";
import type { Category } from "../lib/notifications/types";

/**
 * Per-category notification preference. Mirrors
 * `src-tauri::preferences::CategoryPrefs`. The Settings pane reads
 * the whole map and exposes one row per category; `emit()` reads
 * an individual entry before dispatching to filter `surfaces_requested`.
 */
export interface CategoryPrefs {
  enabled: boolean;
  /** null = follow category priority default; true/false = force on/off. */
  osOverride: boolean | null;
}

export const settingsApi = {
  // ---------- Protected paths (Settings → Protected pane) ----------
  /**
   * Materialized list — defaults (minus removed_defaults) followed by
   * user-added entries in insertion order. Order is stable so the UI
   * can render without sorting.
   */
  protectedPathsList: () => invoke<ProtectedPath[]>("protected_paths_list"),
  /**
   * Add a path. Validates and persists. Returns the new entry; if the
   * path matches a previously-removed default, the entry comes back
   * with `source: "default"` (un-tombstoned, not duplicated under user).
   */
  protectedPathsAdd: (path: string) =>
    invoke<ProtectedPath>("protected_paths_add", { path }),
  /**
   * Remove a path. Defaults are tombstoned (so reset() brings them
   * back); user entries are dropped.
   */
  protectedPathsRemove: (path: string) =>
    invoke<void>("protected_paths_remove", { path }),
  /** Restore the implicit defaults; returns the resulting list. */
  protectedPathsReset: () => invoke<ProtectedPath[]>("protected_paths_reset"),

  // ---------- Preferences (Settings → General) ----------
  /** Read the current persisted UI preferences. */
  preferencesGet: () => invoke<Preferences>("preferences_get"),
  /**
   * Toggle hide-dock-icon. Applies `set_activation_policy` immediately
   * on macOS (Accessory = tray-only; Regular = dock + menu bar), then
   * persists the boolean. No-op on Windows/Linux.
   */
  preferencesSetHideDockIcon: (hide: boolean) =>
    invoke<void>("preferences_set_hide_dock_icon", { hide }),

  /**
   * Persist the "show main window on startup" toggle. The new value
   * applies on the next launch — the currently-visible window is not
   * touched. The user can hide / show through the tray icon.
   */
  preferencesSetShowWindowOnStartup: (show: boolean) =>
    invoke<void>("preferences_set_show_window_on_startup", { show }),

  /**
   * Pin / unpin the main window in front of other apps (the status
   * bar's pin). The backend changes the window level first and then
   * persists; a failed save puts the level back, so file, window and
   * button never disagree. Returns the refreshed snapshot and
   * broadcasts `cp-prefs-changed` with it — the same shape as
   * `preferencesSetServiceStatus`, so any other reader of the flag
   * moves without a second `preferencesGet`.
   */
  preferencesSetWindowAlwaysOnTop: (pinned: boolean) =>
    invoke<Preferences>("preferences_set_window_always_on_top", {
      pinned,
    }).then((p) => {
      void emit("cp-prefs-changed", p).catch(() => {});
      return p;
    }),

  /**
   * Persist the UI language preference. `null` = follow the OS
   * language. Pure persistence — the caller applies the change to the
   * live i18next instance via `applyLocalePreference`.
   */
  preferencesSetLocale: (locale: string | null) =>
    invoke<void>("preferences_set_locale", { locale }),

  /**
   * Read every category's effective notification preference. The
   * backend always returns a complete map — categories without an
   * explicit on-disk entry come back with their defaults filled in.
   */
  preferencesCategoryPrefsGet: () =>
    invoke<Record<Category, CategoryPrefs>>("preferences_category_prefs_get"),

  /**
   * Update one category's preference. The backend mirrors any
   * legacy scalar that maps to the same category (the dual-write
   * contract from Phase 1.5) so a downgrade keeps user state.
   * Returns the refreshed entry so the renderer's cache can sync.
   */
  preferencesCategoryPrefSet: (
    category: Category,
    prefs: CategoryPrefs,
  ) =>
    invoke<CategoryPrefs>("preferences_category_pref_set", {
      category,
      prefs,
    }),
};
