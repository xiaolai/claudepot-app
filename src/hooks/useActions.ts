import { useMemo } from "react";
import { emit } from "@tauri-apps/api/event";
import { api } from "../api";
import type { AccountSummary } from "../types";
import type { OpHandle } from "./useOperations";
import {
  LOGIN_PHASES,
  renderLoginResult,
} from "../sections/accounts/loginProgress";

/** Tell the Rust tray module to rebuild the account menu. */
const rebuildTray = () => emit("rebuild-tray-menu").catch(() => {});

interface Deps {
  pushToast: (
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
  refresh: () => Promise<void>;
  withBusy: <T>(key: string, fn: () => Promise<T>) => Promise<T>;
  /** Mount the shared op-progress modal. Wired through `useOperations`
   *  at the AppStateProvider level so callers don't need to know about
   *  React context details. */
  openOpModal: (handle: OpHandle) => void;
}

export function useActions({ pushToast, refresh, withBusy, openOpModal }: Deps) {
  // Memoize the entire returned object so its identity (and the
  // identity of every inner function) is stable across renders. The
  // four input deps are themselves stable refs (useToasts.pushToast,
  // useRefresh.refresh, useBusy.withBusy, useOperations.open), so this
  // memo only recomputes when one of them legitimately changes.
  // Without this wrapper, AppStateProvider's context value churned on
  // every render and forced every `useAppState()` consumer to
  // re-render — the dominant cold-start CPU cost.
  return useMemo(() => {
  const useCli = (a: AccountSummary, force = false) =>
    withBusy(`cli-${a.uuid}`, async () => {
      try {
        await api.cliUse(a.email, force);
        pushToast("info", `CLI switched to ${a.email}`);
        await refresh();
        rebuildTray();
      } catch (e) {
        const msg = `${e}`;
        // LiveSessionConflict from swap.rs — offer a force retry as an
        // Undo affordance on the error toast. Declining (letting the
        // toast expire) leaves the swap uncommitted; clicking Undo is
        // the explicit "retry with --force" action.
        if (msg.toLowerCase().includes("claude code process is running")) {
          pushToast(
            "error",
            `Claude Code is running — its token refresh can revert the swap. Quit CC first, or override.`,
            () => useCli(a, true),
            { undoLabel: "Override" },
          );
          return;
        }
        pushToast("error", `CLI switch failed: ${msg}`);
      }
    });

  const cancelLogin = async () => {
    try {
      await api.accountLoginCancel();
    } catch (e) {
      pushToast("error", `Cancel failed: ${e}`);
    }
  };

  const login = (a: AccountSummary) =>
    withBusy(`re-${a.uuid}`, async () => {
      try {
        // Kick off the async start. The IPC worker returns immediately
        // with an op_id; phase events flow on `op-progress::<op_id>`.
        const opId = await api.accountLoginStart(a.uuid);
        // The OperationProgressModal owns the user-visible surface.
        // We still drop a discoverable "opening browser" toast so users
        // glancing at the toast region see what's happening; the toast
        // carries the Cancel undo affordance for parity with the
        // legacy synchronous flow.
        pushToast(
          "error",
          `Opening browser — sign in as ${a.email}…`,
          cancelLogin,
          { undoLabel: "Cancel" },
        );
        openOpModal({
          opId,
          title: `Re-login: ${a.email}`,
          phases: LOGIN_PHASES,
          fetchStatus: api.accountLoginStatus,
          renderResult: renderLoginResult,
          onComplete: () => {
            pushToast("info", `Signed in as ${a.email}`);
            void refresh();
            rebuildTray();
          },
          onError: (detail) => {
            const msg = detail ?? "";
            if (msg.toLowerCase().includes("cancel")) {
              pushToast("info", "Login cancelled.");
            } else {
              pushToast("error", `Login failed: ${msg || "unknown"}`);
            }
          },
        });
      } catch (e) {
        const msg = `${e}`;
        if (msg.toLowerCase().includes("already in progress")) {
          pushToast("error", "A login is already in progress.");
        } else if (msg.toLowerCase().includes("cancelled")) {
          pushToast("info", "Login cancelled.");
        } else {
          pushToast("error", `Login failed: ${msg}`);
        }
      }
    });

  const useDesktop = (a: AccountSummary, noLaunch = false) =>
    withBusy(`desk-${a.uuid}`, async () => {
      try {
        await api.desktopUse(a.email, noLaunch);
        pushToast(
          "info",
          noLaunch
            ? `Desktop set to ${a.email} (not launched)`
            : `Desktop switched to ${a.email}`,
        );
        await refresh();
        rebuildTray();
      } catch (e) {
        pushToast("error", `Desktop switch failed: ${e}`);
      }
    });

  /// Bind the live Desktop session to `a`'s snapshot. Runs identity
  /// verification backend-side — fast-path candidates fail here with
  /// an explicit "live Desktop identity is <other>, not <email>"
  /// error, which is the correct behavior (Codex D5-1).
  ///
  /// Callers that want overwrite=true MUST gate the call behind a
  /// user-visible confirmation modal — `adoptDesktopForce` is the
  /// post-confirmation entry point; the bare `adoptDesktop` refuses
  /// to overwrite and lets the caller render the
  /// `DesktopConfirmContext::ReplaceProfile` dialog instead.
  const adoptDesktop = (a: AccountSummary) => adoptDesktopForce(a, false);

  /// Returns `true` iff the bind committed. Callers that only fire
  /// and forget can ignore the result (the toast and refresh are
  /// owned here); callers that need to sequence post-success UI
  /// (e.g. closing the Add-account modal) MUST branch on it — the
  /// action swallows errors to toast them here, so from the
  /// awaiter's perspective a rejected promise never appears.
  const adoptDesktopForce = (a: AccountSummary, overwrite: boolean): Promise<boolean> =>
    withBusy(`adopt-${a.uuid}`, async () => {
      try {
        const r = await api.desktopAdopt(a.uuid, overwrite);
        pushToast(
          "info",
          `Bound Desktop session to ${r.account_email} (${r.captured_items} item(s)).`,
        );
        await refresh();
        rebuildTray();
        return true;
      } catch (e) {
        pushToast("error", `Desktop bind failed: ${e}`);
        return false;
      }
    });

  /// Perform the actual sign-out. Destructive — the caller is
  /// responsible for having already shown the confirm dialog; this
  /// entry point assumes consent.
  const clearDesktopConfirmed = (keepSnapshot = true) =>
    withBusy("desktop-clear", async () => {
      try {
        const r = await api.desktopClear(keepSnapshot);
        const who = r.email ?? "the active session";
        pushToast(
          "info",
          r.snapshot_kept
            ? `Signed Desktop out (${who}); snapshot preserved.`
            : `Signed Desktop out (${who}); snapshot discarded.`,
        );
        await refresh();
        rebuildTray();
      } catch (e) {
        pushToast("error", `Desktop clear failed: ${e}`);
      }
    });

  /// Stub kept for backward compatibility; callers are being migrated
  /// off this and onto the confirm-then-clearDesktopConfirmed flow.
  const clearDesktop = clearDesktopConfirmed;

  const performRemoveImmediate = (a: AccountSummary) =>
    withBusy(`rm-${a.uuid}`, async () => {
      try {
        const r = await api.accountRemove(a.uuid);
        pushToast("info", `Removed ${r.email}`);
        if (r.warnings.length) {
          // Cleanup warnings are non-fatal (stale Desktop profile file,
          // etc.) — the account row was still removed successfully.
          // Use info tone so the surface matches the severity.
          pushToast("info", `Note: ${r.warnings.join(", ")}`);
        }
        await refresh();
        rebuildTray();
      } catch (e) {
        pushToast("error", `Remove failed: ${e}`);
      }
    });

  /**
   * 5s undo window before removal. The toast carries both the Undo
   * affordance and the onCommit callback — tapping Undo cancels the
   * commit; letting the toast age out triggers the actual
   * `accountRemove` call. "Undo clickable ⇔ account still exists" is
   * the invariant, shared with the desktop-switch undo pattern.
   */
  const performRemove = (a: AccountSummary) => {
    let undone = false;
    pushToast(
      "info",
      `Removing ${a.email}…`,
      () => {
        undone = true;
      },
      {
        undoMs: 5000,
        undoLabel: "Undo",
        dedupeKey: `rm-${a.uuid}`,
        onCommit: () => {
          if (undone) return;
          void performRemoveImmediate(a);
        },
      },
    );
  };

  return {
    useCli,
    login,
    cancelLogin,
    useDesktop,
    adoptDesktop,
    adoptDesktopForce,
    clearDesktop,
    clearDesktopConfirmed,
    performRemove,
  };
  }, [pushToast, refresh, withBusy, openOpModal]);
}
