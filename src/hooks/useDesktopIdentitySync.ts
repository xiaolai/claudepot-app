import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { DesktopSyncOutcome } from "../types";

const DESKTOP_SYNC_TTL_MS = 5 * 60_000;

/**
 * Live Desktop identity sync, extracted from AppShell. Runs on shell
 * mount AND on window focus. Each probe costs one Keychain read +
 * one /profile HTTP call (~1s) — hammering every Alt-Tab would be
 * unfriendly, so the cadence is throttled by a last-run timestamp
 * ref mirroring useRefresh's VERIFY_TTL pattern. The 5-minute
 * cooldown is long enough that routine window-focus noise doesn't
 * trigger, but short enough that leaving Claudepot open while
 * signing into Desktop elsewhere catches the change within one
 * focus cycle.
 *
 * Returns the latest sync outcome for the status-issues banner.
 */
export function useDesktopIdentitySync(
  refreshAccounts: () => Promise<void>,
): DesktopSyncOutcome | null {
  const [desktopSync, setDesktopSync] = useState<DesktopSyncOutcome | null>(
    null,
  );
  const lastRun = useRef<number>(0);

  const runDesktopSync = useCallback(
    async (force: boolean) => {
      const now = Date.now();
      if (!force && now - lastRun.current < DESKTOP_SYNC_TTL_MS) return;
      lastRun.current = now;
      try {
        const outcome = await api.syncFromCurrentDesktop();
        setDesktopSync(outcome);
        // Verified means the backend may have pointed `active_desktop`
        // at a different account (see sync_from_current). The
        // `is_desktop_active` flags in our accounts list are now stale
        // — refresh so badges match truth without waiting for the next
        // unrelated refresh to happen to win the race.
        if (outcome.kind === "verified") {
          await refreshAccounts();
        }
      } catch {
        // Slow-path failure (keychain locked, /profile down) is not a
        // user-surfaceable error here — the banner layer already shows
        // CandidateOnly when it can. Swallow.
      }
    },
    [refreshAccounts],
  );

  useEffect(() => {
    void runDesktopSync(true); // cold-start probe runs unthrottled
  }, [runDesktopSync]);

  useEffect(() => {
    const onFocus = () => void runDesktopSync(false);
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [runDesktopSync]);

  return desktopSync;
}
