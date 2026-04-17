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
  /**
   * Select an account in the sidebar. Wired by drift banners so
   * clicking a drift issue jumps the user to the affected row. Omit
   * to disable the jump affordance (tests).
   */
  onSelectAccount?: (uuid: string) => void;
  /**
   * Kick off the per-account re-login flow (browser OAuth). Wired so
   * the CC-slot-drift banner can offer a one-click reconcile when the
   * user knows which account should own the slot.
   */
  onReloginActive?: () => void;
}): StatusIssue[] {
  const {
    ccIdentity,
    status,
    syncError,
    keychainIssue,
    accounts,
    onUnlock,
    onSelectAccount,
    onReloginActive,
  } = opts;

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
      // Single-drift: jump to that account. Multi-drift: jump to the
      // first one — the detail text lists all offenders so the user
      // can triage from there.
      const primary = driftAccounts[0];
      issues.push({
        id: "drift",
        severity: "error",
        label: "Account drift detected",
        detail: driftAccounts
          .map((a) => `${a.email} authenticates as ${a.verified_email}`)
          .join("; "),
        action:
          onSelectAccount && primary
            ? { label: "Open", onClick: () => onSelectAccount(primary.uuid) }
            : undefined,
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
      // Try to match CC's verified email to a registered account so
      // the action can route the user to the right slot. Prefix match
      // mirrors the CLI's resolve logic.
      const target = accounts.find(
        (a) => a.email.toLowerCase() === ccIdentity.email!.toLowerCase(),
      );
      issues.push({
        id: "cc-drift",
        severity: "warning",
        label: `CC slot drift — CC authenticates as ${ccIdentity.email}, Claudepot expects ${status.cli_active_email}`,
        action:
          target && onSelectAccount
            ? {
                label: "Open matching account",
                onClick: () => onSelectAccount(target.uuid),
              }
            : onReloginActive
              ? { label: "Re-login active", onClick: onReloginActive }
              : undefined,
      });
    }

    return issues;
  }, [
    ccIdentity,
    status,
    syncError,
    keychainIssue,
    accounts,
    onUnlock,
    onSelectAccount,
    onReloginActive,
  ]);
}
