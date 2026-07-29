import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Glyph } from "./primitives/Glyph";
import { NF } from "../icons";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { usePaletteActions } from "../hooks/usePaletteActions";
import { useSessionSearch } from "../hooks/useSessionSearch";
import { useProjectSearch, basename } from "../hooks/useProjectSearch";
import { buildPaletteRows, type SelectableRow } from "./palette/rows";
import type { AccountSummary, AppStatus } from "../types";

export function CommandPalette({
  accounts, status, onClose,
  onSwitchCli, onSwitchDesktop, onAdd, onRefresh, onRemove,
  onAdoptDesktop, onClearDesktop, onLaunchDesktop,
  onNavigate, onShowShortcuts, onToggleTheme,
}: {
  accounts: AccountSummary[]; status: AppStatus; onClose: () => void;
  onSwitchCli: (a: AccountSummary) => void;
  onSwitchDesktop: (a: AccountSummary) => void;
  onAdd: () => void; onRefresh: () => void;
  onRemove: (a: AccountSummary) => void;
  /** Desktop adopt from palette — Tier 1: shell-level confirm modal
   * fires when overwrite is needed. */
  onAdoptDesktop?: (a: AccountSummary) => void;
  /** Sign Desktop out — Tier 1 confirm-dialog-gated destructive action. */
  onClearDesktop?: () => void;
  /** Launch Claude Desktop. */
  onLaunchDesktop?: () => void;
  onNavigate?: (section: string, subRoute?: string | null) => void;
  /** Open the global shortcuts reference modal. */
  onShowShortcuts?: () => void;
  /** Flip the light/dark theme. */
  onToggleTheme?: () => void;
}) {
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const trapRef = useFocusTrap<HTMLDivElement>();
  const listboxId = useId();
  const optionIdPrefix = useId();

  const { filter } = usePaletteActions({
    accounts, status, onSwitchCli, onSwitchDesktop, onAdd, onRefresh, onRemove,
    onAdoptDesktop, onClearDesktop, onLaunchDesktop,
    onNavigate, onShowShortcuts, onToggleTheme,
  });

  const actions = filter(query);
  const sessionSearch = useSessionSearch(query);
  const projectSearch = useProjectSearch(query);

  // Built on every render, deliberately un-memoized: `filter` returns
  // a fresh array per call, so a useMemo keyed on it would recompute
  // every render anyway while implying otherwise. The palette holds a
  // few dozen rows and only exists while it's open.
  const { rows, selectable } = buildPaletteRows({
    actions,
    projects: projectSearch.hits,
    projectsLoading: projectSearch.loading,
    sessions: sessionSearch.hits,
    sessionsLoading: sessionSearch.loading,
    query,
  });

  // Snap back to the top whenever the result set changes, so the
  // cursor can't sit past the end of a shorter list.
  useEffect(() => { setSelectedIndex(0); }, [query]);
  useEffect(() => {
    if (selectedIndex >= selectable.length) setSelectedIndex(0);
  }, [selectable.length, selectedIndex]);

  useEffect(() => {
    // Query `.palette-item` specifically — the list also contains
    // `.palette-group-label` dividers. Indexing against every child
    // would scroll a heading into view instead of the selected row.
    const el = listRef.current
      ?.querySelectorAll<HTMLElement>(".palette-item")[selectedIndex];
    el?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);
  useEffect(() => { inputRef.current?.focus(); }, []);

  /**
   * The single activation path. Both Enter and click route through
   * this with the row they are acting on, so the two can never
   * disagree about what a row does.
   */
  const activate = useCallback((row: SelectableRow) => {
    if (row.kind === "action") {
      row.action.onSelect();
    } else if (row.kind === "session") {
      window.dispatchEvent(
        new CustomEvent("cp-goto-session", {
          detail: { filePath: row.hit.file_path },
        }),
      );
    } else {
      window.dispatchEvent(
        new CustomEvent("claudepot:navigate-section", {
          detail: { id: "projects", projectPath: row.project.original_path },
        }),
      );
    }
    onClose();
  }, [onClose]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (selectable.length === 0) return;
      setSelectedIndex((i) => Math.min(i + 1, selectable.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === "Home") {
      e.preventDefault();
      setSelectedIndex(0);
    } else if (e.key === "End") {
      e.preventDefault();
      if (selectable.length > 0) setSelectedIndex(selectable.length - 1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const row = selectable[selectedIndex];
      if (row) activate(row);
    } else if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  }, [selectable, selectedIndex, activate, onClose]);

  const optionId = (index: number) => `${optionIdPrefix}-opt-${index}`;
  const activeId =
    selectable.length > 0 && selectedIndex < selectable.length
      ? optionId(selectedIndex)
      : undefined;

  return (
    // Plain positioned container. The dismiss affordance below is a
    // SIBLING of the dialog, not its ancestor — nesting the dialog's
    // buttons inside a backdrop <button> produced invalid DOM and a
    // React warning on every render.
    <div className="palette-backdrop">
      <button
        type="button"
        className="palette-scrim"
        onClick={onClose}
        aria-label="Close palette"
        tabIndex={-1}
      />
      <div ref={trapRef} className="palette"
        onKeyDown={handleKeyDown} role="dialog" aria-modal="true" aria-label="Command palette">
        <div className="palette-input-row">
          <Glyph g={NF.search} size={16} className="palette-search-icon" />
          {/* Combobox semantics: focus stays in the input while the
              selection moves through the listbox, so the active row is
              announced via aria-activedescendant. Without it a screen
              reader reads nothing as the user arrows down. */}
          <input ref={inputRef} className="palette-input" type="text"
            placeholder="Search accounts, actions, projects…" value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label="Search accounts, actions and projects"
            role="combobox"
            aria-expanded={selectable.length > 0}
            aria-controls={listboxId}
            aria-activedescendant={activeId}
            aria-autocomplete="list"
            autoComplete="off" />
          <kbd className="palette-kbd">esc</kbd>
        </div>
        <div className="palette-list" ref={listRef} id={listboxId} role="listbox"
          aria-label="Commands">
          {rows.map((row) => {
            if (row.kind === "heading") {
              return (
                <div key={row.key} className="palette-group-label" role="presentation">
                  {row.label}{" "}
                  {row.loading && (
                    <span className="palette-group-loading">…searching</span>
                  )}
                </div>
              );
            }
            if (row.kind === "empty") {
              return (
                <div key={row.key} className="palette-empty">{row.text}</div>
              );
            }
            return (
              <PaletteItem
                key={row.key}
                id={optionId(row.index)}
                row={row}
                selected={row.index === selectedIndex}
                onSelect={() => activate(row)}
                onHover={() => setSelectedIndex(row.index)}
              />
            );
          })}
        </div>
      </div>
    </div>
  );
}

function PaletteItem({
  id, row, selected, onSelect, onHover,
}: {
  id: string; row: SelectableRow; selected: boolean;
  onSelect: () => void; onHover: () => void;
}) {
  const { glyph, label, detail, title } = describeRow(row);
  return (
    <button
      id={id}
      className={`palette-item ${selected ? "selected" : ""}`}
      role="option"
      aria-selected={selected}
      onClick={onSelect}
      onMouseEnter={onHover}
      title={title}
    >
      <Glyph g={glyph} size={14} className="palette-item-glyph" />
      <span className="palette-item-label">{label}</span>
      {detail && <span className="palette-item-detail">{detail}</span>}
    </button>
  );
}

/** Row → what the user sees. One place, so every row kind stays
 *  visually consistent. */
function describeRow(row: SelectableRow) {
  if (row.kind === "action") {
    return {
      glyph: row.action.glyph,
      label: row.action.label,
      detail: row.action.detail,
      title: undefined as string | undefined,
    };
  }
  if (row.kind === "session") {
    return {
      glyph: NF.chat,
      label: row.hit.snippet,
      detail: `${row.hit.role} · ${shortSessionId(row.hit.session_id)}`,
      title: undefined as string | undefined,
    };
  }
  // Projects are truncated to their basename in a narrow row, so the
  // full path goes in a tooltip (rules/path-display.md, state B — the
  // canonical copy site is the ProjectDetail header this row opens).
  return {
    glyph: NF.folder,
    label: basename(row.project.original_path),
    detail: `${row.project.session_count} sessions`,
    title: row.project.original_path,
  };
}

function shortSessionId(id: string): string {
  return id.length >= 8 ? id.slice(0, 8) : id;
}
