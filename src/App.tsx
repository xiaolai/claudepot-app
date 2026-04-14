import { useState } from "react";
import { api } from "./api";
import type { AccountSummary } from "./types";
import { useToasts } from "./hooks/useToasts";
import { useBusy } from "./hooks/useBusy";
import { useRefresh } from "./hooks/useRefresh";
import { useActions } from "./hooks/useActions";
import { Header } from "./components/Header";
import { AccountCard } from "./components/AccountCard";
import { EmptyState } from "./components/EmptyState";
import { AddAccountModal } from "./components/AddAccountModal";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { ToastContainer } from "./components/ToastContainer";
import "./App.css";

function App() {
  const { toasts, pushToast, dismissToast } = useToasts();
  const busy = useBusy();
  const { status, accounts, loadError, keychainIssue, refresh } =
    useRefresh(pushToast);
  const actions = useActions({ pushToast, refresh, ...busy });

  const [showAdd, setShowAdd] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<AccountSummary | null>(null);
  const [confirmDesktop, setConfirmDesktop] = useState<AccountSummary | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [expandedUuid, setExpandedUuid] = useState<string | null>(null);

  if (!status) {
    if (loadError) {
      return (
        <main className="app loading">
          <div className="empty">
            <h2>Couldn't load Claudepot</h2>
            <p className="muted mono">{loadError}</p>
            <button className="primary" onClick={refresh}>Retry</button>
          </div>
        </main>
      );
    }
    return <main className="app loading"><p>Loading…</p></main>;
  }

  return (
    <main className="app">
      <Header status={status} onRefresh={refresh} />

      {keychainIssue && (
        <div className="banner warn" role="alert">
          <div>
            <strong>macOS Keychain is locked.</strong> Claudepot can't read
            credentials until you unlock it. Click <em>Unlock</em> — macOS
            will show its standard password prompt.
          </div>
          <div className="banner-actions">
            <button className="primary" onClick={async () => {
              try { await api.unlockKeychain(); await refresh(); }
              catch (e) { pushToast("error", `Unlock failed: ${e}`); }
            }}>Unlock</button>
            <button onClick={refresh}>Retry</button>
          </div>
        </div>
      )}

      <section className="accounts">
        {accounts.length === 0 ? (
          <EmptyState onAdd={() => setShowAdd(true)} />
        ) : (
          accounts.map((a) => (
            <AccountCard key={a.uuid} account={a}
              desktopAvailable={status.desktop_installed}
              busyKeys={busy.busyKeys} anyBusy={busy.anyBusy}
              expanded={expandedUuid === a.uuid}
              onToggleExpand={() => setExpandedUuid((prev) => prev === a.uuid ? null : a.uuid)}
              onUseCli={() => actions.useCli(a)}
              onUseDesktop={() => setConfirmDesktop(a)}
              onLogin={() => actions.login(a)}
              onCancelLogin={actions.cancelLogin}
              onRemove={() => setConfirmRemove(a)} />
          ))
        )}
      </section>

      <footer className="footer">
        <button className="primary" onClick={() => setShowAdd(true)}>+ Add account</button>
        {status.cli_active_email && (
          <button className="danger" onClick={() => setConfirmClear(true)}
            disabled={busy.anyBusy} title="Sign CC out — clears credentials file">
            Clear CLI
          </button>
        )}
        <span className="muted mono">{status.data_dir}</span>
      </footer>

      {showAdd && <AddAccountModal
        onClose={() => setShowAdd(false)}
        onAdded={async () => { setShowAdd(false); await refresh(); pushToast("info", "Account added."); }}
        onError={(msg) => pushToast("error", msg)} />}

      {confirmRemove && <ConfirmDialog title="Remove account?" confirmLabel="Remove" confirmDanger
        body={<><p>Remove <strong>{confirmRemove.email}</strong>?</p>
          <p className="muted small">This deletes the credential blob and any saved Desktop profile.
          Active CLI/Desktop pointers will be cleared.</p></>}
        onCancel={() => setConfirmRemove(null)}
        onConfirm={async () => { const t = confirmRemove; setConfirmRemove(null); await actions.performRemove(t); }} />}

      {confirmDesktop && <ConfirmDialog title="Switch Desktop?" confirmLabel="Switch"
        body={<p>Switch Desktop to <strong>{confirmDesktop.email}</strong>? Claude Desktop will quit and relaunch.</p>}
        onCancel={() => setConfirmDesktop(null)}
        onConfirm={async () => { const t = confirmDesktop; setConfirmDesktop(null); await actions.useDesktop(t); }} />}

      {confirmClear && <ConfirmDialog title="Sign out of Claude Code?" confirmLabel="Clear" confirmDanger
        body={<p>This clears CC's credentials file. You'll need to sign in again.</p>}
        onCancel={() => setConfirmClear(false)}
        onConfirm={async () => { setConfirmClear(false); await actions.performClearCli(); }} />}

      <ToastContainer toasts={toasts} onDismiss={dismissToast} />
    </main>
  );
}

export default App;
