import { enabledSectionIds } from "../lib/optionalSections";
import { api } from "../api";
import { i18n } from "../lib/i18n";
import { toastError } from "../lib/i18n-error";
import { triggerSettingsTab } from "../lib/networkPanelDeepLink";
import { decodeDestination } from "../lib/destination";
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
      // `app-menu:nav:<section>:<subtab>`. The OS menu API carries a
      // string and nothing else, which is why this transport exists at
      // all — but the PARSING is now shared with every other one
      // (`lib/destination`), so a level the menu can express cannot be
      // a level the receiver silently drops. Tray Health uses this form
      // to land on Settings → Health.
      const dest = decodeDestination(cmd.substring("app-menu:nav:".length));
      if (dest && enabledSectionIds().includes(dest.section)) {
        setSection(dest.section);
        if (dest.section === "settings" && dest.tab) {
          // Settings panes reach their target through the cold-mount
          // sessionStorage hint plus a hot-mount event; keeping that in
          // one helper is why this is not a plain setSection subroute.
          triggerSettingsTab(dest.tab);
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
