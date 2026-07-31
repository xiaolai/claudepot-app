// Draws a `RenderPlan`. Makes no decisions.
//
// Every degenerate-data question — log axis with zeros, min == max,
// all-null series, oversized row counts, high-cardinality categories,
// long labels — is already answered in `claudepot_core::board::widget`
// and arrives resolved. Re-deciding any of it here would mean deciding
// it again in every future surface that renders a board, and the two
// would drift.
//
// What this file IS responsible for: never presenting a resolved
// limitation as if it were the whole picture. A downsampled chart says
// so; a truncated table says so; a collapsed category bucket says so.

import type { RenderPlan, ResolvedWidget } from "../../types";
import { Table, Td, Th, Tr } from "../../components/primitives/Table";

// SVG viewBox units, not CSS lengths — the element is sized by
// `height: var(--board-spark-h)` and stretched by
// `preserveAspectRatio="none"`, so these are a coordinate space rather
// than design values and stay literals on purpose.
const SPARK_VIEWBOX_W = 280;
const SPARK_VIEWBOX_H = 72;

function Frame({
  title,
  full,
  note,
  children,
}: {
  title: string;
  full?: string;
  note?: string;
  children: React.ReactNode;
}) {
  return (
    <section
      style={{
        border: "var(--bw-hair) solid var(--border)",
        borderRadius: "var(--radius-sm)",
        background: "var(--bg-raised)",
        padding: "var(--sp-12)",
        minWidth: 0,
      }}
    >
      <header
        style={{
          fontSize: "var(--fs-sm)",
          fontWeight: 600,
          marginBottom: "var(--sp-8)",
        }}
        title={full}
      >
        {title}
      </header>
      {note && (
        // Never a silent truncation: if the render is partial, the
        // frame says how partial.
        <p
          style={{
            margin: "0 0 var(--sp-8)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-muted)",
          }}
        >
          {note}
        </p>
      )}
      {children}
    </section>
  );
}

/** Minimal inline sparkline. Gaps break the path; they never draw zero. */
function Spark({
  points,
  min,
  max,
}: {
  points: { x: string; y: number | null }[];
  min: number;
  max: number;
}) {
  const w = SPARK_VIEWBOX_W;
  const h = SPARK_VIEWBOX_H;
  const span = max - min || 1;
  // Build one path per contiguous run of non-null values. A single
  // path with gaps skipped would join across a hole and invent data.
  const runs: string[] = [];
  let current: string[] = [];
  points.forEach((p, i) => {
    if (p.y === null) {
      if (current.length) runs.push(current.join(" "));
      current = [];
      return;
    }
    const x = points.length === 1 ? w / 2 : (i / (points.length - 1)) * w;
    const y = h - ((p.y - min) / span) * h;
    current.push(`${current.length === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`);
  });
  if (current.length) runs.push(current.join(" "));

  // A run of one point is just `M x,y`, which paints nothing at all —
  // a single-sample series rendered as a blank chart. Repeat the point
  // so round linecaps draw a dot.
  const drawable = runs.map((d) =>
    d.includes("L") ? d : `${d} L${d.slice(1)}`,
  );

  return (
    <svg
      viewBox={`0 0 ${w} ${h}`}
      preserveAspectRatio="none"
      style={{ width: "100%", height: "var(--board-spark-h)", display: "block" }}
      role="img"
      aria-label={`${points.length} points from ${min.toFixed(2)} to ${max.toFixed(2)}`}
    >
      {drawable.map((d, i) => (
        <path
          key={i}
          d={d}
          fill="none"
          stroke="var(--accent)"
          strokeWidth="var(--board-spark-stroke-w)"
          strokeLinecap="round"
          vectorEffect="non-scaling-stroke"
        />
      ))}
    </svg>
  );
}

function Bars({ points, max }: { points: { x: string; y: number | null }[]; max: number }) {
  const span = max || 1;
  return (
    <div style={{ display: "grid", gap: "var(--sp-4)" }}>
      {points.slice(0, 12).map((p) => (
        <div
          key={p.x}
          style={{ display: "flex", alignItems: "center", gap: "var(--sp-8)" }}
        >
          <span
            style={{
              width: "var(--board-bar-label-w)",
              fontSize: "var(--fs-xs)",
              color: "var(--fg-muted)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
            title={p.x}
          >
            {p.x}
          </span>
          <span
            style={{
              flex: 1,
              height: "var(--board-bar-h)",
              background: "var(--bg-sunken)",
              borderRadius: "var(--radius-xs)",
              overflow: "hidden",
            }}
          >
            <span
              style={{
                display: "block",
                height: "100%",
                width: `${Math.max(0, ((p.y ?? 0) / span) * 100)}%`,
                background: "var(--accent)",
              }}
            />
          </span>
          <span style={{ fontSize: "var(--fs-xs)", minWidth: "var(--board-bar-value-w)", textAlign: "right" }}>
            {p.y === null ? "—" : p.y.toFixed(2)}
          </span>
        </div>
      ))}
    </div>
  );
}

function scaleNote(plan: Extract<RenderPlan, { kind: "line" | "bar" }>): string | undefined {
  const parts: string[] = [];
  // Discriminated narrowing, not a cast. The cast that used to be here
  // hid the fact that the wire shape did not match this type at all.
  const scale = plan.y_axis.scale;
  if (scale.kind === "log_fell_back_to_linear") {
    // The spec asked for log and the data made it impossible. Say so
    // rather than silently drawing a different chart.
    parts.push(`log axis unavailable — ${scale.reason}`);
  }
  if (plan.y_axis.padded) {
    parts.push("every value is identical; range padded to stay readable");
  }
  if (plan.kind === "line" && plan.downsampled_from !== null) {
    parts.push(
      `showing ${plan.points.length} of ${plan.downsampled_from} points`,
    );
  }
  if (plan.kind === "bar" && plan.collapsed_categories !== null) {
    parts.push(`${plan.collapsed_categories} smaller categories grouped`);
  }
  return parts.length ? parts.join(" · ") : undefined;
}

export function WidgetView({ widget }: { widget: ResolvedWidget }) {
  const { plan } = widget;
  const title = widget.title.text;
  const full = widget.title.full;

  if (plan.kind === "empty") {
    return (
      <Frame title={title} full={full}>
        <p style={{ margin: 0, color: "var(--fg-muted)", fontSize: "var(--fs-sm)" }}>
          {plan.reason}
        </p>
      </Frame>
    );
  }

  if (plan.kind === "kpi") {
    return (
      <Frame
        title={title}
        full={full}
        note={`over ${plan.sample_size} value${plan.sample_size === 1 ? "" : "s"}`}
      >
        <div style={{ fontSize: "var(--fs-xl)", fontWeight: 600 }}>
          {plan.value === null ? "—" : plan.value.toLocaleString()}
        </div>
      </Frame>
    );
  }

  if (plan.kind === "line") {
    return (
      <Frame title={title} full={full} note={scaleNote(plan)}>
        <Spark points={plan.points} min={plan.y_axis.min} max={plan.y_axis.max} />
      </Frame>
    );
  }

  if (plan.kind === "bar") {
    return (
      <Frame title={title} full={full} note={scaleNote(plan)}>
        <Bars points={plan.points} max={plan.y_axis.max} />
      </Frame>
    );
  }

  // Count what is actually painted. Basing this on `plan.rows.length`
  // meant a 500-row plan rendered 50 rows and claimed to be complete.
  const TABLE_RENDER_LIMIT = 50;
  const shown = Math.min(plan.rows.length, TABLE_RENDER_LIMIT);
  return (
    <Frame
      title={title}
      full={full}
      note={
        shown < plan.total_rows
          ? `showing ${shown} of ${plan.total_rows} rows`
          : undefined
      }
    >
      <Table density="compact">
        <thead>
          <Tr>
            {plan.headers.map((h) => (
              <Th key={h.text} title={h.full}>
                {h.text}
              </Th>
            ))}
          </Tr>
        </thead>
        <tbody>
          {plan.rows.slice(0, TABLE_RENDER_LIMIT).map((r, i) => (
            <Tr key={i}>
              {r.map((cell, j) => (
                <Td key={j}>{cell}</Td>
              ))}
            </Tr>
          ))}
        </tbody>
      </Table>
    </Frame>
  );
}
