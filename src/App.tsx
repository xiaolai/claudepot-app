import React, { useCallback, useEffect, useRef, useState } from "react";
import { api } from "./api";
import type { AccountSummary, AppStatus } from "./types";
import "./App.css";

type Toast = { id: number; kind: "info" | "error"; text: string };

let toastCounter = 0;

function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busyKeys, setBusyKeys] = useState<Set<string>>(new Set());
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [showAdd, setShowAdd] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<AccountSummary | null>(
    null,
  );
  const [confirmDesktop, setConfirmDesktop] = useState<AccountSummary | null>(
    null,
  );
  const [confirmClear, setConfirmClear] = useState(false);
  const [keychainIssue, setKeychainIssue] = useState<string | null>(null);
  const lastRefreshRef = useRef(0);

  const pushToast = useCallback((kind: Toast["kind"], text: string) => {
    toastCounter += 1;
    const id = toastCounter;
    setToasts((t) => [...t, { id, kind, text }]);
    if (kind === "info") {
      setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 4000);
    }
  }, []);

  const dismissToast = useCallback((id: number) => {
    setToasts((t) => t.filter((x) => x.id !== id));
  }, []);

  const refresh = useCallback(async () => {
    lastRefreshRef.current = Date.now();
    try {
      try {
        await api.syncFromCurrentCc();
        setKeychainIssue(null);
      } catch (e) {
        const msg = `${e}`;
        if (msg.toLowerCase().includes("keychain is locked")) {
          setKeychainIssue(msg);
        } else {
          setKeychainIssue(null);
          // eslint-disable-next-line no-console
          console.warn("sync_from_current_cc failed:", msg);
        }
      }
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

  // WI-1: initial load + window-focus refresh (debounced 2s)
  useEffect(() => {
    refresh();
    const onFocus = () => {
      if (Date.now() - lastRefreshRef.current > 2000) {
        refresh();
      }
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [refresh]);

  // WI-2: per-account busy states
  const withBusy = async <T,>(key: string, fn: () => Promise<T>) => {
    setBusyKeys((prev) => new Set(prev).add(key));
    try {
      return await fn();
    } finally {
      setBusyKeys((prev) => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }
  };

  const anyBusy = busyKeys.size > 0;

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

  // WI-5: CLI Clear
  const performClearCli = async () => {
    setBusyKeys((prev) => new Set(prev).add("cli-clear"));
    try {
      await api.cliClear();
      pushToast("info", "CLI signed out.");
      await refresh();
    } catch (e) {
      pushToast("error", `Clear CLI failed: ${e}`);
    } finally {
      setBusyKeys((prev) => {
        const next = new Set(prev);
        next.delete("cli-clear");
        return next;
      });
    }
  };

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
          {/* WI-1: refresh button */}
          <button
            className="refresh-btn"
            onClick={refresh}
            title="Refresh account data"
            aria-label="Refresh"
          >
            ↻
          </button>
          <ActivePill label="CLI" email={status.cli_active_email} />
          <ActivePill
            label="Desktop"
            email={status.desktop_active_email}
            disabled={!status.desktop_installed}
            disabledHint="Desktop not installed"
          />
        </div>
      </header>

      {keychainIssue && (
        <div className="banner warn" role="alert">
          <div>
            <strong>macOS Keychain is locked.</strong> Claudepot can't read
            credentials until you unlock it. Click <em>Unlock</em> — macOS
            will show its standard password prompt (your password goes to
            macOS, not Claudepot).
          </div>
          <div className="banner-actions">
            <button
              className="primary"
              onClick={async () => {
                try {
                  await api.unlockKeychain();
                  await refresh();
                } catch (e) {
                  pushToast("error", `Unlock failed: ${e}`);
                }
              }}
            >
              Unlock
            </button>
            <button onClick={refresh}>Retry</button>
          </div>
        </div>
      )}

      <section className="accounts">
        {accounts.length === 0 ? (
          <EmptyState onAdd={() => setShowAdd(true)} />
        ) : (
          accounts.map((a) => (
            <AccountCard
              key={a.uuid}
              account={a}
              desktopAvailable={status.desktop_installed}
              busyKeys={busyKeys}
              anyBusy={anyBusy}
              onUseCli={() => useCli(a)}
              onUseDesktop={() => setConfirmDesktop(a)}
              onLogin={() => login(a)}
              onCancelLogin={cancelLogin}
              onRemove={() => setConfirmRemove(a)}
            />
          ))
        )}
      </section>

      <footer className="footer">
        <button className="primary" onClick={() => setShowAdd(true)}>
          + Add account
        </button>
        {/* WI-5: Clear CLI button */}
        {status.cli_active_email && (
          <button
            className="danger"
            onClick={() => setConfirmClear(true)}
            disabled={anyBusy}
            title="Sign CC out — clears credentials file"
          >
            Clear CLI
          </button>
        )}
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

      {/* WI-6: Desktop switch confirmation */}
      {confirmDesktop && (
        <ConfirmDialog
          title="Switch Desktop?"
          body={
            <p>
              Switch Desktop to <strong>{confirmDesktop.email}</strong>?
              Claude Desktop will quit and relaunch.
            </p>
          }
          confirmLabel="Switch"
          onCancel={() => setConfirmDesktop(null)}
          onConfirm={async () => {
            const target = confirmDesktop;
            setConfirmDesktop(null);
            await useDesktop(target);
          }}
        />
      )}

      {/* WI-5: Clear CLI confirmation */}
      {confirmClear && (
        <ConfirmDialog
          title="Sign out of Claude Code?"
          body={
            <p>
              This clears CC's credentials file. You'll need to sign in
              again to use Claude Code.
            </p>
          }
          confirmLabel="Clear"
          confirmDanger
          onCancel={() => setConfirmClear(false)}
          onConfirm={async () => {
            setConfirmClear(false);
            await performClearCli();
          }}
        />
      )}

      <div className="toasts" aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className={`toast ${t.kind}`}>
            <span className="toast-text">{t.text}</span>
            {/* WI-8: close button on all toasts, error toasts persist */}
            <button
              className="toast-close"
              onClick={() => dismissToast(t.id)}
              aria-label="Dismiss"
              title="Dismiss"
            >
              ×
            </button>
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

// WI-9: compute inline disabled reason for an account card
function getDisabledHint(
  a: AccountSummary,
  desktopAvailable: boolean,
): string | null {
  if (!a.credentials_healthy) return "Credentials missing — log in to restore";
  if (!desktopAvailable) return "Desktop app not detected";
  if (!a.has_desktop_profile)
    return "No Desktop profile — sign in via Desktop first";
  return null;
}

function AccountCard({
  account: a,
  desktopAvailable,
  busyKeys,
  anyBusy,
  onUseCli,
  onUseDesktop,
  onLogin,
  onCancelLogin,
  onRemove,
}: {
  account: AccountSummary;
  desktopAvailable: boolean;
  busyKeys: Set<string>;
  anyBusy: boolean;
  onUseCli: () => void;
  onUseDesktop: () => void;
  onLogin: () => void;
  onCancelLogin: () => void;
  onRemove: () => void;
}) {
  // WI-2: per-account busy
  const cliBusy = busyKeys.has(`cli-${a.uuid}`);
  const deskBusy = busyKeys.has(`desk-${a.uuid}`);
  const reBusy = busyKeys.has(`re-${a.uuid}`);
  const rmBusy = busyKeys.has(`rm-${a.uuid}`);
  // Only add/remove require global lock
  const selfBusy = cliBusy || deskBusy || reBusy || rmBusy;

  const desktopDisabled =
    selfBusy ||
    a.is_desktop_active ||
    !desktopAvailable ||
    !a.has_desktop_profile;

  // WI-9: inline hint
  const hint =
    !a.is_desktop_active && desktopDisabled && !selfBusy
      ? getDisabledHint(a, desktopAvailable)
      : null;

  return (
    <article
      className={`account ${a.is_cli_active ? "cli-active" : ""} ${
        a.is_desktop_active ? "desktop-active" : ""
      }`}
      aria-current={a.is_cli_active || a.is_desktop_active ? "true" : undefined}
    >
      <div className="account-main">
        <div className="account-head">
          <h3>{a.email}</h3>
          {/* WI-7: active-slot badges */}
          {a.is_cli_active && <span className="slot-badge cli">CLI</span>}
          {a.is_desktop_active && (
            <span className="slot-badge desktop">Desktop</span>
          )}
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
            disabled={selfBusy || a.is_cli_active}
            title={a.is_cli_active ? "Already active CLI" : "Use for CLI"}
          >
            {cliBusy ? "…" : a.is_cli_active ? "✓ CLI" : "Use CLI"}
          </button>
        ) : reBusy ? (
          <button
            onClick={onCancelLogin}
            className="danger"
            title="Cancel the in-flight browser login"
          >
            Cancel login
          </button>
        ) : (
          <button
            onClick={onLogin}
            disabled={selfBusy}
            className="warn"
            title={`Sign in as ${a.email} — opens the browser, imports credentials.`}
          >
            Log in
          </button>
        )}
        <button
          onClick={onUseDesktop}
          disabled={desktopDisabled}
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
          disabled={selfBusy || anyBusy}
          className="danger"
          title="Remove account, credentials, and profile"
        >
          {rmBusy ? "…" : "Remove"}
        </button>
      </div>
      {/* WI-9: inline disabled reason */}
      {hint && <div className="account-hint muted">{hint}</div>}
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
  // WI-14: descriptive tooltip
  const title =
    kind === "ok"
      ? `Access token valid, expires in ${mins ?? "?"} minutes. Auto-refreshes on switch.`
      : status === "expired"
      ? "Access token expired. Will refresh automatically on next switch."
      : status === "missing" || status === "no credentials"
      ? "No stored credentials. Log in to restore."
      : "Credential file is corrupt. Log in to restore.";
  return (
    <span className={`token-badge ${kind}`} title={title}>
      {label}
    </span>
  );
}

// WI-4: Escape key closes modals
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
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

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

// WI-4 + WI-13: Escape key + aria attributes
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

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

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
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="add-account-title"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 id="add-account-title">Add account</h2>
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
