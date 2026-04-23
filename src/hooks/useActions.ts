import { emit } from "@tauri-apps/api/event";
import { api } from "../api";
import type { AccountSummary } from "../types";

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
  addBusy: (key: string) => void;
  removeBusy: (key: string) => void;
}

export function useActions({ pushToast, refresh, withBusy, addBusy, removeBusy }: Deps) {
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
        // The "Opening browser…" toast is error-tone because the login
        // subprocess is long-running and we want the Cancel affordance
        // (Undo button) to stay visible until the user clicks it OR
        // the subprocess terminates. Error toasts don't auto-dismiss.
        pushToast(
          "error",
          `Opening browser — sign in as ${a.email}…`,
          cancelLogin,
          { undoLabel: "Cancel" },
        );
        await api.accountLogin(a.uuid);
        pushToast("info", `Signed in as ${a.email}`);
        await refresh();
        rebuildTray();
      } catch (e) {
        const msg = `${e}`;
        if (msg.toLowerCase().includes("cancelled")) {
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
  const adoptDesktop = (a: AccountSummary, overwrite = false) =>
    withBusy(`adopt-${a.uuid}`, async () => {
      try {
        const r = await api.desktopAdopt(a.uuid, overwrite);
        pushToast(
          "info",
          `Bound Desktop session to ${r.account_email} (${r.captured_items} item(s)).`,
        );
        await refresh();
        rebuildTray();
      } catch (e) {
        pushToast("error", `Desktop bind failed: ${e}`);
      }
    });

  const clearDesktop = (keepSnapshot = true) =>
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

  const performClearCli = async () => {
    addBusy("cli-clear");
    try {
      await api.cliClear();
      pushToast("info", "CLI signed out.");
      await refresh();
      rebuildTray();
    } catch (e) {
      pushToast("error", `Clear CLI failed: ${e}`);
    } finally {
      removeBusy("cli-clear");
    }
  };

  return {
    useCli,
    login,
    cancelLogin,
    useDesktop,
    adoptDesktop,
    clearDesktop,
    performRemove,
    performClearCli,
  };
}
