import { useMemo } from "react";
import type { AccountSummary, AppStatus, CcIdentity } from "../types";

export interface StatusAction {
  label: string;
  onClick: () => void;
}

export interface StatusIssue {
  id: string;
  severity: "error" | "warning" | "info";
  label: string;
  detail?: string;
  /** Primary action — rendered with accent weight. */
  action?: StatusAction;
  /**
   * Optional secondary action — rendered ghost-weight next to the
   * primary. Used when a condition has two meaningful resolutions
   * (e.g. CC-slot drift: import the current login OR re-login the
   * expected one). Banner always renders `action` first when both
   * are present.
   */
  action2?: StatusAction;
  /**
   * True if this issue supports being dismissed ("snoozed") for 24 h.
   * Used for nuisance warnings the user has acknowledged and accepted
   * (e.g. an intentional drift pending cleanup). Errors should NOT be
   * dismissable — they always require resolution.
   */
  dismissable?: boolean;
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
  /**
   * Register CC's currently-authenticated email as a new Claudepot
   * account (via `account_add_from_current`). Wired so the
   * CC-slot-drift banner can offer the opposite reconciliation when
   * the drifted email isn't registered yet. `email` is passed through
   * purely for the button label; the backend reads whatever CC has.
   */
  onImportCurrent?: (email: string) => void;
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
    onImportCurrent,
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
      // Key by the specific error payload so snoozing one sync failure
      // doesn't suppress a later, genuinely different failure. The
      // dismissedIssues store's 24 h expiry still applies per key.
      issues.push({
        id: `sync:${syncError}`,
        severity: "warning",
        label: "Couldn't sync with Claude Code",
        detail: syncError,
        dismissable: true,
      });
    }

    if (
      ccIdentity?.email &&
      status?.cli_active_email &&
      ccIdentity.email.toLowerCase() !== status.cli_active_email.toLowerCase()
    ) {
      // Audit M20: match via unique-prefix semantics, mirroring the
      // CLI's resolve_email rule. The previous exact-email match
      // missed resolvable accounts — e.g. ccIdentity is "alice@"
      // (partial) or the stored email has a case difference beyond
      // ASCII. Accept only a unique prefix; if multiple accounts
      // match or none, fall back to the re-login action path.
      const q = ccIdentity.email!.toLowerCase();
      const exactMatches = accounts.filter(
        (a) => a.email.toLowerCase() === q,
      );
      const prefixMatches = accounts.filter((a) =>
        a.email.toLowerCase().startsWith(q),
      );
      const target =
        exactMatches.length === 1
          ? exactMatches[0]
          : prefixMatches.length === 1
            ? prefixMatches[0]
            : undefined;
      // Action shape depends on whether CC's current email resolves
      // to a registered Claudepot account.
      //
      //   • resolves  → single action ("Open matching account"); the
      //                 per-row AnomalyBanner already offers re-login
      //                 and there's no import to do. If the consumer
      //                 didn't wire `onSelectAccount` (tests, or an
      //                 embedding without a sidebar), leave the
      //                 banner action-less — never fall through to
      //                 "Import", because the email is already
      //                 registered and re-importing would duplicate.
      //   • unknown   → two actions: primary "Import {email}" (adopt
      //                 the drifted login as a new account) and
      //                 secondary "Re-login active" (overwrite CC
      //                 with the expected email). Both are valid
      //                 resolutions — the user picks direction.
      let action: StatusAction | undefined;
      let action2: StatusAction | undefined;
      if (target) {
        if (onSelectAccount) {
          action = {
            label: "Open matching account",
            onClick: () => onSelectAccount(target.uuid),
          };
        }
      } else {
        if (onImportCurrent) {
          const email = ccIdentity.email;
          action = {
            label: `Import ${email}`,
            onClick: () => onImportCurrent(email),
          };
        }
        if (onReloginActive) {
          action2 = { label: "Re-login active", onClick: onReloginActive };
        }
        // If only re-login is wired, promote it to primary so the
        // banner still has a primary-weight action.
        if (!action && action2) {
          action = action2;
          action2 = undefined;
        }
      }
      issues.push({
        // Key by the email pair so snoozing drift-for-A-vs-B doesn't
        // suppress a later drift-for-C-vs-D. Lower-case to match the
        // email comparison above.
        id: `cc-drift:${ccIdentity.email.toLowerCase()}:${status.cli_active_email.toLowerCase()}`,
        severity: "warning",
        label: `CC slot drift — CC authenticates as ${ccIdentity.email}, Claudepot expects ${status.cli_active_email}`,
        action,
        action2,
        dismissable: true,
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
    onImportCurrent,
  ]);
}
