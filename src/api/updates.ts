// Updates API binding — wraps the `updates_*` Tauri commands.
// State is persisted on the Rust side at ~/.claudepot/updates.json;
// CC's own settings (channel, minimumVersion, DISABLE_*) live in
// ~/.claude/settings.json and are routed through `cc_settings` on
// the status DTO.

import { invoke } from "@tauri-apps/api/core";
import type {
  CliInstallResultDto,
  DesktopInstallResultDto,
  UpdateSettings,
  UpdatesStatusDto,
} from "../types/updates";

export interface UpdatesSettingsPatch {
  cli_notify_on_available?: boolean;
  cli_notify_os_on_available?: boolean;
  cli_force_update_on_check?: boolean;
  desktop_notify_on_available?: boolean;
  desktop_notify_os_on_available?: boolean;
  desktop_auto_install_when_quit?: boolean;
  /**
   * Outer-Optional vs inner-null: pass `undefined` to leave unchanged,
   * or pass `{ poll_interval_minutes: null }` to clear the override
   * and fall back to the default (240 min). The Rust side accepts
   * `Option<Option<u32>>` to model this.
   */
  poll_interval_minutes?: number | null;
}

export const updatesApi = {
  /**
   * Read current snapshot — detection + cached probe results + settings.
   * Pure (no network); safe to call often for badge refresh.
   */
  updatesStatusGet: () => invoke<UpdatesStatusDto>("updates_status_get"),

  /**
   * Force a fresh upstream probe (CC + Desktop). Persists to
   * `updates.json`. Returns the refreshed snapshot with the
   * just-probed `latest_remote` values populated.
   */
  updatesCheckNow: () => invoke<UpdatesStatusDto>("updates_check_now"),

  /**
   * Force `claude update` against the active install. Refuses with a
   * surfaced error string if `DISABLE_UPDATES=1` is set in CC's
   * settings.json or no active install is found.
   */
  updatesCliInstall: () => invoke<CliInstallResultDto>("updates_cli_install"),

  /**
   * Drive a Desktop install. Refuses with a surfaced error string if
   * Desktop is currently running. Routes through `brew upgrade --cask`
   * when brew-managed, direct .zip + codesign-verified install otherwise.
   */
  updatesDesktopInstall: () =>
    invoke<DesktopInstallResultDto>("updates_desktop_install"),

  /** Read just the Claudepot-side settings (subset of the full status). */
  updatesSettingsGet: () => invoke<UpdateSettings>("updates_settings_get"),

  /**
   * Patch settings. Each field is optional — only `Some(_)` keys are
   * written. Returns the refreshed settings.
   */
  updatesSettingsSet: (patch: UpdatesSettingsPatch) =>
    invoke<UpdateSettings>("updates_settings_set", { ...patch }),

  /**
   * Set CC's release channel — writes to `~/.claude/settings.json`
   * (CC's own file, NOT Claudepot's). Pass null to clear the key
   * and fall back to CC's default.
   */
  updatesChannelSet: (channel: "latest" | "stable" | null) =>
    invoke<void>("updates_channel_set", { channel }),

  /**
   * Set CC's `minimumVersion` floor. Writes to ~/.claude/settings.json.
   * Pass null to clear the floor.
   */
  updatesMinimumVersionSet: (version: string | null) =>
    invoke<void>("updates_minimum_version_set", { version }),
};
