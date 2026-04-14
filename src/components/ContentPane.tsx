import {
  Terminal,
  Desktop,
  SignIn,
  Trash,
  SignOut,
  XCircle,
  UserPlus,
} from "@phosphor-icons/react";
import type { AccountSummary, AccountUsage, AppStatus } from "../types";
import { AccountDetail } from "./AccountDetail";
import { CopyButton } from "./CopyButton";

function getDesktopDisabledHint(a: AccountSummary, desktopInstalled: boolean): string | null {
  if (!a.credentials_healthy) return "Credentials missing — log in first";
  if (!desktopInstalled) return "Desktop app not detected";
  if (!a.has_desktop_profile) return "No Desktop profile — sign in via Desktop first";
  return null;
}

export function ContentPane({
  account,
  usage,
  status,
  busyKeys,
  anyBusy,
  onUseCli,
  onUseDesktop,
  onLogin,
  onCancelLogin,
  onRemove,
  onClearCli,
  onAdd,
}: {
  account: AccountSummary | null;
  usage: AccountUsage | null;
  status: AppStatus;
  busyKeys: Set<string>;
  anyBusy: boolean;
  onUseCli: (a: AccountSummary) => void;
  onUseDesktop: (a: AccountSummary) => void;
  onLogin: (a: AccountSummary) => void;
  onCancelLogin: () => void;
  onRemove: (a: AccountSummary) => void;
  onClearCli: () => void;
  onAdd: () => void;
}) {
  if (!account) {
    return (
      <div className="empty">
        <UserPlus size={32} weight="thin" />
        <h2>Select an account</h2>
        <p className="muted">
          Choose an account from the sidebar, or add one to get started.
        </p>
        <button className="primary" onClick={onAdd}>Add account</button>
      </div>
    );
  }

  const a = account;
  const cliBusy = busyKeys.has(`cli-${a.uuid}`);
  const deskBusy = busyKeys.has(`desk-${a.uuid}`);
  const reBusy = busyKeys.has(`re-${a.uuid}`);
  const rmBusy = busyKeys.has(`rm-${a.uuid}`);
  const selfBusy = cliBusy || deskBusy || reBusy || rmBusy;

  const desktopDisabled = selfBusy || a.is_desktop_active ||
    !status.desktop_installed || !a.has_desktop_profile;
  const deskHint = !a.is_desktop_active && desktopDisabled && !selfBusy
    ? getDesktopDisabledHint(a, status.desktop_installed) : null;

  const deskTitle = !status.desktop_installed ? "Desktop not installed"
    : a.is_desktop_active ? "Already active"
    : !a.has_desktop_profile ? "No Desktop profile — sign in via Desktop first"
    : "Switch Desktop to this account";

  return (
    <div className="account-detail-pane">
      {/* Header */}
      <div>
        <div className="detail-header">
          <div>
            <h2 className="detail-email">{a.email}</h2>
            <div className="detail-meta">
              {a.org_name ?? "personal"} · {a.subscription_type ?? "—"}
            </div>
          </div>
          <div className="detail-badges">
            {a.is_cli_active && <span className="slot-badge cli">CLI</span>}
            {a.is_desktop_active && <span className="slot-badge desktop">Desktop</span>}
          </div>
        </div>
      </div>

      {/* Actions */}
      <div className="detail-actions">
        {a.credentials_healthy ? (
          <button onClick={() => onUseCli(a)} disabled={selfBusy || a.is_cli_active}
            title={a.is_cli_active ? "Already active CLI" : "Use for CLI"}>
            <Terminal size={14} weight="light" style={{ marginRight: 4, verticalAlign: -2 }} />
            {cliBusy ? "Switching…" : a.is_cli_active ? "Active CLI" : "Use CLI"}
          </button>
        ) : reBusy ? (
          <button onClick={onCancelLogin} className="danger"
            title="Cancel login">
            <XCircle size={14} weight="light" style={{ marginRight: 4, verticalAlign: -2 }} />
            Cancel login
          </button>
        ) : (
          <button onClick={() => onLogin(a)} disabled={selfBusy} className="warn"
            title={`Sign in as ${a.email}`}>
            <SignIn size={14} weight="light" style={{ marginRight: 4, verticalAlign: -2 }} />
            Log in
          </button>
        )}

        <button onClick={() => onUseDesktop(a)} disabled={desktopDisabled}
          title={deskTitle}>
          <Desktop size={14} weight="light" style={{ marginRight: 4, verticalAlign: -2 }} />
          {deskBusy ? "Switching…" : a.is_desktop_active ? "Active Desktop" : "Use Desktop"}
        </button>

        <button onClick={() => onRemove(a)} disabled={selfBusy || anyBusy}
          className="danger" title="Remove account">
          <Trash size={14} weight="light" style={{ marginRight: 4, verticalAlign: -2 }} />
          {rmBusy ? "Removing…" : "Remove"}
        </button>
      </div>

      {deskHint && <div className="account-hint">{deskHint}</div>}

      {/* Details */}
      <AccountDetail account={a} usage={usage} />

      {/* Footer */}
      <div className="content-footer">
        {status.cli_active_email && (
          <button className="danger" onClick={onClearCli}
            disabled={anyBusy} title="Sign CC out — clears credentials file">
            <SignOut size={14} weight="light" style={{ marginRight: 4, verticalAlign: -2 }} />
            Clear CLI
          </button>
        )}
        <span className="muted mono selectable">
          {status.data_dir} <CopyButton text={status.data_dir} />
        </span>
      </div>
    </div>
  );
}
