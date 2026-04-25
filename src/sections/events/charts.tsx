// Aggregate chart components for the Events section's metrics strip.
//
// These were previously the body of `TrendsSection.tsx` (now removed).
// Promoted into reusable subcomponents so the Events surface can show
// "what's happening overall" inline, instead of behind a separate
// Trends tab the user had to remember to visit.
//
// Pure SVG, no charting library — keeps the bundle small and the
// rendering deterministic. Aggregations run client-side over a
// limit-10000 fetch (cheap at the current ~4k-card scale; if the
// index ever grows past 50k, swap in a server-side aggregate
// command).

import type { ActivityCard, CardKindLabel, SeverityLabel } from "../../types";

// ── Types + aggregation ─────────────────────────────────────────

export interface Aggregates {
  total: number;
  bySeverity: Map<SeverityLabel, number>;
  byKind: Map<CardKindLabel, number>;
  byDay: Map<string, number>;
}

/** Walk a card list and count by every dimension we chart. */
export function aggregate(cards: ActivityCard[]): Aggregates {
  const bySeverity = new Map<SeverityLabel, number>();
  const byKind = new Map<CardKindLabel, number>();
  const byDay = new Map<string, number>();
  for (const c of cards) {
    bump(bySeverity, c.severity);
    bump(byKind, c.kind);
    bump(byDay, isoDay(c.ts_ms));
  }
  return { total: cards.length, bySeverity, byKind, byDay };
}

function bump<K>(m: Map<K, number>, k: K) {
  m.set(k, (m.get(k) ?? 0) + 1);
}

function topN<K>(m: Map<K, number>, n: number): [K, number][] {
  return Array.from(m.entries())
    .sort((a, b) => b[1] - a[1])
    .slice(0, n);
}

/** Last `days` days, filled with zeros where no cards landed, in
 *  oldest→newest order. Drives the sparkbar geometry. */
export function daySeries(byDay: Map<string, number>, days: number): number[] {
  const out: number[] = [];
  const today = new Date();
  for (let i = days - 1; i >= 0; i--) {
    const d = new Date(today);
    d.setDate(d.getDate() - i);
    out.push(byDay.get(isoDayFromDate(d)) ?? 0);
  }
  return out;
}

function isoDay(ms: number): string {
  return isoDayFromDate(new Date(ms));
}

function isoDayFromDate(d: Date): string {
  return d.toISOString().slice(0, 10);
}

// ── Components ──────────────────────────────────────────────────

/** Small horizontal bar chart of cards by day. Bars use currentColor
 *  so they inherit text color from the surrounding card. */
export function Sparkbars({ data }: { data: number[] }) {
  const max = Math.max(1, ...data);
  const w = 8;
  const gap = 2;
  const h = 36;
  return (
    <svg
      role="img"
      aria-label={`Cards per day, last ${data.length} days`}
      width="100%"
      height={h}
      viewBox={`0 0 ${data.length * (w + gap)} ${h}`}
      preserveAspectRatio="none"
      style={{ display: "block" }}
    >
      {data.map((v, i) => {
        const barH = (v / max) * h;
        return (
          <rect
            key={i}
            x={i * (w + gap)}
            y={h - barH}
            width={w}
            height={barH}
            fill="currentColor"
            opacity={v === 0 ? 0.15 : 0.7}
          />
        );
      })}
    </svg>
  );
}

/** Stacked horizontal bar — proportions of ERROR / WARN / NOTICE /
 *  INFO. Counts shown as a small legend underneath. */
export function SeverityMix({ agg }: { agg: Aggregates }) {
  const order: { sev: SeverityLabel; color: string }[] = [
    { sev: "ERROR", color: "var(--danger)" },
    { sev: "WARN", color: "var(--warn)" },
    { sev: "NOTICE", color: "var(--accent)" },
    { sev: "INFO", color: "var(--fg-faint)" },
  ];
  const total = agg.total || 1;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-6)" }}>
      <div
        style={{
          display: "flex",
          width: "100%",
          height: 8,
          borderRadius: "var(--r-1)",
          overflow: "hidden",
          background: "var(--bg-sunken)",
        }}
      >
        {order.map(({ sev, color }) => {
          const n = agg.bySeverity.get(sev) ?? 0;
          if (n === 0) return null;
          return (
            <div
              key={sev}
              style={{ width: `${(n / total) * 100}%`, background: color }}
              title={`${sev}: ${n}`}
            />
          );
        })}
      </div>
      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: "var(--sp-10)",
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-muted)",
          fontVariantNumeric: "tabular-nums",
        }}
      >
        {order.map(({ sev, color }) => (
          <span
            key={sev}
            style={{ display: "inline-flex", alignItems: "center", gap: 4 }}
          >
            <span
              style={{
                display: "inline-block",
                width: 6,
                height: 6,
                borderRadius: 1,
                background: color,
              }}
            />
            {sev} {agg.bySeverity.get(sev) ?? 0}
          </span>
        ))}
      </div>
    </div>
  );
}

/** Compact ranking — top N keys with proportional bars and tabular
 *  counts. Used for "top kinds" in the metrics strip. */
export function TopKinds({
  agg,
  limit,
  labelFor,
}: {
  agg: Aggregates;
  limit: number;
  labelFor: (k: CardKindLabel) => string;
}) {
  const data = topN(agg.byKind, limit);
  if (data.length === 0) {
    return (
      <div style={{ fontSize: "var(--fs-2xs)", color: "var(--fg-muted)" }}>
        No cards.
      </div>
    );
  }
  const max = Math.max(1, ...data.map(([, v]) => v));
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-4)" }}>
      {data.map(([k, v]) => (
        <div key={k} style={{ display: "flex", flexDirection: "column", gap: 1 }}>
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              fontSize: "var(--fs-2xs)",
              color: "var(--fg)",
            }}
          >
            <span
              style={{
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                marginRight: "var(--sp-6)",
              }}
              title={k}
            >
              {labelFor(k)}
            </span>
            <span style={{ fontVariantNumeric: "tabular-nums", color: "var(--fg-muted)" }}>
              {v.toLocaleString()}
            </span>
          </div>
          <div
            style={{
              height: 2,
              borderRadius: 1,
              background: "var(--bg-sunken)",
              overflow: "hidden",
            }}
          >
            <div
              style={{
                width: `${(v / max) * 100}%`,
                height: "100%",
                background: "var(--accent)",
                opacity: 0.7,
              }}
            />
          </div>
        </div>
      ))}
    </div>
  );
}
