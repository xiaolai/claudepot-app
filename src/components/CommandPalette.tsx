import { useState, useCallback, useEffect, useRef, useMemo } from "react";
import { Search, Terminal, Monitor, UserPlus, RefreshCw, Trash } from "lucide-react";
import type { AccountSummary, AppStatus } from "../types";

export interface PaletteAction {
  id: string;
  label: string;
  detail?: string;
  icon: React.ReactNode;
  category: "switch" | "action";
  disabled?: boolean;
  onSelect: () => void;
}

function fuzzyMatch(query: string, text: string): boolean {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (t.includes(q)) return true;
  let qi = 0;
  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) qi++;
  }
  return qi === q.length;
}

export function CommandPalette({
  accounts,
  status,
  onClose,
  onSwitchCli,
  onSwitchDesktop,
  onAdd,
  onRefresh,
  onRemove,
}: {
  accounts: AccountSummary[];
  status: AppStatus;
  onClose: () => void;
  onSwitchCli: (a: AccountSummary) => void;
  onSwitchDesktop: (a: AccountSummary) => void;
  onAdd: () => void;
  onRefresh: () => void;
  onRemove: (a: AccountSummary) => void;
}) {
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // Build action list
  const actions = useMemo(() => {
    const items: PaletteAction[] = [];

    // Quick switch — CLI
    for (const a of accounts) {
      if (!a.is_cli_active && a.credentials_healthy) {
        items.push({
          id: `cli-${a.uuid}`,
          label: `Switch CLI to ${a.email}`,
          detail: a.org_name ?? "personal",
          icon: <Terminal size={14} />,
          category: "switch",
          onSelect: () => onSwitchCli(a),
        });
      }
    }

    // Quick switch — Desktop
    for (const a of accounts) {
      if (
        !a.is_desktop_active &&
        a.has_desktop_profile &&
        status.desktop_installed
      ) {
        items.push({
          id: `desk-${a.uuid}`,
          label: `Switch Desktop to ${a.email}`,
          detail: a.org_name ?? "personal",
          icon: <Monitor size={14} />,
          category: "switch",
          onSelect: () => onSwitchDesktop(a),
        });
      }
    }

    // Global actions
    items.push({
      id: "add",
      label: "Add account",
      icon: <UserPlus size={14} />,
      category: "action",
      onSelect: onAdd,
    });

    items.push({
      id: "refresh",
      label: "Refresh all",
      icon: <RefreshCw size={14} />,
      category: "action",
      onSelect: onRefresh,
    });

    // Remove actions
    for (const a of accounts) {
      items.push({
        id: `rm-${a.uuid}`,
        label: `Remove ${a.email}`,
        detail: a.org_name ?? "personal",
        icon: <Trash size={14} />,
        category: "action",
        onSelect: () => onRemove(a),
      });
    }

    return items;
  }, [accounts, status, onSwitchCli, onSwitchDesktop, onAdd, onRefresh, onRemove]);

  // Filter
  const filtered = useMemo(() => {
    if (!query.trim()) return actions;
    return actions.filter(
      (a) =>
        fuzzyMatch(query, a.label) ||
        (a.detail && fuzzyMatch(query, a.detail)),
    );
  }, [query, actions]);

  // Reset selection on filter change
  useEffect(() => {
    setSelectedIndex(0);
  }, [filtered.length]);

  // Scroll selected into view
  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const selected = list.children[selectedIndex] as HTMLElement | undefined;
    selected?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  // Focus input on mount
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1));
          break;
        case "ArrowUp":
          e.preventDefault();
          setSelectedIndex((i) => Math.max(i - 1, 0));
          break;
        case "Enter":
          e.preventDefault();
          if (filtered[selectedIndex]) {
            filtered[selectedIndex].onSelect();
            onClose();
          }
          break;
        case "Escape":
          e.preventDefault();
          onClose();
          break;
      }
    },
    [filtered, selectedIndex, onClose],
  );

  // Group filtered items
  const switchItems = filtered.filter((a) => a.category === "switch");
  const actionItems = filtered.filter((a) => a.category === "action");

  let globalIdx = 0;

  return (
    <div className="palette-backdrop" onClick={onClose}>
      <div
        className="palette"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown}
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
      >
        <div className="palette-input-row">
          <Search size={16} className="palette-search-icon" />
          <input
            ref={inputRef}
            className="palette-input"
            type="text"
            placeholder="Search accounts, actions…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label="Search"
          />
          <kbd className="palette-kbd">esc</kbd>
        </div>

        <div className="palette-list" ref={listRef} role="listbox">
          {filtered.length === 0 && (
            <div className="palette-empty">No matches</div>
          )}

          {switchItems.length > 0 && (
            <>
              <div className="palette-group-label">Quick Switch</div>
              {switchItems.map((item) => {
                const idx = globalIdx++;
                return (
                  <button
                    key={item.id}
                    className={`palette-item ${idx === selectedIndex ? "selected" : ""}`}
                    role="option"
                    aria-selected={idx === selectedIndex}
                    onClick={() => {
                      item.onSelect();
                      onClose();
                    }}
                    onMouseEnter={() => setSelectedIndex(idx)}
                    disabled={item.disabled}
                  >
                    {item.icon}
                    <span className="palette-item-label">{item.label}</span>
                    {item.detail && (
                      <span className="palette-item-detail">{item.detail}</span>
                    )}
                  </button>
                );
              })}
            </>
          )}

          {actionItems.length > 0 && (
            <>
              <div className="palette-group-label">Actions</div>
              {actionItems.map((item) => {
                const idx = globalIdx++;
                return (
                  <button
                    key={item.id}
                    className={`palette-item ${idx === selectedIndex ? "selected" : ""}`}
                    role="option"
                    aria-selected={idx === selectedIndex}
                    onClick={() => {
                      item.onSelect();
                      onClose();
                    }}
                    onMouseEnter={() => setSelectedIndex(idx)}
                    disabled={item.disabled}
                  >
                    {item.icon}
                    <span className="palette-item-label">{item.label}</span>
                    {item.detail && (
                      <span className="palette-item-detail">{item.detail}</span>
                    )}
                  </button>
                );
              })}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
