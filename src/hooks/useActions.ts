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

  const login = (a: AccountSummary) =>
    withBusy(`re-${a.uuid}`, async () => {
      try {
        pushToast("info", `Opening browser — sign in as ${a.email}…`);
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

  const cancelLogin = async () => {
    try {
      await api.accountLoginCancel();
    } catch (e) {
      pushToast("error", `Cancel failed: ${e}`);
    }
  };

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

  const performRemove = (a: AccountSummary) =>
    withBusy(`rm-${a.uuid}`, async () => {
      try {
        const r = await api.accountRemove(a.uuid);
        pushToast("info", `Removed ${r.email}`);
        if (r.warnings.length)
          pushToast("error", `warnings: ${r.warnings.join(", ")}`);
        await refresh();
        rebuildTray();
      } catch (e) {
        pushToast("error", `remove failed: ${e}`);
      }
    });

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

  return { useCli, login, cancelLogin, useDesktop, performRemove, performClearCli };
}
