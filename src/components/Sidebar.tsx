import { ArrowsClockwise, Desktop, Plus, Terminal } from "@phosphor-icons/react";
import type { AccountSummary, UsageMap } from "../types";

export function Sidebar({
  accounts,
  usage,
  selectedUuid,
  onSelect,
  onAdd,
  onRefresh,
}: {
  accounts: AccountSummary[];
  usage: UsageMap;
  selectedUuid: string | null;
  onSelect: (uuid: string) => void;
  onAdd: () => void;
  onRefresh: () => void;
}) {
  return (
    <aside className="sidebar">
      <div className="sidebar-header">
        <span className="sidebar-title">Accounts</span>
        <div className="sidebar-actions">
          <button
            className="icon-btn"
            onClick={onRefresh}
            title="Refresh"
            aria-label="Refresh account data"
          >
            <ArrowsClockwise />
          </button>
          <button
            className="icon-btn"
            onClick={onAdd}
            title="Add account"
            aria-label="Add account"
          >
            <Plus />
          </button>
        </div>
      </div>

      <div className="sidebar-list" role="listbox" aria-label="Account list">
        {accounts.map((a) => {
          const active = selectedUuid === a.uuid;
          const tokenKind = a.token_status.startsWith("valid")
            ? "ok" : a.token_status === "expired" ? "bad" : "warn";
          const acctUsage = usage[a.uuid];
          const fiveHourPct = acctUsage?.five_hour?.utilization ?? null;

          return (
            <div
              key={a.uuid}
              className={`sidebar-item ${active ? "active" : ""}`}
              role="option"
              aria-selected={active}
              onClick={() => onSelect(a.uuid)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onSelect(a.uuid);
                }
              }}
              tabIndex={0}
            >
              <span className={`status-dot ${tokenKind}`} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div className="sidebar-item-row">
                  <span className="sidebar-item-text">{a.email}</span>
                  <span className="sidebar-item-badges">
                    {a.is_cli_active && (
                      <Terminal
                        size={13}
                        weight="fill"
                        className="slot-icon cli"
                        aria-label="Active CLI account"
                      />
                    )}
                    {a.is_desktop_active && (
                      <Desktop
                        size={13}
                        weight="fill"
                        className="slot-icon desktop"
                        aria-label="Active Desktop account"
                      />
                    )}
                  </span>
                </div>
                <div className="sidebar-item-meta">{a.org_name ?? "personal"}</div>
                {fiveHourPct !== null && (
                  <div className="usage-bar-container" title={`5h usage: ${Math.round(fiveHourPct)}%`}>
                    <div
                      className={`usage-bar-fill ${fiveHourPct >= 80 ? "high" : ""}`}
                      style={{ width: `${Math.min(fiveHourPct, 100)}%` }}
                    />
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
