import { useState, useCallback, useEffect, useRef } from "react";
import { Search, Terminal, Monitor, UserPlus, RefreshCw, Trash } from "lucide-react";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { usePaletteActions, type PaletteAction } from "../hooks/usePaletteActions";
import type { AccountSummary, AppStatus } from "../types";

const iconMap = {
  terminal: <Terminal size={14} />,
  monitor: <Monitor size={14} />,
  "user-plus": <UserPlus size={14} />,
  "refresh-cw": <RefreshCw size={14} />,
  trash: <Trash size={14} />,
};

function PaletteItem({
  item, selected, onSelect, onHover,
}: {
  item: PaletteAction; selected: boolean;
  onSelect: () => void; onHover: () => void;
}) {
  return (
    <button
      className={`palette-item ${selected ? "selected" : ""}`}
      role="option"
      aria-selected={selected}
      onClick={onSelect}
      onMouseEnter={onHover}
      disabled={item.disabled}
    >
      {iconMap[item.iconName]}
      <span className="palette-item-label">{item.label}</span>
      {item.detail && <span className="palette-item-detail">{item.detail}</span>}
    </button>
  );
}

export function CommandPalette({
  accounts, status, onClose,
  onSwitchCli, onSwitchDesktop, onAdd, onRefresh, onRemove,
}: {
  accounts: AccountSummary[]; status: AppStatus; onClose: () => void;
  onSwitchCli: (a: AccountSummary) => void;
  onSwitchDesktop: (a: AccountSummary) => void;
  onAdd: () => void; onRefresh: () => void;
  onRemove: (a: AccountSummary) => void;
}) {
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const trapRef = useFocusTrap<HTMLDivElement>();
  const { filter } = usePaletteActions({
    accounts, status, onSwitchCli, onSwitchDesktop, onAdd, onRefresh, onRemove,
  });

  const filtered = filter(query);
  useEffect(() => { setSelectedIndex(0); }, [query]);
  useEffect(() => {
    const el = listRef.current?.children[selectedIndex] as HTMLElement | undefined;
    el?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);
  useEffect(() => { inputRef.current?.focus(); }, []);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") { e.preventDefault(); setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1)); }
    else if (e.key === "ArrowUp") { e.preventDefault(); setSelectedIndex((i) => Math.max(i - 1, 0)); }
    else if (e.key === "Enter") { e.preventDefault(); if (filtered[selectedIndex]) { filtered[selectedIndex].onSelect(); onClose(); } }
    else if (e.key === "Escape") { e.preventDefault(); onClose(); }
  }, [filtered, selectedIndex, onClose]);

  const switchItems = filtered.filter((a) => a.category === "switch");
  const actionItems = filtered.filter((a) => a.category === "action");
  let idx = 0;

  return (
    <div className="palette-backdrop" onClick={onClose}>
      <div ref={trapRef} className="palette" onClick={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown} role="dialog" aria-modal="true" aria-label="Command palette">
        <div className="palette-input-row">
          <Search size={16} className="palette-search-icon" />
          <input ref={inputRef} className="palette-input" type="text"
            placeholder="Search accounts, actions…" value={query}
            onChange={(e) => setQuery(e.target.value)} aria-label="Search accounts and actions" />
          <kbd className="palette-kbd">esc</kbd>
        </div>
        <div className="palette-list" ref={listRef} role="listbox">
          {filtered.length === 0 && <div className="palette-empty">No matches</div>}
          {switchItems.length > 0 && (
            <>
              <div className="palette-group-label">Quick Switch</div>
              {switchItems.map((item) => {
                const i = idx++;
                return <PaletteItem key={item.id} item={item} selected={i === selectedIndex}
                  onSelect={() => { item.onSelect(); onClose(); }} onHover={() => setSelectedIndex(i)} />;
              })}
            </>
          )}
          {actionItems.length > 0 && (
            <>
              <div className="palette-group-label">Actions</div>
              {actionItems.map((item) => {
                const i = idx++;
                return <PaletteItem key={item.id} item={item} selected={i === selectedIndex}
                  onSelect={() => { item.onSelect(); onClose(); }} onHover={() => setSelectedIndex(i)} />;
              })}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
