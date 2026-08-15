import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { Glyph } from "./Glyph";
import { Input } from "./Input";
import { NF, type NfIcon } from "../../icons";
import { scoreFields } from "../../lib/paletteScore";

/**
 * One row of a {@link FilterPicker}. `label` is what the user reads and
 * types against; `detail` is the secondary line (a path, an id) and is
 * matched too, at the discount `scoreFields` applies.
 */
export interface PickerOption {
  value: string;
  label: string;
  detail?: string;
  /** Leading glyph. Reserved for rows that aren't one of the listed
   *  items — e.g. "use this folder", which is an escape hatch, not a
   *  choice from the set. */
  glyph?: NfIcon;
}

/**
 * Search field over a listbox: the paper-mono answer to "pick one of
 * N", for N large enough that a native `<select>` stops being a picker
 * and becomes a scroll. Seventy-seven projects in an unfiltered dropdown
 * is not a list you read, it is a list you give up on.
 *
 * Combobox semantics, same as the ⌘K palette: focus stays in the input
 * while the cursor moves through the listbox, and the active row is
 * announced through `aria-activedescendant`. Arrow keys move, Enter
 * picks, Escape is deliberately **not** handled — it belongs to the
 * enclosing `Modal`, and swallowing it here would strand the user in a
 * dialog they can't dismiss.
 *
 * Filtering and scoring live here; the caller owns the query so it can
 * derive a `pinned` row from it (see `MoveTargetPicker`, which turns a
 * typed path into a "use this folder" row). `pinned` always renders
 * first and is never filtered out — it is the answer to a query the
 * option set by definition cannot contain.
 */
export function FilterPicker({
  options,
  pinned,
  value,
  onChange,
  query,
  onQueryChange,
  placeholder,
  inputId,
  inputAriaLabel,
  listAriaLabel,
  emptyText,
  disabled,
  autoFocus,
  footer,
}: {
  options: readonly PickerOption[];
  /** Always-first row, exempt from filtering. */
  pinned?: PickerOption | null;
  /** Currently picked `value`, or null for "nothing picked yet". */
  value: string | null;
  onChange: (option: PickerOption) => void;
  query: string;
  onQueryChange: (q: string) => void;
  placeholder?: string;
  /** Set when an enclosing `<label htmlFor>` names the field. The
   *  `inputAriaLabel` still applies — a visible label and an accessible
   *  name are not the same thing when the label is a `mono-cap` stub. */
  inputId?: string;
  inputAriaLabel: string;
  listAriaLabel: string;
  /** Shown in place of the list when the filter matches nothing. */
  emptyText: ReactNode;
  disabled?: boolean;
  autoFocus?: boolean;
  /** Row under the list — counts, a Browse button. */
  footer?: ReactNode;
}) {
  const listboxId = useId();
  const optionIdPrefix = useId();
  const listRef = useRef<HTMLUListElement>(null);

  const filtered = useMemo(() => {
    const q = query.trim();
    if (!q) return [...options];
    return options
      .map((o) => ({ o, score: scoreFields(q, o.label, [o.detail]) }))
      .filter((r): r is { o: PickerOption; score: number } => r.score !== null)
      .sort((a, b) => b.score - a.score)
      .map((r) => r.o);
  }, [options, query]);

  const rows = useMemo(
    () => (pinned ? [pinned, ...filtered] : filtered),
    [pinned, filtered],
  );

  // Cursor index into `rows` — the rendered order, so the row reported
  // as active is the row Enter picks. Same invariant the palette holds.
  const [cursor, setCursor] = useState(0);
  useEffect(() => {
    setCursor(0);
  }, [query]);
  useEffect(() => {
    if (cursor >= rows.length) setCursor(0);
  }, [rows.length, cursor]);
  useEffect(() => {
    listRef.current
      ?.querySelectorAll<HTMLElement>("[role='option']")
      [cursor]?.scrollIntoView({ block: "nearest" });
  }, [cursor]);

  const optionId = (i: number) => `${optionIdPrefix}-opt-${i}`;

  function handleKeyDown(e: React.KeyboardEvent) {
    const last = rows.length - 1;
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        if (last >= 0) setCursor((i) => Math.min(i + 1, last));
        break;
      case "ArrowUp":
        e.preventDefault();
        setCursor((i) => Math.max(i - 1, 0));
        break;
      case "Home":
        e.preventDefault();
        setCursor(0);
        break;
      case "End":
        e.preventDefault();
        if (last >= 0) setCursor(last);
        break;
      case "Enter": {
        const row = rows[cursor];
        if (!row) break;
        // Only claim Enter when it does something. Otherwise it falls
        // through to the modal's default submit, which is what a user
        // who has already picked a target expects.
        e.preventDefault();
        onChange(row);
        break;
      }
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-6)" }}>
      <Input
        glyph={NF.search}
        id={inputId}
        type="text"
        value={query}
        onChange={(e) => onQueryChange(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder={placeholder}
        aria-label={inputAriaLabel}
        role="combobox"
        aria-expanded={rows.length > 0}
        aria-controls={listboxId}
        aria-activedescendant={rows.length > 0 ? optionId(cursor) : undefined}
        aria-autocomplete="list"
        autoComplete="off"
        spellCheck={false}
        disabled={disabled}
        autoFocus={autoFocus}
      />
      <ul
        ref={listRef}
        id={listboxId}
        role="listbox"
        aria-label={listAriaLabel}
        style={{
          listStyle: "none",
          margin: 0,
          padding: 0,
          maxHeight: "var(--list-max-height-sm)",
          overflowY: "auto",
          border: "var(--bw-hair) solid var(--line)",
          borderRadius: "var(--r-2)",
          background: "var(--bg)",
          opacity: disabled ? "var(--opacity-disabled)" : 1,
        }}
      >
        {rows.length === 0 && (
          <li
            role="presentation"
            style={{
              padding: "var(--sp-10) var(--sp-10)",
              fontSize: "var(--fs-xs)",
              color: "var(--fg-faint)",
            }}
          >
            {emptyText}
          </li>
        )}
        {rows.map((o, i) => (
          <PickerRow
            key={o.value}
            id={optionId(i)}
            option={o}
            selected={o.value === value}
            active={i === cursor}
            disabled={disabled}
            onPick={() => {
              setCursor(i);
              onChange(o);
            }}
            onHover={() => setCursor(i)}
          />
        ))}
      </ul>
      {footer}
    </div>
  );
}

/**
 * `<li role="option">` rather than a nested `<button>`: a listbox owner
 * must contain only options, and the row is reached by the combobox
 * cursor rather than by tab order (`tabIndex={-1}`).
 */
function PickerRow({
  id,
  option,
  selected,
  active,
  disabled,
  onPick,
  onHover,
}: {
  id: string;
  option: PickerOption;
  selected: boolean;
  active: boolean;
  disabled?: boolean;
  onPick: () => void;
  onHover: () => void;
}) {
  return (
    <li
      id={id}
      role="option"
      aria-selected={selected}
      // `disabled` here strips the handlers but the element is an <li>,
      // not a <button>, so nothing announces the state. `not-allowed`
      // was carrying it alone — and colour/cursor may not be the only
      // cue (design.md accessibility floor).
      aria-disabled={disabled || undefined}
      tabIndex={-1}
      onClick={disabled ? undefined : onPick}
      onMouseMove={disabled ? undefined : onHover}
      style={{
        display: "flex",
        alignItems: "baseline",
        gap: "var(--sp-8)",
        padding: "var(--sp-6) var(--sp-10)",
        
        // Active (cursor) and selected (picked) are different states and
        // both have to be visible at once: the user arrows past their own
        // current choice all the time.
        background: active ? "var(--accent-soft)" : "transparent",
        borderLeft: `var(--bw-strong) solid ${
          selected ? "var(--accent)" : "transparent"
        }`,
      }}
    >
      {option.glyph && (
        <Glyph
          g={option.glyph}
          style={{ fontSize: "var(--fs-xs)", color: "var(--accent)" }}
        />
      )}
      <span
        className="mono"
        style={{
          fontSize: "var(--fs-sm)",
          color: "var(--fg)",
          flexShrink: 0,
        }}
      >
        {option.label}
      </span>
      {option.detail && (
        <span
          className="mono"
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            minWidth: 0,
            flex: 1,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            // Paths read from the end — the basename is the identity.
            // See .claude/rules/path-display.md.
            direction: "rtl",
            textAlign: "left",
          }}
          title={option.detail}
        >
          {option.detail}
        </span>
      )}
    </li>
  );
}
