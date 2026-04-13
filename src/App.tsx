import React, { useCallback, useEffect, useState } from "react";
import { api } from "./api";
import type { AccountSummary, AppStatus } from "./types";
import "./App.css";

type Toast = { id: number; kind: "info" | "error"; text: string };

function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null); // id of busy account
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [showAdd, setShowAdd] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<AccountSummary | null>(
    null,
  );

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
      setLoadError(null);
    } catch (e) {
      const msg = `${e}`;
      setLoadError(msg);
      pushToast("error", `refresh failed: ${msg}`);
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

  const reimport = (a: AccountSummary) =>
    withBusy(`re-${a.uuid}`, async () => {
      try {
        await api.accountReimportFromCurrent(a.uuid);
        pushToast("info", `Re-imported credentials for ${a.email}`);
        await refresh();
      } catch (e) {
        pushToast(
          "error",
          `Re-import failed: ${e}. Make sure CC is logged in as ${a.email} (\`claude auth login\`), then try again.`,
        );
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

  if (!status) {
    if (loadError) {
      return (
        <main className="app loading">
          <div className="empty">
            <h2>Couldn't load Claudepot</h2>
            <p className="muted mono">{loadError}</p>
            <button className="primary" onClick={refresh}>
              Retry
            </button>
          </div>
        </main>
      );
    }
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
              onReimport={() => reimport(a)}
              onRemove={() => setConfirmRemove(a)}
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

      {confirmRemove && (
        <ConfirmDialog
          title="Remove account?"
          body={
            <>
              <p>
                Remove <strong>{confirmRemove.email}</strong>?
              </p>
              <p className="muted small">
                This deletes the credential blob and any saved Desktop
                profile from this machine. Active CLI/Desktop pointers
                will be cleared. The account on Anthropic's side is not
                affected.
              </p>
            </>
          }
          confirmLabel="Remove"
          confirmDanger
          onCancel={() => setConfirmRemove(null)}
          onConfirm={async () => {
            const target = confirmRemove;
            setConfirmRemove(null);
            await performRemove(target);
          }}
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
  onReimport,
  onRemove,
}: {
  account: AccountSummary;
  desktopAvailable: boolean;
  busyKey: string | null;
  onUseCli: () => void;
  onUseDesktop: () => void;
  onReimport: () => void;
  onRemove: () => void;
}) {
  const cliBusy = busyKey === `cli-${a.uuid}`;
  const deskBusy = busyKey === `desk-${a.uuid}`;
  const reBusy = busyKey === `re-${a.uuid}`;
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
        {a.credentials_healthy ? (
          <button
            onClick={onUseCli}
            disabled={anyBusy || a.is_cli_active}
            title={a.is_cli_active ? "Already active CLI" : "Use for CLI"}
          >
            {cliBusy ? "…" : a.is_cli_active ? "✓ CLI" : "Use CLI"}
          </button>
        ) : (
          <button
            onClick={onReimport}
            disabled={anyBusy}
            className="warn"
            title={`Re-import from CC. Sign into CC as ${a.email} first (\`claude auth login\`).`}
          >
            {reBusy ? "…" : "Re-import"}
          </button>
        )}
        <button
          onClick={onUseDesktop}
          disabled={
            anyBusy ||
            a.is_desktop_active ||
            !desktopAvailable ||
            !a.has_desktop_profile
          }
          title={
            !desktopAvailable
              ? "Desktop not installed"
              : a.is_desktop_active
              ? "Already active Desktop"
              : !a.has_desktop_profile
              ? "No Desktop profile yet — sign in via the Desktop app first"
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

function ConfirmDialog({
  title,
  body,
  confirmLabel = "Confirm",
  confirmDanger = false,
  onCancel,
  onConfirm,
}: {
  title: string;
  body: React.ReactNode;
  confirmLabel?: string;
  confirmDanger?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 id="confirm-title">{title}</h2>
        <div className="modal-body">{body}</div>
        <div className="modal-actions">
          <button onClick={onCancel}>Cancel</button>
          <button
            className={confirmDanger ? "danger primary" : "primary"}
            onClick={onConfirm}
            autoFocus
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
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
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    setBusy(true);
    try {
      await api.accountAddFromCurrent();
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
        <div className="modal-body">
          <p className="muted">
            Imports whichever account Claude Code is currently signed into.
            Log in with <code>claude auth login</code> first if needed.
          </p>
          <p className="muted small">
            For headless or token-based onboarding, use the{" "}
            <code>claudepot</code> CLI &mdash; refresh tokens never enter the
            GUI to avoid leaking secrets through the webview.
          </p>
        </div>
        <div className="modal-actions">
          <button onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className="primary" onClick={submit} disabled={busy}>
            {busy ? "Adding…" : "Add from current"}
          </button>
        </div>
      </div>
    </div>
  );
}

export default App;
