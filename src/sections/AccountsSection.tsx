import { useCallback, useEffect, useMemo, useState } from "react";
import type { AccountSummary } from "../types";
import { api } from "../api";
import { useUsage } from "../hooks/useUsage";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { useGlobalShortcuts } from "../hooks/useGlobalShortcuts";
import { useCompactHeader } from "../hooks/useWindowWidth";
import { useAppState } from "../providers/AppStateProvider";
import { Button } from "../components/primitives/Button";
import { IconButton } from "../components/primitives/IconButton";
import { NF } from "../icons";
import { ScreenHeader } from "../shell/ScreenHeader";
import { AccountsGrid } from "./accounts/AccountsGrid";
import { AddAccountModal } from "./accounts/AddAccountModal";
import { HealthChips } from "./accounts/HealthChips";
import { CtxMenuForAccount } from "./accounts/useAccountContextMenu";
import { useAccountHandlers } from "./accounts/useAccountHandlers";
import type {
  CliTargetHandlers,
  DesktopTargetHandlers,
} from "./accounts/targetButtonStates";

/**
 * Accounts section. Renders the header, filter bar, and the card grid.
 * Refresh/toast state is lifted to `AppStateProvider` — the shell-level
 * `StatusIssuesBanner` and this section share the same `/profile` and
 * `verify_all_accounts` traffic off one useRefresh instance. Per-view
 * state (usage cache, busy keys, modals, palette) stays local.
 */
export function AccountsSection({
  onNavigate: _onNavigate,
}: {
  onNavigate?: (section: string, subRoute?: string | null) => void;
}) {
  const {
    pushToast,
    status,
    accounts,
    loadError,
    refresh,
    actions,
    busyKeys,
    requestCliSwap,
    requestRemoveAccount,
    requestDesktopOverwrite,
  } = useAppState();
  const { usage, refreshUsage, refreshUsageFor } = useUsage();
  const compact = useCompactHeader();

  const [showAdd, setShowAdd] = useState(false);
  const [filter, setFilter] = useState("");
  const [ctxMenu, setCtxMenu] = useState<
    | { kind: "row"; x: number; y: number; account: AccountSummary }
    | null
  >(null);

  const { runVerifyAccount, runVerifyAll, handleDesktopSwitch } =
    useAccountHandlers({
      pushToast,
      refresh,
      useDesktop: actions.useDesktop,
    });

  const handleContextMenu = useCallback(
    (e: React.MouseEvent, a: AccountSummary) => {
      e.preventDefault();
      setCtxMenu({ kind: "row", x: e.clientX, y: e.clientY, account: a });
    },
    [],
  );

  const closeCtxMenu = useCallback(() => setCtxMenu(null), []);

  // Cmd+Shift+C — copy first matching email when a filter is active,
  // else the first account in the list.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.shiftKey && e.key === "c") {
        e.preventDefault();
        const target = shown[0];
        if (!target) return;
        void navigator.clipboard
          .writeText(target.email)
          .then(() => pushToast("info", `Copied ${target.email}`))
          .catch((err) => pushToast("error", `Copy failed: ${err}`));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // `shown` is computed below — tracked by accounts/filter deps.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accounts, filter, pushToast]);

  useGlobalShortcuts({
    onRefresh: () => {
      refresh();
      refreshUsage();
    },
    onAdd: () => setShowAdd(true),
  });

  // Add-account bridge — the macOS app menu and the tray menu both
  // dispatch this to open the AddAccountModal from outside the section.
  useEffect(() => {
    const onOpen = () => setShowAdd(true);
    window.addEventListener("cp-open-add", onOpen);
    return () => window.removeEventListener("cp-open-add", onOpen);
  }, []);

  // Shell-level drift banners deep-link into a specific account via
  // `cp-focus-account`. The CustomEvent payload is the target UUID;
  // we find the matching card by data attribute and bring it into view.
  useEffect(() => {
    const onFocus = (e: Event) => {
      const uuid = (e as CustomEvent<string>).detail;
      if (!uuid) return;
      // Clear any filter that would hide the target row so the scroll
      // target is actually mounted.
      setFilter("");
      requestAnimationFrame(() => {
        const el = document.querySelector<HTMLElement>(
          `[data-account-uuid="${uuid}"]`,
        );
        el?.scrollIntoView({ block: "center", behavior: "smooth" });
      });
    };
    window.addEventListener("cp-focus-account", onFocus);
    return () => window.removeEventListener("cp-focus-account", onFocus);
  }, []);

  const trayRefresh = useCallback(() => {
    refresh();
    refreshUsage();
  }, [refresh, refreshUsage]);
  useTauriEvent("tray-cli-switched", trayRefresh);
  useTauriEvent("tray-refresh-requested", trayRefresh);
  useTauriEvent<string>("tray-cli-switch-failed", (e) => {
    const detail = typeof e?.payload === "string" ? e.payload : "unknown";
    pushToast("error", `Tray switch failed: ${detail}`);
  });

  const shown = useMemo(() => {
    if (!filter.trim()) return accounts;
    const q = filter.toLowerCase();
    return accounts.filter(
      (a) =>
        a.email.toLowerCase().includes(q) ||
        a.org_name?.toLowerCase().includes(q),
    );
  }, [accounts, filter]);

  // Handler bags routed into each AccountCard's TargetButtons. The
  // adopt path still branches on `desktop_profile_on_disk` the same
  // way CtxMenuForAccount does — when a snapshot already exists the
  // shell-level overwrite confirm owns the decision.
  const cliHandlers: CliTargetHandlers = useMemo(
    () => ({
      switchCli: (a) => requestCliSwap(a),
      verify: (a) => runVerifyAccount(a),
      login: (a) => actions.login(a),
    }),
    [requestCliSwap, runVerifyAccount, actions],
  );
  const desktopHandlers: DesktopTargetHandlers = useMemo(
    () => ({
      switchDesktop: (a) => handleDesktopSwitch(a),
      switchDesktopNoLaunch: (a) => void actions.useDesktop(a, true),
      launchDesktop: () => {
        api.desktopLaunch().catch((e) => {
          const msg = e instanceof Error ? e.message : String(e);
          pushToast("error", `Desktop launch failed: ${msg}`);
        });
      },
      adoptDesktop: (a) => {
        if (a.desktop_profile_on_disk) requestDesktopOverwrite(a);
        else void actions.adoptDesktop(a);
      },
    }),
    [handleDesktopSwitch, actions, requestDesktopOverwrite, pushToast],
  );

  if (!status) {
    if (loadError) {
      return (
        <div
          style={{
            padding: "var(--sp-48)",
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            gap: "var(--sp-12)",
          }}
        >
          <h2
            style={{
              fontSize: "var(--fs-lg)",
              fontWeight: 600,
              color: "var(--fg)",
              margin: 0,
            }}
          >
            Couldn't load Claudepot
          </h2>
          <p
            style={{
              color: "var(--fg-muted)",
              fontSize: "var(--fs-xs)",
              margin: 0,
            }}
          >
            {loadError}
          </p>
          <Button variant="solid" onClick={() => refresh()}>
            Retry
          </Button>
        </div>
      );
    }
    return (
      <div
        style={{
          padding: "var(--sp-48)",
          color: "var(--fg-muted)",
          fontSize: "var(--fs-sm)",
        }}
      >
        Loading accounts…
      </div>
    );
  }

  return (
    <>
      <ScreenHeader
        title="Accounts"
        subtitle={<HealthChips accounts={accounts} />}
        actions={
          <>
            {compact ? (
              <>
                <IconButton
                  glyph={NF.shield}
                  onClick={runVerifyAll}
                  title="Verify all — check every account against /profile"
                  aria-label="Verify all accounts"
                />
                <IconButton
                  glyph={NF.refresh}
                  onClick={() => {
                    refresh();
                    refreshUsage();
                  }}
                  title="Refresh usage (⌘R)"
                  aria-label="Refresh usage"
                />
              </>
            ) : (
              <>
                <Button
                  variant="ghost"
                  glyph={NF.shield}
                  glyphColor="var(--fg-muted)"
                  onClick={runVerifyAll}
                  title="Verify every account against /profile"
                >
                  Verify all
                </Button>
                <Button
                  variant="ghost"
                  glyph={NF.refresh}
                  glyphColor="var(--fg-muted)"
                  onClick={() => {
                    refresh();
                    refreshUsage();
                  }}
                  title="Refresh (⌘R)"
                >
                  Refresh usage
                </Button>
              </>
            )}
            <Button
              variant="solid"
              glyph={NF.plus}
              onClick={() => setShowAdd(true)}
              title="Add account (⌘N)"
            >
              Add account
            </Button>
          </>
        }
      />

      <AccountsGrid
        accounts={accounts}
        shown={shown}
        usage={usage}
        status={status}
        busyKeys={busyKeys}
        filter={filter}
        onFilterChange={setFilter}
        onLogin={actions.login}
        onContextMenu={handleContextMenu}
        cliHandlers={cliHandlers}
        desktopHandlers={desktopHandlers}
      />

      <AddAccountModal
        open={showAdd}
        onClose={() => setShowAdd(false)}
        accounts={accounts}
        onAdded={async () => {
          setShowAdd(false);
          await refresh();
          pushToast("info", "Account added.");
        }}
        onError={(msg) => pushToast("error", msg)}
        onAdoptDesktop={(a) => actions.adoptDesktop(a)}
      />

      {ctxMenu && (
        <CtxMenuForAccount
          menu={ctxMenu}
          status={status}
          busyKeys={busyKeys}
          onSwitchCli={requestCliSwap}
          onSwitchDesktop={handleDesktopSwitch}
          onSwitchDesktopNoLaunch={(a) => actions.useDesktop(a, true)}
          onVerify={runVerifyAccount}
          onRefreshUsageFor={(a) => refreshUsageFor(a.uuid)}
          onRefreshUsageAll={refreshUsage}
          onLogin={actions.login}
          onRemove={requestRemoveAccount}
          onAdoptDesktop={(a) => {
            // Adopt with no overwrite by default. If a snapshot
            // already exists for this account, go through the
            // shell-level confirm — the user must opt into
            // replacing the existing profile.
            if (a.desktop_profile_on_disk) requestDesktopOverwrite(a);
            else void actions.adoptDesktop(a);
          }}
          onClose={closeCtxMenu}
        />
      )}

    </>
  );
}
