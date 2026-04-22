import { Glyph } from "../../../components/primitives/Glyph";
import { NF } from "../../../icons";
import { COLS, type SortDir, type SortKey } from "../sessionsTable.shared";

/**
 * Sticky header row for the sessions table. The body below is a
 * `<ul role="listbox">`, so the header is presentational only — we
 * deliberately do NOT use `role="row"` / `role="columnheader"` /
 * `aria-sort` here, because those imply a `grid` / `table` parent
 * and would mislead screen readers about the table's structure.
 *
 * Sort controls are plain `<button>`s that announce the current sort
 * state through `aria-label` ("Sort by project (currently ascending,
 * click to switch to descending)") and a visible direction glyph.
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
  // Clicking advances the cycle: asc → desc → unsorted (default).
  // We surface that explicitly in the label so screen readers
  // announce both the current state and what clicking will do.
  const nextStateLabel = active
    ? currentDir === "asc"
      ? "click to sort descending"
      : "click to clear sort"
    : "click to sort ascending";
  const currentStateLabel = active
    ? currentDir === "asc"
      ? "currently ascending"
      : "currently descending"
    : "not sorted";
  return (
    <button
      type="button"
      onClick={() => onToggle(col)}
      title={`Sort by ${label.toLowerCase()}`}
      aria-label={`Sort by ${label.toLowerCase()} (${currentStateLabel}, ${nextStateLabel})`}
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
