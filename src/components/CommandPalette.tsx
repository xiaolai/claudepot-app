import { useCallback, useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Glyph } from "./primitives/Glyph";
import { NF } from "../icons";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { usePaletteActions } from "../hooks/usePaletteActions";
import { useSessionSearch } from "../hooks/useSessionSearch";
import { useProjectSearch } from "../hooks/useProjectSearch";
import { i18n } from "../lib/i18n";
import { basename } from "../lib/paths";
import { shortSessionId } from "../sections/sessions/format";
import { buildPaletteRows, type SelectableRow } from "./palette/rows";
import { usePaletteSelection } from "./palette/usePaletteSelection";
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
  const { t } = useTranslation("components");
  const [query, setQuery] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
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

  useEffect(() => { inputRef.current?.focus(); }, []);

  /**
   * What running a row means. Both Enter and click reach it through
   * `activate` below, so the two can never disagree about what a row
   * does.
   */
  const runRow = useCallback((row: SelectableRow) => {
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
  }, []);

  const { selectedIndex, setSelectedIndex, activate, handleKeyDown, listRef } =
    usePaletteSelection({ selectable, resetKey: query, onClose, onRun: runRow });

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
        aria-label={t("palette.close")}
        tabIndex={-1}
      />
      <div ref={trapRef} className="palette"
        onKeyDown={handleKeyDown} role="dialog" aria-modal="true"
        aria-label={t("palette.dialogLabel")}>
        <div className="palette-input-row">
          <Glyph g={NF.search} size={16} className="palette-search-icon" />
          {/* Combobox semantics: focus stays in the input while the
              selection moves through the listbox, so the active row is
              announced via aria-activedescendant. Without it a screen
              reader reads nothing as the user arrows down. */}
          <input ref={inputRef} className="palette-input" type="text"
            placeholder={t("palette.placeholder")} value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label={t("palette.inputLabel")}
            role="combobox"
            aria-expanded={selectable.length > 0}
            aria-controls={listboxId}
            aria-activedescendant={activeId}
            aria-autocomplete="list"
            autoComplete="off" />
          <kbd className="palette-kbd">esc</kbd>
        </div>
        <ul className="palette-list" ref={listRef} id={listboxId} role="listbox"
          aria-label={t("palette.listLabel")}>
          {rows.map((row) => {
            if (row.kind === "heading") {
              return (
                <li key={row.key} className="palette-group-label" role="presentation">
                  {row.label}{" "}
                  {row.loading && (
                    <span className="palette-group-loading">
                      {t("palette.searching")}
                    </span>
                  )}
                </li>
              );
            }
            if (row.kind === "empty") {
              return (
                <li key={row.key} className="palette-empty" role="presentation">
                  {row.text}
                </li>
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
        </ul>
      </div>
    </div>
  );
}

/**
 * A row is an `<li role="option">`, deliberately NOT a focusable
 * control.
 *
 * These were `<button>`s, which put every row in the tab order. Tab to
 * a row and press Enter and two things fired: the row's own click
 * (right row) and the dialog's keydown handler resolving
 * `selectable[selectedIndex]` (whatever the arrow-key cursor last
 * pointed at). That is the same "activates a command you weren't
 * looking at" bug this component was rewritten to eliminate, reached
 * by a different key.
 *
 * The combobox pattern owns keyboard input in one place: focus stays
 * in the input, `aria-activedescendant` names the active row, and the
 * rows are pointer targets only. Note this is the ARIA *combobox*
 * pattern, not the standalone listbox in `.claude/rules/design.md`,
 * whose `tabIndex={0}` options are correct only when focus moves into
 * the list itself.
 */
function PaletteItem({
  id, row, selected, onSelect, onHover,
}: {
  id: string; row: SelectableRow; selected: boolean;
  onSelect: () => void; onHover: () => void;
}) {
  const { glyph, label, detail, title } = describeRow(row);
  return (
    // eslint-disable-next-line jsx-a11y/click-events-have-key-events
    <li
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
    </li>
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
    // Plain helper, not a component — reads the global i18n instance.
    // `CommandPalette` subscribes via `useTranslation`, so a language
    // switch re-renders it and re-invokes this.
    detail: i18n.t("palette.projectSessions", {
      ns: "components",
      n: row.project.session_count,
    }),
    title: row.project.original_path,
  };
}
