// Protected paths + General preferences.
// Sharded from src/api.ts; src/api/index.ts merges every
// domain slice into the canonical `api` object.

import { invoke } from "@tauri-apps/api/core";
import type {
  Preferences,
  ProtectedPath,
} from "../types";

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

};
