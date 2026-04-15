import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { AccountSummary, AppStatus, CcIdentity } from "../types";

/**
 * Central refresh hook. Exposes:
 *  - `status` / `accounts` / `loadError` — the primary render state
 *  - `keychainIssue` — surfaced when macOS login keychain is locked
 *  - `syncError` — populated when `sync_from_current_cc` fails with
 *    something other than a keychain lock (e.g. 401 on CC's blob). The
 *    old code only `console.warn`d; now the banner is user-visible.
 *  - `ccIdentity` — ground-truth who CC is signed in as, fetched alongside
 *    each refresh so the top-of-window truth strip can render reality
 *    instead of just what the DB believes.
 *  - `verifying` — true while `verify_all_accounts` is in flight so the
 *    sidebar can show a "Reconciling…" chip and the user knows the
 *    green dots they see are being cross-checked against /profile.
 */
export function useRefresh(pushToast: (kind: "info" | "error", text: string) => void) {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [keychainIssue, setKeychainIssue] = useState<string | null>(null);
  const [syncError, setSyncError] = useState<string | null>(null);
  const [ccIdentity, setCcIdentity] = useState<CcIdentity | null>(null);
  const [verifying, setVerifying] = useState(false);
  const lastRefreshRef = useRef(0);
  const refreshingRef = useRef(false);
  // Generation token — every call to refresh() bumps this. Background
  // callbacks (currentCcIdentity, verifyAllAccounts) capture the gen
  // that started them and check before calling setState; a later
  // refresh that kicked off newer work will have incremented the gen,
  // so the older promise's late resolution is silently ignored.
  const refreshGenRef = useRef(0);

  const refresh = useCallback(async () => {
    if (refreshingRef.current) return;
    refreshingRef.current = true;
    lastRefreshRef.current = Date.now();
    refreshGenRef.current += 1;
    const gen = refreshGenRef.current;
    try {
      try {
        await api.syncFromCurrentCc();
        setKeychainIssue(null);
        setSyncError(null);
      } catch (e) {
        const msg = `${e}`;
        if (msg.toLowerCase().includes("keychain is locked")) {
          setKeychainIssue(msg);
          setSyncError(null);
        } else {
          setKeychainIssue(null);
          // Surface this to the UI instead of just logging — silent sync
          // failures were how drift used to hide from users. The banner
          // tells them Claudepot's active_cli may be stale and offers
          // context so they can decide what to do.
          setSyncError(msg);
          // eslint-disable-next-line no-console
          console.warn("sync_from_current_cc failed:", msg);
        }
      }
      const [s, list] = await Promise.all([
        api.appStatus(),
        api.accountList(),
      ]);
      setStatus(s);
      setAccounts(list);
      setLoadError(null);

      // Ground-truth CC identity + background reconciliation run in
      // parallel — both are slow (network), both are decorative next to
      // the DB render that just completed, so we don't block. Each
      // callback checks `gen === refreshGenRef.current` before calling
      // setState — if a newer refresh() has started since, we drop the
      // stale result so it can't overwrite fresher state.
      api
        .currentCcIdentity()
        .then((identity) => {
          if (gen === refreshGenRef.current) setCcIdentity(identity);
        })
        .catch((e) => {
          // eslint-disable-next-line no-console
          console.warn("current_cc_identity failed:", e);
        });

      // Staleness gate on the expensive reconciliation: skip
      // verify_all_accounts if every account was verified within the
      // VERIFY_TTL window. Focus events hammer the endpoint otherwise —
      // with N accounts and a 1s /profile round-trip, 4 window-focus
      // events in a minute cost 4N network calls when only the first
      // one produced new information.
      const now = Date.now();
      const VERIFY_TTL_MS = 60_000;
      const needsVerify = list.some((a) => {
        if (!a.verified_at) return true;
        const age = now - new Date(a.verified_at).getTime();
        return age >= VERIFY_TTL_MS || a.verify_status === "never";
      });
      if (needsVerify) {
        setVerifying(true);
        api
          .verifyAllAccounts()
          .then((verified) => {
            if (gen === refreshGenRef.current) setAccounts(verified);
          })
          .catch((e) => {
            // eslint-disable-next-line no-console
            console.warn("verify_all_accounts failed:", e);
          })
          .finally(() => {
            // Only clear `verifying` if this IS the latest refresh. An
            // older pass finishing shouldn't remove the chip while a
            // newer one is still in flight.
            if (gen === refreshGenRef.current) setVerifying(false);
          });
      }
    } catch (e) {
      const msg = `${e}`;
      setLoadError(msg);
      pushToast("error", `refresh failed: ${msg}`);
    } finally {
      refreshingRef.current = false;
    }
  }, [pushToast]);

  useEffect(() => {
    refresh();
    const onFocus = () => {
      if (Date.now() - lastRefreshRef.current > 2000) {
        refresh();
      }
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [refresh]);

  return {
    status,
    accounts,
    loadError,
    keychainIssue,
    syncError,
    ccIdentity,
    verifying,
    refresh,
  };
}
