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
  accountAddFromToken: (refreshToken: string) =>
    invoke<RegisterOutcome>("account_add_from_token", { refreshToken }),
  accountRemove: (uuid: string) =>
    invoke<RemoveOutcome>("account_remove", { uuid }),
};
