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
  accountRemove: (uuid: string) =>
    invoke<RemoveOutcome>("account_remove", { uuid }),
};
