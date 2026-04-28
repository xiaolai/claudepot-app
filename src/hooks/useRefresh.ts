import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import { runVerifyAll } from "../sections/accounts/runVerifyAll";
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
  // Audit T4-3: a refresh kicked off while another is in flight used
  // to be silently dropped by `if (refreshingRef.current) return`.
  // Mutation-triggered refreshes (e.g. after `account_use_cli`) that
  // landed during the cold-start refresh would never run, leaving
  // the UI staring at stale flags. Track a `pending` bit instead:
  // late callers set it, the active refresh notices in its finally
  // and fires exactly one follow-up. Coalesces a burst of N calls
  // into 2 actual refreshes, never zero.
  const pendingRefreshRef = useRef(false);
  // Generation token — every call to refresh() bumps this. Background
  // callbacks (currentCcIdentity, verifyAllAccounts) capture the gen
  // that started them and check before calling setState; a later
  // refresh that kicked off newer work will have incremented the gen,
  // so the older promise's late resolution is silently ignored.
  const refreshGenRef = useRef(0);
  // Mirror authRejectedAt into a ref so the refresh callback can
  // observe the latest value without re-memoizing on every change.
  // Without this, focus-driven refresh calls captured a stale value
  // and could re-enter the cooldown window after another caller had
  // cleared it (or vice versa).
  const authRejectedAtRef = useRef<number | null>(null);
  useEffect(() => {
    authRejectedAtRef.current = authRejectedAt;
  }, [authRejectedAt]);
  // Pending requestIdleCallback handle for the deferred runVerifyAll
  // dispatch. Stored so the surrounding effect's cleanup (and any
  // refresh that supersedes a still-queued idle callback) can cancel
  // it cleanly instead of letting the rIC fire after teardown and
  // hit dead state setters.
  const verifyIdleRef = useRef<number | null>(null);

  const refresh = useCallback(async () => {
    if (refreshingRef.current) {
      // A refresh is already underway — flag the request so the
      // currently-running pass triggers a follow-up in its finally.
      pendingRefreshRef.current = true;
      return;
    }
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
      const cooldownStart = authRejectedAtRef.current;
      const shouldSkipSync =
        cooldownStart !== null &&
        nowMs - cooldownStart < AUTH_REJECTED_COOLDOWN_MS;
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
        //
        // Defer the verify burst past first paint. `verify_all_accounts`
        // fans out one HTTPS `/profile` round-trip per account in
        // parallel; nothing the user sees on cold start depends on the
        // result, so paying for it on the boot critical path just
        // delays ambient network for the same staleness gate that fires
        // on the next focus event.
        const rIC: (cb: () => void) => number =
          (window as typeof window & {
            requestIdleCallback?: (cb: () => void) => number;
          }).requestIdleCallback ?? ((cb) => window.setTimeout(cb, 0));
        const cIC: (h: number) => void =
          (window as typeof window & {
            cancelIdleCallback?: (h: number) => void;
          }).cancelIdleCallback ?? window.clearTimeout;
        // Cancel any prior pending verify-rIC: a refresh that fires
        // while one is still queued would otherwise run two verifies
        // back-to-back (the queued one against stale data, the new
        // one against fresh).
        if (verifyIdleRef.current !== null) {
          cIC(verifyIdleRef.current);
          verifyIdleRef.current = null;
        }
        verifyIdleRef.current = rIC(() => {
          verifyIdleRef.current = null;
          setVerifyingCount((n) => n + 1);
          runVerifyAll({
            patchAccount: (uuid, patch) => {
              if (gen !== refreshGenRef.current) return;
              setAccounts((prev) =>
                prev.map((a) => (a.uuid === uuid ? { ...a, ...patch } : a)),
              );
            },
            setAccounts: (rows) => {
              if (gen === refreshGenRef.current) setAccounts(rows);
            },
          })
            .catch((e) => {
              // eslint-disable-next-line no-console
              console.warn("verify_all_accounts failed:", e);
            })
            .finally(() => {
              setVerifyingCount((n) => Math.max(0, n - 1));
            });
        });
      }
    } catch (e) {
      const msg = `${e}`;
      setLoadError(msg);
      pushToast("error", `refresh failed: ${msg}`);
    } finally {
      refreshingRef.current = false;
      // Drain the pending bit. If anyone called `refresh()` while
      // we were running, kick a single follow-up. Doing it here —
      // not in a `while` loop — keeps the function bounded: more
      // requests landing during the follow-up coalesce into one
      // more pass, and the chain ends naturally when no caller
      // arrives during the active run.
      if (pendingRefreshRef.current) {
        pendingRefreshRef.current = false;
        // Schedule on a microtask so the current promise's `.then`
        // chain unwinds before the next refresh starts; otherwise
        // React state updates from this pass would batch with the
        // next pass's setState calls in unpredictable ways.
        queueMicrotask(() => {
          void refresh();
        });
      }
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
    return () => {
      window.removeEventListener("focus", onFocus);
      // Drop any still-queued deferred verify so it can't fire after
      // teardown and call setVerifyingCount on a dead component.
      if (verifyIdleRef.current !== null) {
        const cIC: (h: number) => void =
          (window as typeof window & {
            cancelIdleCallback?: (h: number) => void;
          }).cancelIdleCallback ?? window.clearTimeout;
        cIC(verifyIdleRef.current);
        verifyIdleRef.current = null;
      }
    };
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
