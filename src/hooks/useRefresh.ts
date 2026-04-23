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
 *  - `authRejectedAt` — timestamp of the last `sync_from_current_cc`
 *    that came back with `auth rejected:` (refresh_token dead, must
 *    re-login). The backend returns this as a distinct prefix so the
 *    status banner can render an actionable "Sign in again" state
 *    instead of a generic sync warning. Also drives a 60 s cooldown
 *    on focus-triggered syncs so window-thrashing doesn't hammer the
 *    endpoint with a token we already know is dead.
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
  const [authRejectedAt, setAuthRejectedAt] = useState<number | null>(null);
  const [ccIdentity, setCcIdentity] = useState<CcIdentity | null>(null);
  // Audit H10: `verifying` used to be a plain boolean cleared by the
  // initiating refresh's finally, guarded by generation check. If a
  // later refresh decided verification WASN'T needed, it never
  // cleared the flag, and the initiator's finally refused to clear
  // (stale gen) — the chip stuck forever.
  //
  // Fix: count concurrent verifies. Each verify start increments,
  // each finally decrements, unconditionally. `verifying` is derived
  // from count > 0 so there's no stuck state no matter how refreshes
  // overlap.
  const [verifyingCount, setVerifyingCount] = useState(0);
  const verifying = verifyingCount > 0;
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
      // Fire sync_from_current_cc in parallel with the list fetches —
      // it costs a keychain read + HTTP /profile roundtrip (~1-2 s),
      // which used to block first paint. The UI can safely render the
      // stored DB state immediately; if the sync later flips
      // active_cli or heals a credential flag, the subsequent
      // verifyAllAccounts pass picks up the delta.
      //
      // AUTH_REJECTED_COOLDOWN_MS: once the backend has told us the
      // refresh_token is dead, skip the sync call for 60 s. Every
      // window-focus would otherwise re-hit the keychain + /profile
      // to learn the same thing, and the UI banner is already telling
      // the user what to do.
      const AUTH_REJECTED_COOLDOWN_MS = 60_000;
      const nowMs = Date.now();
      const shouldSkipSync =
        authRejectedAt !== null &&
        nowMs - authRejectedAt < AUTH_REJECTED_COOLDOWN_MS;
      const syncPromise = shouldSkipSync
        ? Promise.resolve()
        : api
            .syncFromCurrentCc()
            .then(async (syncedEmail) => {
              if (gen !== refreshGenRef.current) return;
              setKeychainIssue(null);
              setSyncError(null);
              setAuthRejectedAt(null);
              // If the sync actually adopted a blob, re-pull the list
              // so the freshly-healed `has_cli_credentials` /
              // active_cli flags reach the UI without a second
              // user-triggered refresh.
              if (syncedEmail) {
                try {
                  const refreshed = await api.accountList();
                  if (gen === refreshGenRef.current) setAccounts(refreshed);
                } catch {
                  /* non-fatal — next tick picks it up */
                }
              }
            })
            .catch((e) => {
              if (gen !== refreshGenRef.current) return;
              const msg = `${e}`;
              if (msg.toLowerCase().includes("keychain is locked")) {
                setKeychainIssue(msg);
                setSyncError(null);
                setAuthRejectedAt(null);
              } else if (msg.toLowerCase().includes("auth rejected")) {
                // Terminal: refresh_token refused. Don't route to the
                // generic sync-warning banner — useStatusIssues keys
                // off authRejectedAt to render a "Sign in again" CTA.
                setKeychainIssue(null);
                setSyncError(null);
                setAuthRejectedAt(Date.now());
              } else {
                setKeychainIssue(null);
                setSyncError(msg);
                setAuthRejectedAt(null);
                // eslint-disable-next-line no-console
                console.warn("sync_from_current_cc failed:", msg);
              }
            });

      const [s, list] = await Promise.all([
        api.appStatus(),
        api.accountList(),
      ]);
      setStatus(s);
      setAccounts(list);
      setLoadError(null);

      // Wait for the sync to finish AFTER we've already painted a full
      // list — any flag/active-pointer flips it writes will show up on
      // the next refresh() tick (focus event, explicit action).
      void syncPromise;

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
        // Count-based verifying flag (H10): increment on start,
        // decrement ALWAYS in finally. The accounts-patch write still
        // gates on gen to avoid stale data overwriting fresher data,
        // but the flag itself is now a simple ref-count — can't get
        // stuck on an orphaned set.
        setVerifyingCount((n) => n + 1);
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
            setVerifyingCount((n) => Math.max(0, n - 1));
          });
      }
    } catch (e) {
      const msg = `${e}`;
      setLoadError(msg);
      pushToast("error", `refresh failed: ${msg}`);
    } finally {
      refreshingRef.current = false;
    }
  }, [pushToast, authRejectedAt]);

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
    authRejectedAt,
    ccIdentity,
    verifying,
    refresh,
  };
}
