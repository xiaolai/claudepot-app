import { useCallback, useEffect, useState } from "react";
import { api } from "./api";
import type { AccountSummary, AppStatus } from "./types";
import "./App.css";

type Toast = { id: number; kind: "info" | "error"; text: string };

function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [busy, setBusy] = useState<string | null>(null); // id of busy account
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [showAdd, setShowAdd] = useState(false);

  const pushToast = useCallback((kind: Toast["kind"], text: string) => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t, { id, kind, text }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 4000);
  }, []);

  const refresh = useCallback(async () => {
    try {
      const [s, list] = await Promise.all([
        api.appStatus(),
        api.accountList(),
      ]);
      setStatus(s);
      setAccounts(list);
    } catch (e) {
      pushToast("error", `refresh failed: ${e}`);
    }
  }, [pushToast]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const withBusy = async <T,>(key: string, fn: () => Promise<T>) => {
    setBusy(key);
    try {
      return await fn();
    } finally {
      setBusy(null);
    }
  };

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

  const remove = (a: AccountSummary) =>
    withBusy(`rm-${a.uuid}`, async () => {
      if (
        !confirm(`Remove ${a.email}? Credentials and profile will be deleted.`)
      )
        return;
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

  if (!status) {
    return (
      <main className="app loading">
        <p>Loading…</p>
      </main>
    );
  }

  return (
    <main className="app">
      <header className="header">
        <div className="brand">
          <h1>Claudepot</h1>
          <span className="muted">
            {status.platform} / {status.arch} · {status.account_count}{" "}
            account{status.account_count === 1 ? "" : "s"}
          </span>
        </div>
        <div className="active-row">
          <ActivePill label="CLI" email={status.cli_active_email} />
          <ActivePill
            label="Desktop"
            email={status.desktop_active_email}
            disabled={!status.desktop_installed}
            disabledHint="Desktop not installed"
          />
        </div>
      </header>

      <section className="accounts">
        {accounts.length === 0 ? (
          <EmptyState onAdd={() => setShowAdd(true)} />
        ) : (
          accounts.map((a) => (
            <AccountCard
              key={a.uuid}
              account={a}
              desktopAvailable={status.desktop_installed}
              busyKey={busy}
              onUseCli={() => useCli(a)}
              onUseDesktop={() => useDesktop(a)}
              onRemove={() => remove(a)}
            />
          ))
        )}
      </section>

      <footer className="footer">
        <button className="primary" onClick={() => setShowAdd(true)}>
          + Add account
        </button>
        <span className="muted mono">{status.data_dir}</span>
      </footer>

      {showAdd && (
        <AddAccountModal
          onClose={() => setShowAdd(false)}
          onAdded={async () => {
            setShowAdd(false);
            await refresh();
            pushToast("info", "Account added.");
          }}
          onError={(msg) => pushToast("error", msg)}
        />
      )}

      <div className="toasts">
        {toasts.map((t) => (
          <div key={t.id} className={`toast ${t.kind}`}>
            {t.text}
          </div>
        ))}
      </div>
    </main>
  );
}

function ActivePill({
  label,
  email,
  disabled,
  disabledHint,
}: {
  label: string;
  email: string | null;
  disabled?: boolean;
  disabledHint?: string;
}) {
  if (disabled) {
    return (
      <div className="pill disabled" title={disabledHint}>
        <span className="pill-label">{label}</span>
        <span className="pill-value muted">{disabledHint}</span>
      </div>
    );
  }
  return (
    <div className={`pill ${email ? "active" : ""}`}>
      <span className="pill-label">{label}</span>
      <span className="pill-value">{email ?? "—"}</span>
    </div>
  );
}

function AccountCard({
  account: a,
  desktopAvailable,
  busyKey,
  onUseCli,
  onUseDesktop,
  onRemove,
}: {
  account: AccountSummary;
  desktopAvailable: boolean;
  busyKey: string | null;
  onUseCli: () => void;
  onUseDesktop: () => void;
  onRemove: () => void;
}) {
  const cliBusy = busyKey === `cli-${a.uuid}`;
  const deskBusy = busyKey === `desk-${a.uuid}`;
  const rmBusy = busyKey === `rm-${a.uuid}`;
  const anyBusy = busyKey !== null;

  return (
    <article
      className={`account ${a.is_cli_active ? "cli-active" : ""} ${
        a.is_desktop_active ? "desktop-active" : ""
      }`}
    >
      <div className="account-main">
        <div className="account-head">
          <h3>{a.email}</h3>
          <TokenBadge status={a.token_status} mins={a.token_remaining_mins} />
        </div>
        <div className="account-meta muted">
          {a.org_name ?? "—"} · {a.subscription_type ?? "—"}
        </div>
      </div>
      <div className="account-actions">
        <button
          onClick={onUseCli}
          disabled={anyBusy || a.is_cli_active || !a.has_cli_credentials}
          title={
            a.is_cli_active
              ? "Already active CLI"
              : !a.has_cli_credentials
              ? "No credentials on file"
              : "Use for CLI"
          }
        >
          {cliBusy ? "…" : a.is_cli_active ? "✓ CLI" : "Use CLI"}
        </button>
        <button
          onClick={onUseDesktop}
          disabled={anyBusy || a.is_desktop_active || !desktopAvailable}
          title={
            !desktopAvailable
              ? "Desktop not installed"
              : a.is_desktop_active
              ? "Already active Desktop"
              : "Use for Desktop (quits + relaunches Claude)"
          }
        >
          {deskBusy ? "…" : a.is_desktop_active ? "✓ Desktop" : "Use Desktop"}
        </button>
        <button
          onClick={onRemove}
          disabled={anyBusy}
          className="danger"
          title="Remove account, credentials, and profile"
        >
          {rmBusy ? "…" : "Remove"}
        </button>
      </div>
    </article>
  );
}

function TokenBadge({
  status,
  mins,
}: {
  status: string;
  mins: number | null;
}) {
  const kind = status.startsWith("valid")
    ? "ok"
    : status === "expired"
    ? "bad"
    : "warn";
  const label = kind === "ok" && mins != null ? `valid · ${mins}m` : status;
  return <span className={`token-badge ${kind}`}>{label}</span>;
}

function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="empty">
      <h2>No accounts yet</h2>
      <p className="muted">
        Add your first account — Claudepot will pick up whichever one Claude
        Code is currently signed into.
      </p>
      <button className="primary" onClick={onAdd}>
        + Add account
      </button>
    </div>
  );
}

function AddAccountModal({
  onClose,
  onAdded,
  onError,
}: {
  onClose: () => void;
  onAdded: () => void;
  onError: (msg: string) => void;
}) {
  const [mode, setMode] = useState<"current" | "token">("current");
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    setBusy(true);
    try {
      if (mode === "current") {
        await api.accountAddFromCurrent();
      } else {
        if (!token.trim()) throw new Error("refresh token required");
        await api.accountAddFromToken(token.trim());
      }
      onAdded();
    } catch (e) {
      onError(`add failed: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Add account</h2>

        <div className="mode-tabs">
          <button
            className={mode === "current" ? "active" : ""}
            onClick={() => setMode("current")}
          >
            From current CC login
          </button>
          <button
            className={mode === "token" ? "active" : ""}
            onClick={() => setMode("token")}
          >
            From refresh token
          </button>
        </div>

        <div className="modal-body">
          {mode === "current" ? (
            <p className="muted">
              Imports whichever account Claude Code is currently signed into.
              Log in with <code>claude auth login</code> first if needed.
            </p>
          ) : (
            <>
              <p className="muted">
                Paste an <code>sk-ant-ort01-…</code> refresh token (headless
                onboarding).
              </p>
              <input
                type="password"
                value={token}
                onChange={(e) => setToken(e.currentTarget.value)}
                placeholder="sk-ant-ort01-…"
                spellCheck={false}
                autoComplete="off"
                disabled={busy}
              />
            </>
          )}
        </div>

        <div className="modal-actions">
          <button onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className="primary" onClick={submit} disabled={busy}>
            {busy ? "Adding…" : "Add"}
          </button>
        </div>
      </div>
    </div>
  );
}

export default App;
