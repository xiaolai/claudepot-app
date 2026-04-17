import { useEffect, useMemo, useRef, useState } from "react";
import { RefreshCw, Plus, Search, X } from "lucide-react";
import type { AccountSummary, UsageMap } from "../types";
import { SidebarAccountItem } from "./SidebarAccountItem";

export function Sidebar({
  accounts, usage, selectedUuid, busyKeys,
  onSelect, onAdd, onRefresh, onSwitchCli, onLogin, onContextMenu,
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
  const [filterText, setFilterText] = useState("");
  const filterRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "f" && !e.shiftKey && !e.altKey) {
        if (filterRef.current) { e.preventDefault(); filterRef.current.focus(); filterRef.current.select(); }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const filtered = useMemo(() => {
    if (!filterText.trim()) return accounts;
    const q = filterText.toLowerCase();
    return accounts.filter(
      (a) => a.email.toLowerCase().includes(q) || (a.org_name?.toLowerCase().includes(q)),
    );
  }, [accounts, filterText]);

  return (
    <aside className="sidebar">
      <div className="sidebar-header">
        <span className="sidebar-title">Accounts</span>
        <div className="sidebar-actions">
          <button className="icon-btn" onClick={onRefresh} title="Refresh (⌘R)" aria-label="Refresh account data">
            <RefreshCw />
          </button>
          <button className="icon-btn" onClick={onAdd} title="Add account (⌘N)" aria-label="Add account">
            <Plus />
          </button>
        </div>
      </div>

      {accounts.length > 3 && (
        <div className="sidebar-search">
          <Search size={12} className="sidebar-search-icon" />
          <input ref={filterRef} className="sidebar-search-input" type="text"
            placeholder="Filter accounts… (⌘F)" value={filterText}
            onChange={(e) => setFilterText(e.target.value)}
            aria-label="Filter accounts" aria-controls="account-listbox" />
          {filterText && (
            <button className="sidebar-search-clear" aria-label="Clear filter" title="Clear"
              onClick={() => { setFilterText(""); filterRef.current?.focus(); }}>
              <X size={10} strokeWidth={2.5} />
            </button>
          )}
        </div>
      )}

      <div className="sidebar-list" role="listbox" id="account-listbox" aria-label="Account list">
        {filtered.map((a) => (
          <SidebarAccountItem
            key={a.uuid}
            account={a}
            active={selectedUuid === a.uuid}
            fiveHour={usage[a.uuid]?.five_hour ?? null}
            cliBusy={busyKeys.has(`cli-${a.uuid}`)}
            reBusy={busyKeys.has(`re-${a.uuid}`)}
            onSelect={() => onSelect(a.uuid)}
            onSwitchCli={() => onSwitchCli(a)}
            onLogin={() => onLogin(a)}
            onContextMenu={onContextMenu ? (e) => onContextMenu(e, a) : undefined}
          />
        ))}
      </div>

      {accounts.length === 0 && (
        <div className="sidebar-footer">
          <p className="muted sidebar-empty-hint">No accounts yet</p>
        </div>
      )}
    </aside>
  );
}
