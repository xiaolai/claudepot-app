// Settings / preferences DTOs (protected paths + persisted UI prefs).
// Sharded out of project.ts in the audit-fix domain-coherence pass;
// index.ts re-exports them so `from "../types"` import sites
// resolve unchanged. Mirrors src-tauri/src/dto.rs.

/**
 * One row in the protected-paths Settings list. `source` drives the
 * badge: `"default"` rows came from the built-in DEFAULT_PATHS;
 * `"user"` rows are user-added.
 */
export interface ProtectedPath {
  path: string;
  source: "default" | "user";
}

/**
 * Persisted UI preferences. Backed by `preferences.json` in the
 * Claudepot data dir; read synchronously at Rust startup.
 */
export interface Preferences {
  /** macOS-only: when true, the app runs tray-only (no dock icon, no
   *  Cmd+Tab, no app menu bar). No-op on Windows/Linux. */
  hide_dock_icon: boolean;

  /** When false, the main window starts hidden on app launch; the
   *  tray icon brings it back. Pairs with `Launch at login` for a
   *  quiet tray-only background. Defaults to true. */
  show_window_on_startup: boolean;

  /** User opted in to the live Activity feature. Gate for starting
   *  the LiveRuntime. Defaults to false until the consent modal is
   *  accepted. */
  activity_enabled: boolean;

  /** First-run consent modal has been seen (accepted OR declined).
   *  Separate from activity_enabled so a user who declined once
   *  isn't re-prompted every launch. */
  activity_consent_seen: boolean;

  /** Thinking blocks render redacted-by-default with a "▸ reveal"
   *  affordance. Defaults to true — privacy-forward. */
  activity_hide_thinking: boolean;

  /** Project paths the live runtime should ignore. Path-prefix
   *  matched against PidRecord.cwd. */
  activity_excluded_paths: string[];

  notify_on_error: boolean;
  notify_on_idle_done: boolean;
  /** null = feature off; number = fire after N minutes stuck. */
  notify_on_stuck_minutes: number | null;
  /** Fires an OS notification when a long-running op terminates while
   *  the main window is unfocused. Default false — opt-in. */
  notify_on_op_done: boolean;
  /** Fires when a session transitions to Waiting (CC paused pending
   *  user input — permission, plan-mode approval, clarifying answer).
   *  Defaults to **true**: this is the highest-leverage alert in the
   *  set, gated behind `activity_enabled` which is already opt-in. */
  notify_on_waiting: boolean;
  /** Integer-percent thresholds (e.g. 80, 90) that fire when the
   *  CLI-active account's usage crosses them. Empty = feature off.
   *  Default `[90]` — single near-cap nudge per (window × cycle).
   *  The watcher polls every 5 min on the Rust side. Add 80 back
   *  via Settings → Notifications if you want the early warning. */
  notify_on_usage_thresholds: number[];

  /** Whether the per-model 7-day sub-windows (Opus, Sonnet)
   *  participate in usage-threshold alerts. Default false — these
   *  sub-quotas typically track the umbrella 7-day window for users
   *  near cap, so leaving them on triples the 7-day toast volume.
   *  The umbrella `seven_day` window is always checked regardless. */
  notify_on_sub_windows: boolean;
}
