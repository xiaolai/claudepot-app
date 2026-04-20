import { useCallback } from "react";
import { api } from "../../api";
import type { AccountSummary } from "../../types";

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
      } catch (e) {
        pushToast("error", `Verify failed: ${e}`);
      }
    },
    [pushToast, refresh],
  );

  const runVerifyAll = useCallback(async () => {
    try {
      const verified = await api.verifyAllAccounts();
      const drift = verified.filter((a) => a.verify_status === "drift").length;
      const rejected = verified.filter(
        (a) => a.verify_status === "rejected",
      ).length;
      if (drift + rejected === 0) {
        pushToast("info", `All ${verified.length} accounts verified.`);
      } else {
        pushToast(
          "error",
          `Verify: ${drift} drift, ${rejected} rejected — see card banners.`,
        );
      }
      await refresh();
    } catch (e) {
      pushToast("error", `Verify-all failed: ${e}`);
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

  return { runVerifyAccount, runVerifyAll, handleDesktopSwitch };
}
