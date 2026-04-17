import { Terminal, Monitor, LogIn, Trash2, XCircle } from "lucide-react";
import type { AccountSummary, AppStatus } from "../types";

function getDesktopDisabledHint(a: AccountSummary, desktopInstalled: boolean): string | null {
  if (!a.credentials_healthy) return "Credentials missing — log in first";
  if (!desktopInstalled) return "Desktop app not detected";
  if (!a.has_desktop_profile) return "No Desktop profile — sign in via Desktop first";
  return null;
}

export function AccountActions({
  account: a, status, busyKeys, anyBusy,
  onUseCli, onUseDesktop, onLogin, onCancelLogin, onRemove,
}: {
  account: AccountSummary;
  status: AppStatus;
  busyKeys: Set<string>;
  anyBusy: boolean;
  onUseCli: (a: AccountSummary) => void;
  onUseDesktop: (a: AccountSummary) => void;
  onLogin: (a: AccountSummary) => void;
  onCancelLogin: () => void;
  onRemove: (a: AccountSummary) => void;
}) {
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
    <>
      <div className="detail-actions">
        {a.credentials_healthy ? (
          <button onClick={() => onUseCli(a)} disabled={selfBusy || a.is_cli_active}
            title={a.is_cli_active ? "Already active CLI" : "Use for CLI"}>
            <Terminal size={14} />
            {cliBusy ? "Switching…" : a.is_cli_active ? "Active CLI" : "Use CLI"}
          </button>
        ) : reBusy ? (
          <button onClick={onCancelLogin} className="danger" title="Cancel login">
            <XCircle size={14} /> Cancel login
          </button>
        ) : (
          <button onClick={() => onLogin(a)} disabled={selfBusy} className="warn"
            title={`Sign in as ${a.email}`}>
            <LogIn size={14} /> Log in
          </button>
        )}

        <button onClick={() => onUseDesktop(a)} disabled={desktopDisabled} title={deskTitle}>
          <Monitor size={14} />
          {deskBusy ? "Switching…" : a.is_desktop_active ? "Active Desktop" : "Use Desktop"}
        </button>

        <button onClick={() => onRemove(a)} disabled={selfBusy || anyBusy}
          className="danger" title="Remove account">
          <Trash2 size={14} /> {rmBusy ? "Removing…" : "Remove"}
        </button>
      </div>
      {deskHint && <div className="account-hint">{deskHint}</div>}
    </>
  );
}
