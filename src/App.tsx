import { useEffect, useState } from "react";
import { IconContext } from "@phosphor-icons/react";
import { api } from "./api";
import type { AccountSummary } from "./types";
import { useToasts } from "./hooks/useToasts";
import { useBusy } from "./hooks/useBusy";
import { useRefresh } from "./hooks/useRefresh";
import { useUsage } from "./hooks/useUsage";
import { useActions } from "./hooks/useActions";
import { Sidebar } from "./components/Sidebar";
import { ContentPane } from "./components/ContentPane";
import { AddAccountModal } from "./components/AddAccountModal";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { ToastContainer } from "./components/ToastContainer";

function App() {
  const { toasts, pushToast, dismissToast } = useToasts();
  const busy = useBusy();
  const { status, accounts, loadError, keychainIssue, refresh } =
    useRefresh(pushToast);
  const { usage, refreshUsage } = useUsage();
  const actions = useActions({ pushToast, refresh, ...busy });

  const [showAdd, setShowAdd] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<AccountSummary | null>(null);
  const [confirmDesktop, setConfirmDesktop] = useState<AccountSummary | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [selectedUuid, setSelectedUuid] = useState<string | null>(null);

  // Auto-select an account on first load. Priority: active CLI > active
  // Desktop > first account. Only fires while nothing is selected, so it
  // won't fight a user click later. Re-runs after refresh if the selected
  // account vanished (e.g. removed externally).
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
        <div className="app-layout">
          <div className="content loading">
            <div className="empty">
              <h2>Couldn't load Claudepot</h2>
              <p className="muted mono">{loadError}</p>
              <button className="primary" onClick={refresh}>Retry</button>
            </div>
          </div>
        </div>
      );
    }
    return (
      <div className="app-layout">
        <div className="content loading">
          <div className="skeleton-container">
            <div className="skeleton skeleton-header" />
            <div className="skeleton skeleton-card" />
            <div className="skeleton skeleton-card" />
            <div className="skeleton skeleton-card short" />
          </div>
        </div>
      </div>
    );
  }

  return (
    <IconContext.Provider value={{ size: 16, weight: "light" }}>
      <div className="app-layout">
        <div className="titlebar-drag" data-tauri-drag-region />
        <Sidebar
          accounts={accounts}
          usage={usage}
          selectedUuid={selectedUuid}
          onSelect={setSelectedUuid}
          onAdd={() => setShowAdd(true)}
          onRefresh={() => { refresh(); refreshUsage(); }}
        />

        <main className="content">
          {keychainIssue && (
            <div className="banner warn" role="alert">
              <div>
                <strong>Keychain locked.</strong> Click{" "}
                <em>Unlock</em> to enter your macOS password.
              </div>
              <div className="banner-actions">
                <button className="primary" onClick={async () => {
                  try { await api.unlockKeychain(); await refresh(); }
                  catch (e) { pushToast("error", `Unlock failed: ${e}`); }
                }}>Unlock</button>
              </div>
            </div>
          )}

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

        <ToastContainer toasts={toasts} onDismiss={dismissToast} />
      </div>
    </IconContext.Provider>
  );
}

export default App;
