// Types for the auto-updates feature (CC CLI + Claude Desktop manager).
// Mirrors src-tauri/src/dto_updates.rs and the public API of
// claudepot-core::updates.

export type CliInstallKind =
  | "native-curl"
  | "npm-global"
  | "homebrew-stable"
  | "homebrew-latest"
  | "apt"
  | "dnf"
  | "apk"
  | "win-get"
  | "unknown";

export interface CliInstall {
  kind: CliInstallKind;
  binary_path: string;
  version: string | null;
  /** True iff this is the install that runs when the user types `claude`. */
  is_active: boolean;
  /** True iff this install auto-updates itself (native + npm). */
  auto_updates: boolean;
}

export type DesktopSource =
  | "homebrew"
  | "direct-dmg"
  | "setapp"
  | "mac-app-store"
  | "user-local";

export interface DesktopInstall {
  app_path: string;
  version: string | null;
  source: DesktopSource;
  /** True iff Claudepot can drive an update on this install. */
  manageable: boolean;
}

/** Snapshot of the four CC settings keys that gate auto-update behavior. */
export interface CcUpdateSettings {
  auto_updates_channel: string | null;
  minimum_version: string | null;
  disable_autoupdater: boolean;
  disable_updates: boolean;
}

export interface CliStatusDto {
  channel: string;
  installs: CliInstall[];
  latest_remote: string | null;
  last_known: string | null;
  last_check_unix: number | null;
  last_error: string | null;
  cc_settings: CcUpdateSettings;
  running_count: number;
}

export interface DesktopStatusDto {
  install: DesktopInstall | null;
  running: boolean;
  latest_remote: string | null;
  latest_commit_sha: string | null;
  last_check_unix: number | null;
  last_error: string | null;
}

export interface CliSettings {
  /** Tray badge when an update is detected. */
  notify_on_available: boolean;
  /** OS notification (toast) when an update is detected. Deduped per
   *  version so reopening / re-checking doesn't re-fire. */
  notify_os_on_available: boolean;
  /** Run `claude update` automatically on every check that finds a delta. */
  force_update_on_check: boolean;
}

export interface DesktopSettings {
  notify_on_available: boolean;
  notify_os_on_available: boolean;
  auto_install_when_quit: boolean;
}

export interface UpdateSettings {
  cli: CliSettings;
  desktop: DesktopSettings;
  poll_interval_minutes: number | null;
}

/**
 * Outcome of an auto-install pass kicked off by `updates_check_now`.
 * `disabled` means the toggle is off; `up-to-date` means the toggle
 * fired but nothing was needed; `skipped` means a precondition
 * blocked the install (Desktop running, DISABLE_UPDATES, etc).
 */
export type AutoInstallOutcome =
  | { kind: "disabled" }
  | { kind: "up-to-date" }
  | { kind: "skipped"; reason: string }
  | { kind: "installed"; version: string | null }
  | { kind: "failed"; error: string };

export interface UpdatesStatusDto {
  cli: CliStatusDto;
  desktop: DesktopStatusDto;
  settings: UpdateSettings;
  /**
   * Auto-install result for the CLI side. Populated only by
   * `updates_check_now`; `updates_status_get` always returns
   * `{ kind: "disabled" }` here regardless of the toggle.
   */
  cli_auto_outcome: AutoInstallOutcome;
  desktop_auto_outcome: AutoInstallOutcome;
}

export interface CliInstallResultDto {
  stdout: string;
  stderr: string;
  installed_after: string | null;
}

export interface DesktopInstallResultDto {
  /** "brew" or "direct-zip" */
  method: string;
  version_after: string | null;
  stdout: string;
  stderr: string;
}
