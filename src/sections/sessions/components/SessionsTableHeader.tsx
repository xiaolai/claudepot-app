import { Glyph } from "../../../components/primitives/Glyph";
import { NF } from "../../../icons";
import { COLS, type SortDir, type SortKey } from "../sessionsTable.shared";

/**
 * Sticky header row for the sessions table. Columns whose order is
 * meaningful (project, turns, tokens, last-active) are sortable
 * `<button role="columnheader">` controls; the leading glyph and
 * trailing chevron columns are placeholders to keep the grid template
 * in sync with each row.
 */
export function SessionsTableHeader({
  sort,
  onToggle,
}: {
  sort: { key: SortKey; dir: SortDir };
  onToggle: (key: SortKey) => void;
}) {
  return (
    <div
      role="row"
      style={{
        display: "grid",
        gridTemplateColumns: COLS,
        padding: "var(--sp-8) var(--sp-32)",
        fontSize: "var(--fs-xs)",
        color: "var(--fg-faint)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
        gap: "var(--sp-16)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-sunken)",
        alignItems: "center",
        position: "sticky",
        top: 0,
        zIndex: "var(--z-sticky)" as unknown as number,
      }}
    >
      <span />
      <span>Session</span>
      <SortHeader
        label="Project"
        col="project"
        currentKey={sort.key}
        currentDir={sort.dir}
        onToggle={onToggle}
      />
      <SortHeader
        label="Turns"
        col="turns"
        currentKey={sort.key}
        currentDir={sort.dir}
        onToggle={onToggle}
      />
      <SortHeader
        label="Tokens"
        col="tokens"
        currentKey={sort.key}
        currentDir={sort.dir}
        onToggle={onToggle}
      />
      <SortHeader
        label="Last active"
        col="last_active"
        currentKey={sort.key}
        currentDir={sort.dir}
        onToggle={onToggle}
      />
      <span />
    </div>
  );
}

function SortHeader({
  label,
  col,
  currentKey,
  currentDir,
  onToggle,
}: {
  label: string;
  col: SortKey;
  currentKey: SortKey;
  currentDir: SortDir;
  onToggle: (key: SortKey) => void;
}) {
  const active = currentKey === col;
  const aria: "ascending" | "descending" | "none" = active
    ? currentDir === "asc"
      ? "ascending"
      : "descending"
    : "none";
  return (
    <button
      type="button"
      role="columnheader"
      aria-sort={aria}
      onClick={() => onToggle(col)}
      title={`Sort by ${label.toLowerCase()}`}
      style={{
        background: "transparent",
        border: 0,
        padding: 0,
        font: "inherit",
        color: active ? "var(--fg)" : "var(--fg-faint)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
        textAlign: "left",
        cursor: "pointer",
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-4)",
      }}
    >
      <span>{label}</span>
      {active && (
        <Glyph
          g={currentDir === "asc" ? NF.chevronU : NF.chevronD}
          color="var(--fg-muted)"
          style={{ fontSize: "var(--fs-2xs)" }}
        />
      )}
    </button>
  );
}
