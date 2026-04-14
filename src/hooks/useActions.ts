import { api } from "../api";
import type { AccountSummary } from "../types";

interface Deps {
  pushToast: (kind: "info" | "error", text: string) => void;
  refresh: () => Promise<void>;
  withBusy: <T>(key: string, fn: () => Promise<T>) => Promise<T>;
  addBusy: (key: string) => void;
  removeBusy: (key: string) => void;
}

export function useActions({ pushToast, refresh, withBusy, addBusy, removeBusy }: Deps) {
  const useCli = (a: AccountSummary) =>
    withBusy(`cli-${a.uuid}`, async () => {
      try {
        await api.cliUse(a.email);
        pushToast("info", `CLI switched to ${a.email}`);
        await refresh();
      } catch (e) {
        pushToast("error", `CLI switch failed: ${e}`);
      }
    });

  const login = (a: AccountSummary) =>
    withBusy(`re-${a.uuid}`, async () => {
      try {
        pushToast("info", `Opening browser — sign in as ${a.email}…`);
        await api.accountLogin(a.uuid);
        pushToast("info", `Signed in as ${a.email}`);
        await refresh();
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

  const useDesktop = (a: AccountSummary) =>
    withBusy(`desk-${a.uuid}`, async () => {
      try {
        await api.desktopUse(a.email, false);
        pushToast("info", `Desktop switched to ${a.email}`);
        await refresh();
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
    } catch (e) {
      pushToast("error", `Clear CLI failed: ${e}`);
    } finally {
      removeBusy("cli-clear");
    }
  };

  return { useCli, login, cancelLogin, useDesktop, performRemove, performClearCli };
}
