import { useCallback } from "react";
import { RefreshCw, Monitor, Plus, Terminal, Play, LogIn } from "lucide-react";
import type { AccountSummary, UsageMap } from "../types";

function formatResetTime(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function Sidebar({
  accounts,
  usage,
  selectedUuid,
  busyKeys,
  onSelect,
  onAdd,
  onRefresh,
  onSwitchCli,
  onLogin,
  onContextMenu,
}: {
  accounts: AccountSummary[];
  usage: UsageMap;
  selectedUuid: string | null;
  busyKeys: Set<string>;
  onSelect: (uuid: string) => void;
  onAdd: () => void;
  onRefresh: () => void;
  onSwitchCli: (a: AccountSummary) => void;
  onLogin: (a: AccountSummary) => void;
  onContextMenu?: (e: React.MouseEvent, a: AccountSummary) => void;
}) {
  const handleSwitchClick = useCallback(
    (e: React.MouseEvent, a: AccountSummary) => {
      e.stopPropagation();
      onSwitchCli(a);
    },
    [onSwitchCli],
  );

  const handleLoginClick = useCallback(
    (e: React.MouseEvent, a: AccountSummary) => {
      e.stopPropagation();
      onLogin(a);
    },
    [onLogin],
  );

  return (
    <aside className="sidebar">
      <div className="sidebar-header">
        <span className="sidebar-title">Accounts</span>
        <div className="sidebar-actions">
          <button
            className="icon-btn"
            onClick={onRefresh}
            title="Refresh (⌘R)"
            aria-label="Refresh account data"
          >
            <RefreshCw />
          </button>
          <button
            className="icon-btn"
            onClick={onAdd}
            title="Add account (⌘N)"
            aria-label="Add account"
          >
            <Plus />
          </button>
        </div>
      </div>

      <div className="sidebar-list" role="listbox" aria-label="Account list">
        {accounts.map((a) => {
          const active = selectedUuid === a.uuid;
          const tokenKind = a.drift
            ? "bad"
            : a.token_status.startsWith("valid")
            ? "ok"
            : a.token_status === "expired"
            ? "bad"
            : "warn";
          const dotTitle = a.drift
            ? `DRIFT — blob authenticates as ${a.verified_email}`
            : a.token_status;
          const acctUsage = usage[a.uuid];
          const fiveHour = acctUsage?.five_hour ?? null;
          const fiveHourPct = fiveHour?.utilization ?? null;
          const cliBusy = busyKeys.has(`cli-${a.uuid}`);
          const reBusy = busyKeys.has(`re-${a.uuid}`);

          return (
            <div
              key={a.uuid}
              className={`sidebar-item ${active ? "active" : ""}`}
              role="option"
              aria-selected={active}
              onClick={() => onSelect(a.uuid)}
              onContextMenu={
                onContextMenu ? (e) => onContextMenu(e, a) : undefined
              }
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onSelect(a.uuid);
                }
              }}
              tabIndex={0}
            >
              <span className={`status-dot ${tokenKind}`} title={dotTitle} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="sidebar-item-row">
                  <span className="sidebar-item-text">{a.email}</span>
                  <span className="sidebar-item-badges">
                    {a.is_cli_active ? (
                      <Terminal
                        size={13}
                        strokeWidth={2.5}
                        className="slot-icon cli active"
                        aria-label="Active CLI account"
                      />
                    ) : null}
                    {a.is_desktop_active ? (
                      <Monitor
                        size={13}
                        strokeWidth={2.5}
                        className="slot-icon desktop active"
                        aria-label="Active Desktop account"
                      />
                    ) : null}
                    {/* P0.1: Inline switch — one click to switch CLI */}
                    {!a.is_cli_active && a.credentials_healthy && (
                      <button
                        className="sidebar-switch-btn"
                        onClick={(e) => handleSwitchClick(e, a)}
                        disabled={cliBusy}
                        title="Switch CLI to this account"
                        aria-label={`Switch CLI to ${a.email}`}
                      >
                        <Play size={11} strokeWidth={2.5} />
                      </button>
                    )}
                    {!a.credentials_healthy && !a.is_cli_active && (
                      <button
                        className="sidebar-switch-btn login"
                        onClick={(e) => handleLoginClick(e, a)}
                        disabled={reBusy}
                        title={`Log in as ${a.email}`}
                        aria-label={`Log in as ${a.email}`}
                      >
                        <LogIn size={11} strokeWidth={2} />
                      </button>
                    )}
                  </span>
                </div>
                <div className="sidebar-item-meta">
                  {a.org_name ?? "personal"}
                  {a.subscription_type ? ` · ${a.subscription_type}` : ""}
                </div>
                {/* P0.2: Usage bar with percentage label + reset time */}
                {fiveHourPct !== null && (
                  <div className="usage-bar-row">
                    <div className="usage-bar-container">
                      <div
                        className={`usage-bar-fill ${fiveHourPct >= 80 ? "high" : ""}`}
                        style={{ width: `${Math.min(fiveHourPct, 100)}%` }}
                      />
                    </div>
                    <span className={`usage-bar-label ${fiveHourPct >= 80 ? "high" : ""}`}>
                      {Math.round(fiveHourPct)}%
                      {fiveHour?.resets_at && (
                        <> · resets {formatResetTime(fiveHour.resets_at)}</>
                      )}
                    </span>
                  </div>
                )}
              </div>
            </div>
          );
        })}
      </div>

      {accounts.length === 0 && (
        <div className="sidebar-footer">
          <p className="muted" style={{ fontSize: 11, textAlign: "center", padding: "8px 0" }}>
            No accounts yet
          </p>
        </div>
      )}
    </aside>
  );
}
