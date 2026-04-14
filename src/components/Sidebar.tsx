import { ArrowsClockwise, Plus } from "@phosphor-icons/react";
import type { AccountSummary } from "../types";

export function Sidebar({
  accounts,
  selectedUuid,
  onSelect,
  onAdd,
  onRefresh,
}: {
  accounts: AccountSummary[];
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
                <div className="sidebar-item-text">{a.email}</div>
                <div className="sidebar-item-meta">
                  {a.org_name ?? "personal"}
                  {a.is_cli_active && " · CLI"}
                  {a.is_desktop_active && " · Desktop"}
                </div>
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
