/**
 * Paper-mono table primitive. Replaces four near-duplicate local
 * Th/Td/Tr helpers (KeysSection, LifecyclePresentational, CostTab,
 * UsageTable) with one source.
 *
 * Density is the only style knob. `default` (sp-6 / sp-10) suits
 * mixed-content rows; `compact` (sp-4 / sp-8) suits dense numeric
 * tables and tight side-panels (RunHistoryPanel, CostTab). Density is
 * threaded via context so callers don't pass it to every cell.
 */
import { createContext, type ReactNode, useContext } from "react";

type Density = "default" | "compact";

const TableDensityContext = createContext<Density>("default");

function cellPadding(density: Density): string {
  return density === "compact"
    ? "var(--sp-4) var(--sp-8)"
    : "var(--sp-6) var(--sp-10)";
}

export interface TableProps {
  children: ReactNode;
  density?: Density;
  /** Optional style escape hatch for table-level overrides
   *  (e.g. caller-supplied font-size). */
  style?: React.CSSProperties;
}

export function Table({ children, density = "default", style }: TableProps) {
  return (
    <TableDensityContext.Provider value={density}>
      <table
        style={{
          width: "100%",
          borderCollapse: "collapse",
          fontSize: "var(--fs-sm)",
          fontVariantNumeric: "tabular-nums",
          ...style,
        }}
      >
        {children}
      </table>
    </TableDensityContext.Provider>
  );
}

export interface ThProps
  extends React.ThHTMLAttributes<HTMLTableCellElement> {
  children?: ReactNode;
  align?: "left" | "right" | "center";
}

export function Th({ children, align, style, title, ...rest }: ThProps) {
  const density = useContext(TableDensityContext);
  return (
    <th
      {...rest}
      title={title}
      className={`mono-cap${rest.className ? ` ${rest.className}` : ""}`}
      style={{
        padding: cellPadding(density),
        textAlign: align ?? "left",
        fontSize: "var(--fs-2xs)",
        fontWeight: 500,
        color: "var(--fg-muted)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
        borderBottom: "var(--bw-hair) solid var(--line)",
        cursor: title ? "help" : undefined,
        ...style,
      }}
    >
      {children}
    </th>
  );
}

export interface ThSortProps<K extends string> {
  value: K;
  current: K;
  /** Direction of the active sort. Omit when this column isn't the
   *  active one. */
  dir?: "asc" | "desc";
  onSort: (k: K) => void;
  children: ReactNode;
  align?: "left" | "right" | "center";
}

export function ThSort<K extends string>({
  value,
  current,
  dir,
  onSort,
  children,
  align,
}: ThSortProps<K>) {
  const density = useContext(TableDensityContext);
  const active = current === value;
  return (
    <th
      role="button"
      tabIndex={0}
      onClick={() => onSort(value)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSort(value);
        }
      }}
      aria-sort={
        active ? (dir === "asc" ? "ascending" : "descending") : "none"
      }
      className="mono-cap"
      style={{
        padding: cellPadding(density),
        textAlign: align ?? "left",
        fontSize: "var(--fs-2xs)",
        fontWeight: active ? 600 : 500,
        color: active ? "var(--fg)" : "var(--fg-muted)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
        borderBottom: "var(--bw-hair) solid var(--line)",
        cursor: "pointer",
        userSelect: "none",
      }}
    >
      {children}
      {active && (
        <span aria-hidden style={{ marginLeft: "var(--sp-4)" }}>
          {dir === "asc" ? "▲" : "▼"}
        </span>
      )}
    </th>
  );
}

export function Tr({
  children,
  style,
}: {
  children: ReactNode;
  style?: React.CSSProperties;
}) {
  return (
    <tr
      style={{
        borderBottom: "var(--bw-hair) solid var(--line)",
        ...style,
      }}
    >
      {children}
    </tr>
  );
}

export interface TdProps
  extends Omit<React.TdHTMLAttributes<HTMLTableCellElement>, "align"> {
  children?: ReactNode;
  align?: "left" | "right" | "center";
  /** Render in muted ink. */
  muted?: boolean;
  /** Right-align + tabular-nums presentation. Implies `align="right"`
   *  unless overridden. */
  num?: boolean;
  /** Bolder weight for the cell that carries the row's primary value. */
  emphasis?: boolean;
}

export function Td({
  children,
  align,
  muted,
  num,
  emphasis,
  style,
  ...rest
}: TdProps) {
  const density = useContext(TableDensityContext);
  return (
    <td
      {...rest}
      style={{
        padding: cellPadding(density),
        textAlign: align ?? (num ? "right" : "left"),
        verticalAlign: "middle",
        color: muted ? "var(--fg-muted)" : "var(--fg)",
        fontWeight: emphasis ? 600 : undefined,
        ...style,
      }}
    >
      {children}
    </td>
  );
}
