import { emit } from "@tauri-apps/api/event";
import { useCallback, useState } from "react";
import { api } from "../../api";
import { USAGE_REFETCH_EVENT } from "../../lib/events";
import { runVerifyAll } from "./runVerifyAll";
import type { AccountSummary, VerifyOutcomeKind } from "../../types";

/** Live state of a "Verify all" run, for the in-progress indicator.
 *  `active` gates the whole indicator; `done`/`total` drive the button
 *  label; `outcomes` records the per-account result the instant it
 *  streams in, so a card flips from "verifying…" straight to its real
 *  status without waiting for the terminal refresh. */
export interface VerifyAllState {
  active: boolean;
  done: number;
  total: number;
  outcomes: Record<string, VerifyOutcomeKind>;
}

const IDLE_VERIFY: VerifyAllState = {
  active: false,
  done: 0,
  total: 0,
  outcomes: {},
};

/** Per-card verification state for the live indicator: `"verifying"`
 *  while a run is active and this account hasn't resolved yet, the
 *  streamed outcome the moment it does, or `undefined` when no run is
 *  active (the card falls back to its persisted `verify_status`). */
export type VerifyLive = "verifying" | VerifyOutcomeKind;

export function verifyLiveFor(
  v: VerifyAllState,
  uuid: string,
): VerifyLive | undefined {
  if (!v.active) return undefined;
  return v.outcomes[uuid] ?? "verifying";
}

type Push = (
  kind: "info" | "error",
  text: string,
  onUndo?: () => void,
  opts?: {
    undoMs?: number;
    undoLabel?: string;
    onCommit?: () => void;
    dedupeKey?: string;
  },
) => void;

interface Args {
  pushToast: Push;
  refresh: () => Promise<void>;
  useDesktop: (a: AccountSummary, noLaunch?: boolean) => Promise<void>;
}

/**
 * The long-tail of AccountsSection handlers — verify (single + all)
 * and the desktop-switch undo toast. The split-brain preflight now
 * lives in AppStateProvider so sidebar binds share it; this hook
 * stays lean.
 */
export function useAccountHandlers({
  pushToast,
  refresh,
  useDesktop,
}: Args) {
  const runVerifyAccount = useCallback(
    async (a: AccountSummary) => {
      try {
        const updated = await api.verifyAccount(a.uuid);
        // The backend doesn't throw on drift/rejected — it returns the
        // refreshed summary. Tone the toast to match the outcome.
        switch (updated.verify_status) {
          case "ok":
            pushToast("info", `Verified ${a.email}`);
            break;
          case "drift":
            pushToast(
              "error",
              `Drift: ${a.email} actually authenticates as ${updated.verified_email ?? "unknown"}`,
            );
            break;
          case "rejected":
            pushToast("error", `Server rejected ${a.email} — re-login required`);
            break;
          case "network_error":
            pushToast("error", `Couldn't verify ${a.email} — /profile unreachable`);
            break;
          default:
            pushToast("info", `Verified ${a.email}`);
        }
        await refresh();
        // Only a verify that actually healed the account (status now
        // "ok") should nudge usage to re-pull — that's the case where a
        // card can flip from "token expired" to live numbers. Drift /
        // rejected / network_error healed nothing, so emitting there
        // would fire a needless all-account usage fetch on every failed
        // verify. (src-tauri/src/events.rs::USAGE_REFETCH)
        if (updated.verify_status === "ok") {
          emit(USAGE_REFETCH_EVENT).catch(() => {});
        }
      } catch (e) {
        pushToast("error", `Verify failed: ${e}`);
      }
    },
    [pushToast, refresh],
  );

  const [verify, setVerify] = useState<VerifyAllState>(IDLE_VERIFY);

  const runVerifyAllHandler = useCallback(async () => {
    // Mark active up-front so the button disables + flips to
    // "Verifying…" and every card shows the pending pulse on the same
    // frame the user clicks — before the first row resolves.
    setVerify({ ...IDLE_VERIFY, active: true });
    try {
      // Row persistence still rides `refresh()` below (useRefresh owns
      // the canonical row surface). `onProgress` only feeds the live
      // indicator: per-account outcome + running tally as each streams.
      const summary = await runVerifyAll({
        patchAccount: () => {},
        setAccounts: () => {},
        onProgress: ({ uuid, outcome, done, total }) =>
          setVerify((s) => ({
            active: true,
            done,
            total,
            outcomes: { ...s.outcomes, [uuid]: outcome },
          })),
      });
      if (summary.drift + summary.rejected === 0) {
        pushToast("info", `All ${summary.total} accounts verified.`);
      } else {
        pushToast(
          "error",
          `Verify: ${summary.drift} drift, ${summary.rejected} rejected — see card banners.`,
        );
      }
      await refresh();
    } catch (e) {
      pushToast("error", `Verify-all failed: ${e}`);
    } finally {
      // Clear only after `refresh()` has landed the persisted statuses,
      // so cards read `account.verify_status` (now current) the instant
      // the live override drops — no stale flash.
      setVerify(IDLE_VERIFY);
    }
  }, [pushToast, refresh]);

  const handleDesktopSwitch = useCallback(
    (a: AccountSummary) => {
      pushToast("info", `Switching Desktop to ${a.email}…`, () => {}, {
        undoMs: 3000,
        dedupeKey: "desktop-switch",
        onCommit: () => useDesktop(a),
      });
    },
    [pushToast, useDesktop],
  );

  return {
    runVerifyAccount,
    runVerifyAll: runVerifyAllHandler,
    handleDesktopSwitch,
    verify,
  };
}
