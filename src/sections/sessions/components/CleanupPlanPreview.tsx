import type { ReactNode } from "react";

/**
 * One row in the preview. `id` is the React key (a unique stable
 * string — typically the file path). `leftText` is the wide cell
 * (file path or label); `rightText` is the right-aligned numeric
 * cell. `leftTitle` becomes the hover tooltip on the wide cell.
 */
export interface CleanupPreviewRow {
  id: string;
  leftText: string;
  rightText: string;
  leftTitle?: string;
}

interface CleanupPlanPreviewProps {
  /** `data-testid` for tests (e.g. "prune-preview", "slim-preview"). */
  testid: string;
  /** Header summary line — composed by the caller because each plan
   * type has a bespoke set of counters. */
  summaryText: string;
  rows: CleanupPreviewRow[];
  /** Soft cap on visible rows; the rest collapse into a single
   * "… and N more" line. Matches the previous prune + slim cap. */
  maxRows?: number;
  /** Optional extras rendered at the bottom of the frame, after the
   * "and N more" line. Used by the slim panel for the failed-to-plan
   * sub-list. */
  extrasFooter?: ReactNode;
  /** Margin between the frame and whatever sits above. Slim sits
   * inside its own subsection and wants a gap; prune is the first
   * thing in its column and doesn't. */
  marginTop?: string;
}

/**
 * Bordered preview list shared by the prune and bulk-slim cleanup
 * panes. Both previews used to inline the same frame + header +
 * truncating list — extracting them here drops ~140 LOC of
 * duplication and removes the drift risk that comes with maintaining
 * two near-identical blocks.
 */
export function CleanupPlanPreview({
  testid,
  summaryText,
  rows,
  maxRows = 50,
  extrasFooter,
  marginTop,
}: CleanupPlanPreviewProps) {
  const visible = rows.slice(0, maxRows);
  const overflow = rows.length - visible.length;
  return (
    <div
      data-testid={testid}
      style={{
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        overflow: "hidden",
        marginTop,
      }}
    >
      <div
        style={{
          padding: "var(--sp-12) var(--sp-16)",
          background: "var(--bg-sunken)",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
        }}
      >
        {summaryText}
      </div>
      <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
        {visible.map((row) => (
          <li
            key={row.id}
            style={{
              padding: "var(--sp-8) var(--sp-16)",
              borderBottom: "var(--bw-hair) solid var(--line)",
              fontSize: "var(--fs-sm)",
              display: "grid",
              gridTemplateColumns: "1fr auto",
              gap: "var(--sp-16)",
            }}
          >
            <span
              title={row.leftTitle ?? row.leftText}
              style={{
                whiteSpace: "nowrap",
                overflow: "hidden",
                textOverflow: "ellipsis",
              }}
            >
              {row.leftText}
            </span>
            <span
              style={{
                fontVariantNumeric: "tabular-nums",
                color: "var(--fg-muted)",
              }}
            >
              {row.rightText}
            </span>
          </li>
        ))}
      </ul>
      {overflow > 0 && (
        <div
          style={{
            padding: "var(--sp-8) var(--sp-16)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-faint)",
          }}
        >
          … and {overflow} more
        </div>
      )}
      {extrasFooter}
    </div>
  );
}
