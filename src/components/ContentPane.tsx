import { LogOut } from "lucide-react";
import type { AccountSummary, AccountUsage, AppStatus } from "../types";
import { AccountDetail } from "./AccountDetail";
import { AccountActions } from "./AccountActions";
import { EmptyState } from "./EmptyState";
import { CopyButton } from "./CopyButton";

export function ContentPane({
  account, usage, status, busyKeys, anyBusy,
  onUseCli, onUseDesktop, onLogin, onCancelLogin, onRemove, onClearCli, onAdd,
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
    return <EmptyState onAdd={onAdd} />;
  }

  return (
    <div className="account-detail-pane">
      <div>
        <div className="detail-header">
          <div>
            <h2 className="detail-email">{account.email}</h2>
            <div className="detail-meta">
              {account.org_name ?? "personal"} · {account.subscription_type ?? "—"}
            </div>
          </div>
          <div className="detail-badges">
            {account.is_cli_active && <span className="slot-badge cli">CLI</span>}
            {account.is_desktop_active && <span className="slot-badge desktop">Desktop</span>}
          </div>
        </div>
      </div>

      <AccountActions
        account={account} status={status} busyKeys={busyKeys} anyBusy={anyBusy}
        onUseCli={onUseCli} onUseDesktop={onUseDesktop} onLogin={onLogin}
        onCancelLogin={onCancelLogin} onRemove={onRemove}
      />

      <AccountDetail account={account} usage={usage} />

      <div className="content-footer">
        {status.cli_active_email && (
          <button className="danger" onClick={onClearCli}
            disabled={anyBusy} title="Sign CC out — clears credentials file">
            <LogOut size={14} /> Clear CLI
          </button>
        )}
        <span className="muted mono selectable">
          {status.data_dir} <CopyButton text={status.data_dir} />
        </span>
      </div>
    </div>
  );
}
