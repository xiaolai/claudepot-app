import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { i18n } from "../lib/i18n";
import type {
  AccountSummary,
  AppStatus,
  CcIdentity,
  DesktopSyncOutcome,
} from "../types";

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
  /**
   * Non-null when the last `sync_from_current_cc` returned
   * `auth rejected:` — CC's stored refresh_token is terminally dead
   * and only a browser re-login recovers. Renders an error-severity
   * banner with a "Sign in again" primary action wired to
   * `onReloginActive`, distinct from the dismissable `syncError`
   * warning used for transient failures. Optional so existing tests
   * (which predate auto-refresh) keep working without a fixture churn.
   */
  authRejectedAt?: number | null;
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

  /**
   * Latest outcome of `sync_from_current_desktop`. Null when sync
   * hasn't run yet. UI surfaces adoption / stranger / candidate
   * banners based on the discriminated variant.
   */
  desktopSync?: DesktopSyncOutcome | null;
  /**
   * Trigger adopt-live-session for the matched Claudepot account.
   * Wired so the "adoption available" banner can run end-to-end.
   */
  onAdoptLiveDesktop?: (email: string) => void;
  /**
   * Open the Add-account modal pre-filled with the live Desktop
   * identity. Wired so the "stranger" banner can offer one-click
   * onboarding of an account Desktop is already signed into.
   */
  onImportDesktop?: (email: string) => void;
}): StatusIssue[] {
  const {
    ccIdentity,
    status,
    syncError,
    authRejectedAt,
    keychainIssue,
    accounts,
    onUnlock,
    onSelectAccount,
    onReloginActive,
    onImportCurrent,
    desktopSync,
    onAdoptLiveDesktop,
    onImportDesktop,
  } = opts;

  // Banner copy is resolved inside the memo, so the active locale is
  // one of its inputs — without it a language switch would leave the
  // already-derived banners in the previous language until some other
  // dependency happened to change.
  const { i18n: runtimeI18n } = useTranslation();
  const language = runtimeI18n.language;

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
        label: i18n.t("status.keychain.label"),
        detail: isMac
          ? i18n.t("status.keychain.detailMac")
          : i18n.t("status.keychain.detailOther"),
        action: isMac
          ? { label: i18n.t("status.keychain.action"), onClick: onUnlock }
          : undefined,
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
        label: i18n.t("status.drift.label"),
        detail: driftAccounts
          .map((a) =>
            i18n.t("status.drift.detailItem", {
              email: a.email,
              verified: a.verified_email,
            }),
          )
          .join("; "),
        action:
          onSelectAccount && primary
            ? {
                label: i18n.t("status.drift.action"),
                onClick: () => onSelectAccount(primary.uuid),
              }
            : undefined,
      });
    }

    if (authRejectedAt != null) {
      // CC's stored refresh_token was refused — the user MUST re-login.
      // Error-severity (not dismissable) because the stale token won't
      // self-heal and other UI surfaces (verify_all_accounts, swap)
      // will keep noisily failing until it's resolved.
      issues.push({
        id: "auth-rejected",
        severity: "error",
        label: i18n.t("status.authRejected.label"),
        detail: i18n.t("status.authRejected.detail"),
        action: onReloginActive
          ? {
              label: i18n.t("status.authRejected.action"),
              onClick: onReloginActive,
            }
          : undefined,
      });
    } else if (syncError) {
      // Key by the specific error payload so snoozing one sync failure
      // doesn't suppress a later, genuinely different failure. The
      // dismissedIssues store's 24 h expiry still applies per key.
      issues.push({
        id: `sync:${syncError}`,
        severity: "warning",
        label: i18n.t("status.syncErrorLabel"),
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
      //
      // Single pass — one toLowerCase per account, no intermediate
      // arrays. Old form did three traversals (exact filter +
      // prefix filter + length checks), each toLowerCase'ing every
      // row twice. This memo runs on every accounts/ccIdentity tick.
      const q = ccIdentity.email!.toLowerCase();
      let exactCount = 0;
      let exactHit: AccountSummary | undefined;
      let prefixCount = 0;
      let prefixHit: AccountSummary | undefined;
      for (const a of accounts) {
        const lower = a.email.toLowerCase();
        if (lower === q) {
          exactCount++;
          exactHit = a;
        } else if (lower.startsWith(q)) {
          prefixCount++;
          prefixHit = a;
        }
      }
      const target =
        exactCount === 1
          ? exactHit
          : prefixCount === 1
            ? prefixHit
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
            label: i18n.t("status.ccDrift.openMatch"),
            onClick: () => onSelectAccount(target.uuid),
          };
        }
      } else {
        if (onImportCurrent) {
          const email = ccIdentity.email;
          action = {
            label: i18n.t("status.importEmail", { email }),
            onClick: () => onImportCurrent(email),
          };
        }
        if (onReloginActive) {
          action2 = {
            label: i18n.t("status.ccDrift.relogin"),
            onClick: onReloginActive,
          };
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
        label: i18n.t("status.ccDrift.label", {
          ccEmail: ccIdentity.email,
          expectedEmail: status.cli_active_email,
        }),
        action,
        action2,
        dismissable: true,
      });
    }

    // --- Desktop-side banners (Phase 4) ---------------------------
    //
    // Driven by `sync_from_current_desktop` outcomes. Only mutating
    // affordances run on "adoption_available" (Decrypted tier); the
    // "candidate_only" variant renders an advisory banner only.
    if (desktopSync) {
      switch (desktopSync.kind) {
        case "adoption_available": {
          const email = desktopSync.email;
          issues.push({
            id: `desktop-adopt:${email.toLowerCase()}`,
            severity: "info",
            label: i18n.t("status.desktop.signedInAs", { email }),
            detail: i18n.t("status.desktop.adoptDetail"),
            action: onAdoptLiveDesktop
              ? {
                  label: i18n.t("status.desktop.bind"),
                  onClick: () => onAdoptLiveDesktop(email),
                }
              : undefined,
            dismissable: true,
          });
          break;
        }
        case "stranger": {
          const email = desktopSync.email;
          issues.push({
            id: `desktop-stranger:${email.toLowerCase()}`,
            severity: "info",
            label: i18n.t("status.desktop.signedInAs", { email }),
            detail: i18n.t("status.desktop.strangerDetail"),
            action: onImportDesktop
              ? {
                  label: i18n.t("status.importEmail", { email }),
                  onClick: () => onImportDesktop(email),
                }
              : undefined,
            dismissable: true,
          });
          break;
        }
        case "candidate_only": {
          // Unverified match (fast path only). Surface as info — the
          // slow path failed transiently (keychain not yet unlocked,
          // /profile blip, decrypt race) and the next sync cycle
          // almost always heals it. A "warning" here false-alarms on
          // every cold start. Never offer mutation: the email may be
          // wrong when multiple accounts share an org (Codex D5-1/D5-2).
          const email = desktopSync.email;
          issues.push({
            id: `desktop-candidate:${email.toLowerCase()}`,
            severity: "info",
            label: i18n.t("status.desktop.candidateLabel"),
            detail: i18n.t("status.desktop.candidateDetail", { email }),
            dismissable: true,
          });
          break;
        }
        case "verified":
        case "no_live":
          // No banner — sync is in the steady state.
          break;
      }
    }

    return issues;
  }, [
    ccIdentity,
    status,
    syncError,
    authRejectedAt,
    keychainIssue,
    accounts,
    onUnlock,
    onSelectAccount,
    onReloginActive,
    onImportCurrent,
    desktopSync,
    onAdoptLiveDesktop,
    onImportDesktop,
    language,
  ]);
}
