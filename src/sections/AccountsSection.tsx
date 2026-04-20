import { useCallback, useEffect, useMemo, useState } from "react";
import type { AccountSummary, AppStatus } from "../types";
import { useBusy } from "../hooks/useBusy";
import { useUsage } from "../hooks/useUsage";
import { useActions } from "../hooks/useActions";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { useGlobalShortcuts } from "../hooks/useGlobalShortcuts";
import { useAppState } from "../providers/AppStateProvider";
import { ContextMenu } from "../components/ContextMenu";
import { CommandPalette } from "../components/CommandPalette";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { Button } from "../components/primitives/Button";
import { Glyph } from "../components/primitives/Glyph";
import { Input } from "../components/primitives/Input";
import { NF } from "../icons";
import { ScreenHeader } from "../shell/ScreenHeader";
import { AccountCard } from "./accounts/AccountCard";
import { AddAccountModal } from "./accounts/AddAccountModal";
import { isAnomaly } from "./accounts/AnomalyBanner";
import { SplitBrainConfirm } from "./accounts/SplitBrainConfirm";
import { useAccountContextMenu } from "./accounts/useAccountContextMenu";
import { useAccountHandlers } from "./accounts/useAccountHandlers";

/**
 * Accounts section. Renders the header, filter bar, and the card grid.
 * Refresh/toast state is lifted to `AppStateProvider` — the shell-level
 * `StatusIssuesBanner` and this section share the same `/profile` and
 * `verify_all_accounts` traffic off one useRefresh instance. Per-view
 * state (usage cache, busy keys, modals, palette) stays local.
 */
export function AccountsSection({
  onNavigate,
}: {
  onNavigate?: (section: string, subRoute?: string | null) => void;
}) {
  const { pushToast, status, accounts, loadError, refresh } = useAppState();
  const busy = useBusy();
  const { usage, refreshUsage, refreshUsageFor } = useUsage();
  const actions = useActions({ pushToast, refresh, ...busy });

  const [showAdd, setShowAdd] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<AccountSummary | null>(
    null,
  );
  const [confirmClear, setConfirmClear] = useState(false);
  const [confirmSplitBrain, setConfirmSplitBrain] =
    useState<AccountSummary | null>(null);
  const [showPalette, setShowPalette] = useState(false);
  const [filter, setFilter] = useState("");
  const [ctxMenu, setCtxMenu] = useState<
    | { kind: "row"; x: number; y: number; account: AccountSummary }
    | null
  >(null);

  const {
    runVerifyAccount,
    runVerifyAll,
    handleDesktopSwitch,
    guardedUseCli,
  } = useAccountHandlers({
    pushToast,
    refresh,
    useDesktop: actions.useDesktop,
    useCli: actions.useCli,
    setConfirmSplitBrain,
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
        if (target) {
          navigator.clipboard.writeText(target.email);
          pushToast("info", `Copied ${target.email}`);
        }
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
    onPalette: () => setShowPalette(true),
  });

  // Command palette bridge — WindowChrome dispatches this event when
  // the ⌘K hint is clicked. App.tsx can't open the palette directly
  // because the palette component currently lives inside this section.
  useEffect(() => {
    const onOpen = () => setShowPalette(true);
    window.addEventListener("cp-open-palette", onOpen);
    return () => window.removeEventListener("cp-open-palette", onOpen);
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

  const anomalyCount = useMemo(
    () => accounts.filter(isAnomaly).length,
    [accounts],
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

  const subtitle = (() => {
    const n = accounts.length;
    if (n === 0) return "No accounts registered yet.";
    const accountsLabel = `${n} account${n === 1 ? "" : "s"}`;
    return anomalyCount > 0
      ? `${accountsLabel} · ${anomalyCount} need${anomalyCount === 1 ? "s" : ""} attention`
      : `${accountsLabel} · all healthy`;
  })();

  return (
    <>
      <ScreenHeader
        title="Accounts"
        subtitle={subtitle}
        actions={
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
            <Button
              variant="ghost"
              glyph={NF.unlock}
              glyphColor="var(--fg-muted)"
              onClick={() => setConfirmClear(true)}
              title="Clear Claude Code's stored credentials"
            >
              Sign out CC
            </Button>
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

      {/* Filter input only earns its row when there are enough
          accounts to usefully narrow. With 1–3 accounts the input
          is pure chrome. Once a 4th lands, the filter appears. */}
      {accounts.length > 3 && (
        <div
          style={{
            padding: "var(--sp-14) var(--sp-32)",
            borderBottom: "var(--bw-hair) solid var(--line)",
            display: "flex",
            gap: "var(--sp-12)",
            alignItems: "center",
            background: "var(--bg)",
          }}
        >
          <Input
            glyph={NF.search}
            placeholder="Filter accounts"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            style={{ width: "var(--filter-input-width)" }}
            aria-label="Filter accounts"
          />
          {filter.trim() !== "" && (
            <span
              className="mono-cap"
              style={{
                color: "var(--fg-faint)",
                marginLeft: "var(--sp-4)",
              }}
            >
              {`${shown.length} / ${accounts.length}`}
            </span>
          )}
          <div style={{ flex: 1 }} />
        </div>
      )}

      <div
        style={{
          padding: "var(--sp-20) var(--sp-32) var(--sp-40)",
          display: "grid",
          gridTemplateColumns:
            "repeat(auto-fill, minmax(var(--content-cap-sm), 1fr))",
          gap: "var(--sp-16)",
        }}
      >
        {shown.map((a) => (
          <AccountCard
            key={a.uuid}
            account={a}
            usageEntry={usage[a.uuid] ?? null}
            loginBusy={busy.busyKeys.has(`re-${a.uuid}`)}
            onRemove={(x) => setConfirmRemove(x)}
            onLogin={(x) => actions.login(x)}
            onContextMenu={handleContextMenu}
          />
        ))}
        {shown.length === 0 && accounts.length > 0 && (
          <div
            style={{
              gridColumn: "1 / -1",
              padding: "var(--sp-60)",
              textAlign: "center",
              color: "var(--fg-faint)",
              fontSize: "var(--fs-sm)",
            }}
          >
            No accounts match "{filter}".
          </div>
        )}
        {accounts.length === 0 && (
          <div
            style={{
              gridColumn: "1 / -1",
              padding: "var(--sp-60)",
              textAlign: "center",
              color: "var(--fg-faint)",
              fontSize: "var(--fs-sm)",
              display: "flex",
              flexDirection: "column",
              gap: "var(--sp-10)",
              alignItems: "center",
            }}
          >
            <Glyph g={NF.users} size="var(--sp-32)" color="var(--fg-ghost)" />
            <p style={{ margin: 0 }}>No accounts yet.</p>
            <p
              style={{
                margin: 0,
                fontSize: "var(--fs-xs)",
                color: "var(--fg-faint)",
              }}
            >
              {"Click "}
              <b>Add account</b>
              {" to import Claude Code's current session."}
            </p>
          </div>
        )}
      </div>

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
      />

      {confirmRemove && (
        <ConfirmDialog
          title="Remove account?"
          confirmLabel="Remove"
          confirmDanger
          body={
            <>
              <p>
                Remove <strong>{confirmRemove.email}</strong>?
              </p>
              <p className="muted small">
                Deletes credentials and Desktop profile. Active
                CLI/Desktop pointers will be cleared.
              </p>
            </>
          }
          onCancel={() => setConfirmRemove(null)}
          onConfirm={() => {
            const t = confirmRemove;
            setConfirmRemove(null);
            actions.performRemove(t);
          }}
        />
      )}

      {confirmClear && (
        <ConfirmDialog
          title="Sign out of Claude Code?"
          confirmLabel="Clear"
          confirmDanger
          body={
            <p>
              Clears CC's credentials file. You'll need to sign in
              again.
            </p>
          }
          onCancel={() => setConfirmClear(false)}
          onConfirm={async () => {
            setConfirmClear(false);
            await actions.performClearCli();
          }}
        />
      )}

      {confirmSplitBrain && (
        <SplitBrainConfirm
          account={confirmSplitBrain}
          onCancel={() => setConfirmSplitBrain(null)}
          onConfirm={() => {
            const target = confirmSplitBrain;
            setConfirmSplitBrain(null);
            void actions.useCli(target, true);
          }}
        />
      )}

      {showPalette && status && (
        <CommandPalette
          accounts={accounts}
          status={status}
          onClose={() => setShowPalette(false)}
          onSwitchCli={(a) => guardedUseCli(a)}
          onSwitchDesktop={(a) => handleDesktopSwitch(a)}
          onAdd={() => setShowAdd(true)}
          onRefresh={() => {
            refresh();
            refreshUsage();
          }}
          onRemove={(a) => setConfirmRemove(a)}
          onNavigate={onNavigate}
        />
      )}

      {ctxMenu && (
        <CtxMenuForAccount
          menu={ctxMenu}
          status={status}
          busyKeys={busy.busyKeys}
          onSwitchCli={guardedUseCli}
          onSwitchDesktop={handleDesktopSwitch}
          onSwitchDesktopNoLaunch={(a) => actions.useDesktop(a, true)}
          onVerify={runVerifyAccount}
          onRefreshUsageFor={(a) => refreshUsageFor(a.uuid)}
          onRefreshUsageAll={refreshUsage}
          onLogin={actions.login}
          onRemove={setConfirmRemove}
          onClose={closeCtxMenu}
        />
      )}

    </>
  );
}

/**
 * Small hook wrapper. The menu-item set is computed via
 * `useAccountContextMenu` which can only run inside a component;
 * splitting it out keeps the main `AccountsSection` under the LOC
 * limit without adding a provider.
 */
function CtxMenuForAccount({
  menu,
  status,
  busyKeys,
  onSwitchCli,
  onSwitchDesktop,
  onSwitchDesktopNoLaunch,
  onVerify,
  onRefreshUsageFor,
  onRefreshUsageAll,
  onLogin,
  onRemove,
  onClose,
}: {
  menu: { x: number; y: number; account: AccountSummary };
  status: AppStatus;
  busyKeys: Set<string>;
  onSwitchCli: (a: AccountSummary) => void;
  onSwitchDesktop: (a: AccountSummary) => void;
  onSwitchDesktopNoLaunch: (a: AccountSummary) => void;
  onVerify: (a: AccountSummary) => void;
  onRefreshUsageFor: (a: AccountSummary) => void;
  onRefreshUsageAll: () => void;
  onLogin: (a: AccountSummary) => void;
  onRemove: (a: AccountSummary) => void;
  onClose: () => void;
}) {
  const items = useAccountContextMenu({
    account: menu.account,
    status,
    busyKeys,
    onSwitchCli,
    onSwitchDesktop,
    onSwitchDesktopNoLaunch,
    onVerify,
    onRefreshUsageFor,
    onRefreshUsageAll,
    onLogin,
    onRemove,
  });
  return (
    <ContextMenu x={menu.x} y={menu.y} items={items} onClose={onClose} />
  );
}
