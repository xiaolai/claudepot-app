import type { AppStatus } from "../types";
import { ActivePill } from "./ActivePill";

export function Header({
  status,
  onRefresh,
}: {
  status: AppStatus;
  onRefresh: () => void;
}) {
  return (
    <header className="header">
      <div className="brand">
        <h1>Claudepot</h1>
        <span className="muted">
          {status.platform} / {status.arch} · {status.account_count}{" "}
          account{status.account_count === 1 ? "" : "s"}
        </span>
      </div>
      <div className="active-row">
        <button
          className="refresh-btn"
          onClick={onRefresh}
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
  );
}
