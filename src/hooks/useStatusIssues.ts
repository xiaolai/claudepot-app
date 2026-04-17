import { useMemo } from "react";
import type { AccountSummary, AppStatus, CcIdentity } from "../types";

export interface StatusIssue {
  id: string;
  severity: "error" | "warning" | "info";
  label: string;
  detail?: string;
  action?: { label: string; onClick: () => void };
}

export function useStatusIssues(opts: {
  ccIdentity: CcIdentity | null;
  status: AppStatus | null;
  syncError: string | null;
  keychainIssue: string | null;
  accounts: AccountSummary[];
  onUnlock: () => void;
}): StatusIssue[] {
  const { ccIdentity, status, syncError, keychainIssue, accounts, onUnlock } = opts;

  return useMemo(() => {
    const issues: StatusIssue[] = [];

    if (keychainIssue) {
      // `unlock_keychain` is macOS-only — the Rust command returns
      // an error on other platforms. Hide the button so users aren't
      // led into a dead-end click.
      const isMac = status?.platform === "macos";
      issues.push({
        id: "keychain",
        severity: "error",
        label: "Keychain locked",
        detail: isMac
          ? "Click Unlock to enter your macOS password."
          : "Unlock the system keychain, then click Refresh.",
        action: isMac ? { label: "Unlock", onClick: onUnlock } : undefined,
      });
    }

    const driftAccounts = accounts.filter((a) => a.drift);
    if (driftAccounts.length > 0) {
      issues.push({
        id: "drift",
        severity: "error",
        label: "Account drift detected",
        detail: driftAccounts
          .map((a) => `${a.email} authenticates as ${a.verified_email}`)
          .join("; "),
      });
    }

    if (syncError) {
      issues.push({
        id: "sync",
        severity: "warning",
        label: "Couldn't sync with Claude Code",
        detail: syncError,
      });
    }

    if (
      ccIdentity?.email &&
      status?.cli_active_email &&
      ccIdentity.email.toLowerCase() !== status.cli_active_email.toLowerCase()
    ) {
      issues.push({
        id: "cc-drift",
        severity: "warning",
        label: `CC slot drift — CC authenticates as ${ccIdentity.email}, Claudepot expects ${status.cli_active_email}`,
      });
    }

    return issues;
  }, [ccIdentity, status, syncError, keychainIssue, accounts, onUnlock]);
}
