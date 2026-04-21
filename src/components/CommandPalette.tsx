import { useState, useCallback, useEffect, useRef } from "react";
import { Icon } from "./Icon";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { usePaletteActions, type PaletteAction } from "../hooks/usePaletteActions";
import { useSessionSearch } from "../hooks/useSessionSearch";
import type { AccountSummary, AppStatus, SearchHit } from "../types";

const iconMap = {
  terminal: <Icon name="terminal" size={14} />,
  monitor: <Icon name="monitor" size={14} />,
  "user-plus": <Icon name="user-plus" size={14} />,
  "refresh-cw": <Icon name="refresh" size={14} />,
  trash: <Icon name="trash" size={14} />,
  folder: <Icon name="folder" size={14} />,
  wrench: <Icon name="wrench" size={14} />,
  settings: <Icon name="settings" size={14} />,
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
  onSwitchCli, onSwitchDesktop, onAdd, onRefresh, onRemove, onNavigate,
}: {
  accounts: AccountSummary[]; status: AppStatus; onClose: () => void;
  onSwitchCli: (a: AccountSummary) => void;
  onSwitchDesktop: (a: AccountSummary) => void;
  onAdd: () => void; onRefresh: () => void;
  onRemove: (a: AccountSummary) => void;
  onNavigate?: (section: string, subRoute?: string | null) => void;
}) {
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const trapRef = useFocusTrap<HTMLDivElement>();
  const { filter } = usePaletteActions({
    accounts, status, onSwitchCli, onSwitchDesktop, onAdd, onRefresh, onRemove, onNavigate,
  });

  const filtered = filter(query);
  const sessionSearch = useSessionSearch(query);
  const sessionHits: SearchHit[] = sessionSearch.hits;
  useEffect(() => { setSelectedIndex(0); }, [query]);
  useEffect(() => {
    // Query `.palette-item` specifically — `children` also contains
    // `.palette-group-label` dividers (Quick Switch / Navigate /
    // Actions). Indexing against the full children list scrolls a
    // heading into view instead of the selected command.
    const el = listRef.current
      ?.querySelectorAll<HTMLElement>(".palette-item")[selectedIndex];
    el?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);
  useEffect(() => { inputRef.current?.focus(); }, []);

  const totalItems = filtered.length + sessionHits.length;
  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") { e.preventDefault(); setSelectedIndex((i) => Math.min(i + 1, totalItems - 1)); }
    else if (e.key === "ArrowUp") { e.preventDefault(); setSelectedIndex((i) => Math.max(i - 1, 0)); }
    else if (e.key === "Enter") {
      e.preventDefault();
      if (selectedIndex < filtered.length) {
        const item = filtered[selectedIndex];
        if (item) { item.onSelect(); onClose(); }
      } else {
        const hit = sessionHits[selectedIndex - filtered.length];
        if (hit) {
          window.dispatchEvent(
            new CustomEvent("cp-goto-session", {
              detail: { filePath: hit.file_path },
            }),
          );
          onClose();
        }
      }
    }
    else if (e.key === "Escape") { e.preventDefault(); onClose(); }
  }, [filtered, sessionHits, selectedIndex, totalItems, onClose]);

  const switchItems = filtered.filter((a) => a.category === "switch");
  const navigateItems = filtered.filter((a) => a.category === "navigate");
  const actionItems = filtered.filter((a) => a.category === "action");
  let idx = 0;

  return (
    <div className="palette-backdrop" onClick={onClose}>
      <div ref={trapRef} className="palette" onClick={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown} role="dialog" aria-modal="true" aria-label="Command palette">
        <div className="palette-input-row">
          <Icon name="search" size={16} className="palette-search-icon" />
          <input ref={inputRef} className="palette-input" type="text"
            placeholder="Search accounts, actions…" value={query}
            onChange={(e) => setQuery(e.target.value)} aria-label="Search accounts and actions" />
          <kbd className="palette-kbd">esc</kbd>
        </div>
        <div className="palette-list" ref={listRef} role="listbox">
          {filtered.length === 0 && sessionHits.length === 0 && !sessionSearch.loading && (
            <div className="palette-empty">No matches</div>
          )}
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
          {navigateItems.length > 0 && (
            <>
              <div className="palette-group-label">Navigate</div>
              {navigateItems.map((item) => {
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
          {query.trim().length >= 2 && (
            <>
              <div className="palette-group-label">
                Sessions{" "}
                {sessionSearch.loading && (
                  <span style={{ color: "var(--fg-faint)" }}>…searching</span>
                )}
              </div>
              {/* Empty session section — only show when there are ALSO no
                  action hits, so mixed results aren't labeled "empty". */}
              {sessionHits.length === 0 &&
                !sessionSearch.loading &&
                filtered.length === 0 && (
                  <div className="palette-empty">No session matches</div>
                )}
              {sessionHits.map((hit, hi) => {
                const i = filtered.length + hi;
                return (
                  <SessionHitItem
                    key={hit.file_path + hi}
                    hit={hit}
                    selected={i === selectedIndex}
                    onSelect={() => {
                      window.dispatchEvent(
                        new CustomEvent("cp-goto-session", {
                          detail: { filePath: hit.file_path },
                        }),
                      );
                      onClose();
                    }}
                    onHover={() => setSelectedIndex(i)}
                  />
                );
              })}
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function SessionHitItem({
  hit,
  selected,
  onSelect,
  onHover,
}: {
  hit: SearchHit;
  selected: boolean;
  onSelect: () => void;
  onHover: () => void;
}) {
  return (
    <button
      className={`palette-item ${selected ? "selected" : ""}`}
      role="option"
      aria-selected={selected}
      onClick={onSelect}
      onMouseEnter={onHover}
    >
      <Icon name="folder" size={14} />
      <span className="palette-item-label">
        {hit.snippet}
      </span>
      <span className="palette-item-detail">
        {hit.role} · {shortSessionId(hit.session_id)}
      </span>
    </button>
  );
}

function shortSessionId(id: string): string {
  return id.length >= 8 ? id.slice(0, 8) : id;
}
