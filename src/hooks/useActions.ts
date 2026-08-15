import { useMemo } from "react";
import { emit } from "@tauri-apps/api/event";
import { api } from "../api";
import { i18n } from "../lib/i18n";
import { renderError } from "../lib/i18n-error";
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
      /** See `useToasts`: manual close cancels instead of committing.
       *  For destructive deferred actions only. */
      cancelOnDismiss?: boolean;
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
        pushToast("info", i18n.t("account.cliSwitched", { email: a.email }));
        await refresh();
        rebuildTray();
      } catch (e) {
        const msg = renderError(e);
        // LiveSessionConflict from swap.rs — offer a force retry as an
        // Undo affordance on the error toast. Declining (letting the
        // toast expire) leaves the swap uncommitted; clicking Undo is
        // the explicit "retry with --force" action.
        if (msg.toLowerCase().includes("claude code process is running")) {
          pushToast(
            "error",
            i18n.t("account.ccRunningOverride"),
            () => useCli(a, true),
            { undoLabel: i18n.t("account.override") },
          );
          return;
        }
        pushToast("error", renderError(e, i18n.t("account.cliSwitchFailed")));
      }
    });

  const cancelLogin = async () => {
    try {
      await api.accountLoginCancel();
    } catch (e) {
      pushToast("error", renderError(e, i18n.t("account.cancelFailed")));
    }
  };

  const login = (a: AccountSummary) =>
    withBusy(`re-${a.uuid}`, async () => {
      try {
        // Kick off the async start. The IPC worker returns immediately
        // with an op_id; phase events flow on `op-progress::<op_id>`.
        const opId = await api.accountLoginStart(a.uuid);
        // The OperationProgressModal owns the user-visible surface and
        // carries the canonical Cancel button via `onCancel`. A short
        // info toast tells the user the browser is opening so the
        // attention shift to the browser tab is expected.
        pushToast(
          "info",
          i18n.t("account.openingBrowser", { email: a.email }),
        );
        openOpModal({
          opId,
          title: i18n.t("account.reloginTitle", { email: a.email }),
          phases: LOGIN_PHASES,
          fetchStatus: api.accountLoginStatus,
          renderResult: renderLoginResult,
          onCancel: cancelLogin,
          cancelLabel: i18n.t("account.cancelLogin"),
          onComplete: () => {
            pushToast("info", i18n.t("account.signedIn", { email: a.email }));
            void refresh();
            rebuildTray();
          },
          onError: (detail) => {
            const msg = detail ?? "";
            if (msg.toLowerCase().includes("cancel")) {
              pushToast("info", i18n.t("account.loginCancelled"));
            } else {
              pushToast(
                "error",
                renderError(
                  msg || i18n.t("ui.unknown"),
                  i18n.t("account.loginFailed"),
                ),
              );
            }
          },
        });
      } catch (e) {
        const msg = renderError(e);
        if (msg.toLowerCase().includes("already in progress")) {
          pushToast("error", i18n.t("account.loginInProgress"));
        } else if (msg.toLowerCase().includes("cancelled")) {
          pushToast("info", i18n.t("account.loginCancelled"));
        } else {
          pushToast("error", renderError(e, i18n.t("account.loginFailed")));
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
            ? i18n.t("account.desktopSetNoLaunch", { email: a.email })
            : i18n.t("account.desktopSwitched", { email: a.email }),
        );
        await refresh();
        rebuildTray();
      } catch (e) {
        pushToast(
          "error",
          renderError(e, i18n.t("account.desktopSwitchFailed")),
        );
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
          i18n.t("account.desktopBound", {
            email: r.account_email,
            items: r.captured_items,
          }),
        );
        await refresh();
        rebuildTray();
        return true;
      } catch (e) {
        pushToast("error", renderError(e, i18n.t("account.desktopBindFailed")));
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
        const who = r.email ?? i18n.t("account.activeSessionFallback");
        pushToast(
          "info",
          r.snapshot_kept
            ? i18n.t("account.desktopSignedOutKept", { who })
            : i18n.t("account.desktopSignedOutDiscarded", { who }),
        );
        await refresh();
        rebuildTray();
      } catch (e) {
        pushToast(
          "error",
          renderError(e, i18n.t("account.desktopClearFailed")),
        );
      }
    });

  /// Stub kept for backward compatibility; callers are being migrated
  /// off this and onto the confirm-then-clearDesktopConfirmed flow.
  const clearDesktop = clearDesktopConfirmed;

  const performRemoveImmediate = (a: AccountSummary) =>
    withBusy(`rm-${a.uuid}`, async () => {
      try {
        const r = await api.accountRemove(a.uuid);
        pushToast("info", i18n.t("account.removed", { email: r.email }));
        if (r.warnings.length) {
          // Cleanup warnings are non-fatal (stale Desktop profile file,
          // etc.) — the account row was still removed successfully.
          // Use info tone so the surface matches the severity.
          pushToast(
            "info",
            i18n.t("account.removeWarnings", {
              warnings: r.warnings.join(", "),
            }),
          );
        }
        await refresh();
        rebuildTray();
      } catch (e) {
        pushToast("error", renderError(e, i18n.t("account.removeFailed")));
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
      i18n.t("account.removing", { email: a.email }),
      () => {
        undone = true;
      },
      {
        undoMs: 5000,
        undoLabel: i18n.t("ui.undo"),
        dedupeKey: `rm-${a.uuid}`,
        // Closing this toast CANCELS the removal rather than
        // committing it. The default (commit on any dismissal) is
        // right for a reversible deferred action like a Desktop
        // switch; here it meant a user who had just been promised
        // "a few seconds to undo" destroyed the account the instant
        // they tidied the toast away — dismissing was indistinguishable
        // from confirming, for the app's most destructive action.
        cancelOnDismiss: true,
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
