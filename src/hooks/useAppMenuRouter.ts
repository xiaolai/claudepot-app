import { enabledSectionIds } from "../lib/optionalSections";
import { api } from "../api";
import { i18n } from "../lib/i18n";
import { toastError } from "../lib/i18n-error";
import { triggerSettingsTab } from "../lib/networkPanelDeepLink";
import { useTauriEvent } from "./useTauriEvent";

/**
 * App menu bar + tray menu both emit `app-menu` with a string id as
 * payload. Routing lives at shell level (not in a section) because
 * nav items need the shell-level setSection. Action items delegate
 * to the section via window events to avoid entangling state trees.
 *
 * The subscription is registered once for the shell's lifetime —
 * useTauriEvent holds the handler in a ref, so the unstable arg
 * identities never re-wire the channel.
 */
export function useAppMenuRouter(args: {
  setSection: (id: string) => void;
  toggleTheme: () => void;
  refreshAccounts: () => Promise<void>;
  pushToast: (kind: "info" | "error", text: string) => void;
}): void {
  const { setSection, toggleTheme, refreshAccounts, pushToast } = args;

  useTauriEvent<string>("app-menu", (event) => {
    const cmd = event.payload;
    if (cmd.startsWith("app-menu:nav:")) {
      // Format: `app-menu:nav:<section>` or
      // `app-menu:nav:<section>:<subtab>`. The optional subtab
      // applies to sections that expose subtabs internally (today
      // only Settings); routing it via `triggerSettingsTab` keeps
      // the cold/hot-mount machinery in one place. Tray Health
      // entry uses this form to land on Settings → Health.
      const parts = cmd.substring("app-menu:nav:".length).split(":");
      const section = parts[0];
      if (section && enabledSectionIds().includes(section)) {
        setSection(section);
        const subtab = parts[1];
        if (section === "settings" && subtab) {
          triggerSettingsTab(subtab);
        }
      }
      return;
    }
    if (cmd === "app-menu:view:toggle-theme") {
      toggleTheme();
      return;
    }
    if (cmd === "app-menu:view:reload") {
      void refreshAccounts();
      return;
    }
    if (cmd === "app-menu:account:login-browser") {
      setSection("accounts");
      window.dispatchEvent(new CustomEvent("cp-open-add"));
      return;
    }
    if (cmd === "app-menu:account:sync-cc") {
      api
        .syncFromCurrentCc()
        .then((email) =>
          pushToast(
            "info",
            email
              ? i18n.t("menu.synced", { email })
              : i18n.t("menu.nothingToSync"),
          ),
        )
        .catch((e) => toastError(pushToast, i18n.t("menu.syncFailed"), e));
      return;
    }
    if (cmd === "app-menu:account:verify-all") {
      api
        .verifyAllAccounts()
        .then(() => {
          pushToast("info", i18n.t("menu.verifyAllComplete"));
          void refreshAccounts();
        })
        .catch((e) => toastError(pushToast, i18n.t("menu.verifyFailed"), e));
      return;
    }
    if (cmd === "app-menu:help:copy-diag") {
      setSection("settings");
      pushToast("info", i18n.t("menu.copyDiagHint"));
      return;
    }
  });
}
