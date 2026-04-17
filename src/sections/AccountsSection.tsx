import { useCallback, useEffect, useState } from "react";
import { User } from "lucide-react";
import { api } from "../api";
import type { AccountSummary } from "../types";
import { useToasts } from "../hooks/useToasts";
import { useBusy } from "../hooks/useBusy";
import { useRefresh } from "../hooks/useRefresh";
import { useUsage } from "../hooks/useUsage";
import { useActions } from "../hooks/useActions";
import { Sidebar } from "../components/Sidebar";
import { ContentPane } from "../components/ContentPane";
import { StatusBar } from "../components/StatusBar";
import { ContextMenu, type ContextMenuItem } from "../components/ContextMenu";
import { AddAccountModal } from "../components/AddAccountModal";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { ToastContainer } from "../components/ToastContainer";

/**
 * Accounts section. Owns all account-related state, data fetching,
 * and modals. Rendered as the body of the Accounts tab in the section
 * rail. Nothing here knows about the rail itself — the shell is
 * responsible for placement.
 */
export function AccountsSection() {
  const { toasts, pushToast, dismissToast } = useToasts();
  const busy = useBusy();
  const {
    status,
    accounts,
    loadError,
    keychainIssue,
    syncError,
    ccIdentity,
    verifying,
    refresh,
  } = useRefresh(pushToast);
  const { usage, refreshUsage } = useUsage();
  const actions = useActions({ pushToast, refresh, ...busy });

  const [showAdd, setShowAdd] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<AccountSummary | null>(null);
  const [confirmDesktop, setConfirmDesktop] = useState<AccountSummary | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [selectedUuid, setSelectedUuid] = useState<string | null>(null);
  const [ctxMenu, setCtxMenu] = useState<{
    x: number; y: number; account: AccountSummary;
  } | null>(null);

  const handleContextMenu = useCallback(
    (e: React.MouseEvent, a: AccountSummary) => {
      e.preventDefault();
      setCtxMenu({ x: e.clientX, y: e.clientY, account: a });
    },
    [],
  );

  const closeCtxMenu = useCallback(() => setCtxMenu(null), []);

  // Auto-select: active CLI > active Desktop > first account. Re-runs
  // if the selected account vanished externally.
  useEffect(() => {
    if (accounts.length === 0) return;
    const stillExists = selectedUuid && accounts.some((a) => a.uuid === selectedUuid);
    if (stillExists) return;
    const cliActive = accounts.find((a) => a.is_cli_active);
    const desktopActive = accounts.find((a) => a.is_desktop_active);
    setSelectedUuid((cliActive ?? desktopActive ?? accounts[0]).uuid);
  }, [accounts, selectedUuid]);

  const selectedAccount = accounts.find((a) => a.uuid === selectedUuid) ?? null;

  if (!status) {
    if (loadError) {
      return (
        <div className="content loading">
          <div className="empty">
            <h2>Couldn't load Claudepot</h2>
            <p className="muted mono">{loadError}</p>
            <button className="primary" onClick={refresh}>Retry</button>
          </div>
        </div>
      );
    }
    return (
      <div className="content loading">
        <div className="skeleton-container">
          <div className="skeleton skeleton-header" />
          <div className="skeleton skeleton-card" />
          <div className="skeleton skeleton-card" />
          <div className="skeleton skeleton-card short" />
        </div>
      </div>
    );
  }

  return (
    <>
      <Sidebar
        accounts={accounts}
        usage={usage}
        selectedUuid={selectedUuid}
        busyKeys={busy.busyKeys}
        onSelect={setSelectedUuid}
        onAdd={() => setShowAdd(true)}
        onRefresh={() => { refresh(); refreshUsage(); }}
        onSwitchCli={(a) => actions.useCli(a)}
        onLogin={(a) => actions.login(a)}
        onContextMenu={handleContextMenu}
      />

      <main className="content">
        <StatusBar
          ccIdentity={ccIdentity}
          status={status}
          syncError={syncError}
          keychainIssue={keychainIssue}
          accounts={accounts}
          verifying={verifying}
          onUnlock={async () => {
            try { await api.unlockKeychain(); await refresh(); }
            catch (e) { pushToast("error", `Unlock failed: ${e}`); }
          }}
        />

        <ContentPane
          account={selectedAccount}
          usage={selectedAccount ? usage[selectedAccount.uuid] ?? null : null}
          status={status}
          busyKeys={busy.busyKeys}
          anyBusy={busy.anyBusy}
          onUseCli={(a) => actions.useCli(a)}
          onUseDesktop={(a) => setConfirmDesktop(a)}
          onLogin={(a) => actions.login(a)}
          onCancelLogin={actions.cancelLogin}
          onRemove={(a) => setConfirmRemove(a)}
          onClearCli={() => setConfirmClear(true)}
          onAdd={() => setShowAdd(true)}
        />
      </main>

      {showAdd && <AddAccountModal
        onClose={() => setShowAdd(false)}
        onAdded={async () => { setShowAdd(false); await refresh(); pushToast("info", "Account added."); }}
        onError={(msg) => pushToast("error", msg)} />}

      {confirmRemove && <ConfirmDialog title="Remove account?" confirmLabel="Remove" confirmDanger
        body={<><p>Remove <strong>{confirmRemove.email}</strong>?</p>
          <p className="muted small">Deletes credentials and Desktop profile.
          Active CLI/Desktop pointers will be cleared.</p></>}
        onCancel={() => setConfirmRemove(null)}
        onConfirm={async () => { const t = confirmRemove; setConfirmRemove(null); await actions.performRemove(t); }} />}

      {confirmDesktop && <ConfirmDialog title="Switch Desktop?" confirmLabel="Switch"
        body={<p>Switch Desktop to <strong>{confirmDesktop.email}</strong>? Claude Desktop will quit and relaunch.</p>}
        onCancel={() => setConfirmDesktop(null)}
        onConfirm={async () => { const t = confirmDesktop; setConfirmDesktop(null); await actions.useDesktop(t); }} />}

      {confirmClear && <ConfirmDialog title="Sign out of Claude Code?" confirmLabel="Clear" confirmDanger
        body={<p>Clears CC's credentials file. You'll need to sign in again.</p>}
        onCancel={() => setConfirmClear(false)}
        onConfirm={async () => { setConfirmClear(false); await actions.performClearCli(); }} />}

      {ctxMenu && (() => {
        const a = ctxMenu.account;
        const items: ContextMenuItem[] = [
          { label: "Copy email", onClick: () => navigator.clipboard.writeText(a.email) },
          { label: "Copy UUID", onClick: () => navigator.clipboard.writeText(a.uuid) },
          { label: "", separator: true, onClick: () => {} },
          {
            label: a.is_cli_active ? "Active CLI" : "Set as CLI",
            disabled: a.is_cli_active || !a.credentials_healthy,
            onClick: () => actions.useCli(a),
          },
          {
            label: a.is_desktop_active ? "Active Desktop" : "Set as Desktop",
            disabled: a.is_desktop_active || !a.has_desktop_profile || !status.desktop_installed,
            onClick: () => setConfirmDesktop(a),
          },
          { label: "", separator: true, onClick: () => {} },
          { label: "Remove", danger: true, onClick: () => setConfirmRemove(a) },
        ];
        return (
          <ContextMenu x={ctxMenu.x} y={ctxMenu.y} items={items} onClose={closeCtxMenu} />
        );
      })()}

      <ToastContainer toasts={toasts} onDismiss={dismissToast} />
    </>
  );
}

AccountsSection.icon = <User />;
AccountsSection.label = "Accounts";
