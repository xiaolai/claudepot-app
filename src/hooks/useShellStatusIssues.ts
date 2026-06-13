import { useCallback, useMemo } from "react";
import { api } from "../api";
import { toastError } from "../lib/toastError";
import { useAppState } from "../providers/AppStateProvider";
import { useStatusIssues, type StatusIssue } from "./useStatusIssues";
import { useStatusBannerEmits } from "./useStatusBannerEmits";
import { useSnoozeAutoClear } from "./useSnoozeAutoClear";
import { useDesktopIdentitySync } from "./useDesktopIdentitySync";

/**
 * Shell status-issues pipeline, extracted from AppShell. Owns:
 *
 *   - the action callbacks the banner rows invoke (unlock keychain,
 *     re-login active, import CC's current login, adopt/import the
 *     live Desktop identity, focus an account row)
 *   - the live Desktop identity sync probe feeding the banner
 *   - issue derivation (useStatusIssues) + snooze filtering
 *   - the Phase 5 banner state machine (useStatusBannerEmits)
 *   - snooze auto-clear when an issue's condition resolves
 *
 * Reads account/app state straight from AppStateProvider; the only
 * shell dependency is `setSection` for the deep-link actions.
 */
export function useShellStatusIssues(setSection: (id: string) => void): {
  visibleIssues: StatusIssue[];
  dismiss: (id: string) => void;
} {
  const {
    accounts,
    status: appStatus,
    ccIdentity,
    syncError,
    authRejectedAt,
    keychainIssue,
    refresh: refreshAccounts,
    pushToast,
    isDismissed,
    dismiss,
    clearDismissed,
    knownDismissedKeys,
    actions,
    requestDesktopOverwrite,
  } = useAppState();

  // Live Desktop identity sync (mount + focus, 5-min throttle).
  const desktopSync = useDesktopIdentitySync(refreshAccounts);

  // Jump to Accounts and pass the UUID so the section can scroll to
  // (or highlight) the flagged row. Deep-link plumbing is currently a
  // section jump — AccountsSection subscribes to this event and owns
  // the row-level focus.
  const onSelectAccount = useCallback(
    (uuid: string) => {
      setSection("accounts");
      window.dispatchEvent(
        new CustomEvent("cp-focus-account", { detail: uuid }),
      );
    },
    [setSection],
  );

  const onUnlockKeychain = useCallback(async () => {
    try {
      await api.unlockKeychain();
      await refreshAccounts();
    } catch (e) {
      toastError(pushToast, "Unlock failed", e);
    }
  }, [pushToast, refreshAccounts]);

  const onReloginActive = useCallback(() => {
    const active = accounts.find((a) => a.is_cli_active);
    if (!active) {
      pushToast("error", "No active CLI account to re-login.");
      return;
    }
    // Shared login helper owns the busy keyring, cancel affordance,
    // and tray refresh — no reason to re-implement it inline.
    void actions.login(active);
  }, [accounts, actions, pushToast]);

  // Adopt CC's currently-authenticated login as a new Claudepot
  // account. Surfaced by the CC-slot-drift banner when the drifted
  // email isn't already registered — saves a Sign-out → Add → OAuth
  // round-trip because the credential already exists.
  const onImportCurrent = useCallback(
    async (email: string) => {
      try {
        const outcome = await api.accountAddFromCurrent();
        pushToast("info", `Imported ${outcome.email}`);
        await refreshAccounts();
      } catch (e) {
        toastError(pushToast, "Import failed", e);
      }
      // `email` is intentionally unused — supplied by the hook so the
      // button label can show the address, but the backend reads CC's
      // current state directly.
      void email;
    },
    [pushToast, refreshAccounts],
  );

  const onAdoptLiveDesktop = useCallback(
    (email: string) => {
      const match = accounts.find(
        (a) => a.email.toLowerCase() === email.toLowerCase(),
      );
      if (!match) {
        pushToast("error", `No registered account matches ${email}.`);
        return;
      }
      // Banner action never implicitly overwrites — if the match
      // already has a snapshot, route through the shell-level
      // confirm modal so the user opts in to replacing it.
      if (match.desktop_profile_on_disk) requestDesktopOverwrite(match);
      else void actions.adoptDesktop(match);
    },
    [accounts, actions, pushToast, requestDesktopOverwrite],
  );

  const onImportDesktop = useCallback(
    (email: string) => {
      // Same as CC-side Import: jump to Accounts + open Add modal.
      // The user completes browser login there, then triggers adopt.
      setSection("accounts");
      window.dispatchEvent(new CustomEvent("cp-open-add"));
      void email; // label-only; the modal reads live state itself.
    },
    [setSection],
  );

  const rawIssues = useStatusIssues({
    ccIdentity,
    status: appStatus,
    syncError,
    authRejectedAt,
    keychainIssue,
    accounts,
    onUnlock: onUnlockKeychain,
    onSelectAccount,
    onReloginActive,
    onImportCurrent,
    desktopSync,
    onAdoptLiveDesktop,
    onImportDesktop,
  });
  const visibleIssues = useMemo(
    () => rawIssues.filter((i) => !(i.dismissable && isDismissed(i.id))),
    [rawIssues, isDismissed],
  );

  // Phase 5: banner state machine. Watches the visible-issues list
  // and emits a routing event when an id appears (P0 category per
  // banner type) or leaves (P2 bannerResolved). Memory-only state;
  // see useStatusBannerEmits header for the design rationale.
  useStatusBannerEmits(visibleIssues);

  // Drop snoozes for issues whose underlying condition resolved.
  useSnoozeAutoClear({ rawIssues, clearDismissed, knownDismissedKeys });

  return { visibleIssues, dismiss };
}
