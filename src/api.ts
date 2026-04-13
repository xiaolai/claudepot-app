// Thin wrappers around Tauri's invoke — one function per Rust command.
import { invoke } from "@tauri-apps/api/core";
import type {
  AccountSummary,
  AppStatus,
  RegisterOutcome,
  RemoveOutcome,
} from "./types";

export const api = {
  appStatus: () => invoke<AppStatus>("app_status"),
  /// Idempotent startup adoption: if CC holds credentials for one of the
  /// registered accounts, imports them into the matching slot. Returns
  /// the synced email (empty string when nothing matched).
  syncFromCurrentCc: () => invoke<string>("sync_from_current_cc"),
  /// macOS-only: request a native keychain-unlock dialog. The user's
  /// password is entered directly into macOS's own trusted prompt and
  /// never reaches Claudepot.
  unlockKeychain: () => invoke<void>("unlock_keychain"),
  accountList: () => invoke<AccountSummary[]>("account_list"),
  cliUse: (email: string) => invoke<void>("cli_use", { email }),
  cliClear: () => invoke<void>("cli_clear"),
  desktopUse: (email: string, noLaunch: boolean) =>
    invoke<void>("desktop_use", { email, noLaunch }),
  accountAddFromCurrent: () =>
    invoke<RegisterOutcome>("account_add_from_current"),
  // Token-based onboarding is CLI-only — the refresh token must never enter
  // the webview JS heap. Use a future browser-flow command instead.
  /// Re-log in via browser (opens Claude's OAuth flow) and imports the
  /// resulting blob into the given account's slot. Can take several
  /// minutes while the user completes auth in the browser.
  accountLogin: (uuid: string) => invoke<void>("account_login", { uuid }),
  accountLoginCancel: () => invoke<void>("account_login_cancel"),
  accountRemove: (uuid: string) =>
    invoke<RemoveOutcome>("account_remove", { uuid }),
};
