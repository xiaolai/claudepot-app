import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import type { ActivityCard, CardKindLabel, SeverityLabel } from "../types";

/**
 * Trends — aggregate view of activity cards over time. Separate
 * surface from Events (the per-event stream) per design v2 §9 —
 * "Now" and "Trends" are genuinely different surfaces and a single
 * segmented control was conflating them.
 *
 * v1 charts (pure SVG, no charting lib):
 *   - Cards by day (sparkbar, last 30 days)
 *   - Top templates by count
 *   - Top plugins by count
 *   - Severity mix (horizontal bar)
 *   - Top sessions by failure count
 *
 * Aggregations run client-side over a limit-10000 fetch — cheap at
 * the current scale (~4k cards on the reference machine). If the
 * index ever grows past 50k cards, swap in a server-side aggregate
 * Tauri command.
 */

const FETCH_LIMIT = 10_000;

interface Aggregates {
  total: number;
  bySeverity: Map<SeverityLabel, number>;
  byKind: Map<CardKindLabel, number>;
  byTemplate: Map<string, number>;
  byPlugin: Map<string, number>;
  bySession: Map<string, number>;
  byDay: Map<string, number>;
}

export function TrendsSection() {
  const [cards, setCards] = useState<ActivityCard[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [windowChoice, setWindowChoice] = useState<"7d" | "30d" | "all">("30d");

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    const sinceMs =
      windowChoice === "all"
        ? undefined
        : Date.now() -
          (windowChoice === "7d"
            ? 7 * 24 * 60 * 60 * 1000
            : 30 * 24 * 60 * 60 * 1000);
    api
      .cardsRecent({ sinceMs, limit: FETCH_LIMIT })
      .then((list) => {
        if (cancelled) return;
        setCards(list);
        setLoading(false);
      })
      .catch((e) => {
        if (!cancelled) {
          setError(String(e));
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [windowChoice]);

  const agg = useMemo<Aggregates>(() => aggregate(cards), [cards]);

  return (
    <div
      style={{
        padding: "var(--sp-20)",
        overflowY: "auto",
        height: "100%",
        background: "var(--bg)",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          marginBottom: "var(--sp-20)",
        }}
      >
        <h2
          style={{
            margin: 0,
            fontSize: "var(--fs-md)",
            fontWeight: 600,
            color: "var(--fg)",
          }}
        >
          Trends
        </h2>
        <div style={{ display: "flex", gap: "var(--sp-8)" }}>
          {(["7d", "30d", "all"] as const).map((w) => (
            <button
              key={w}
              onClick={() => setWindowChoice(w)}
              style={{
                ...btnStyle,
                background:
                  windowChoice === w ? "var(--bg-elev)" : "var(--bg)",
                fontWeight: windowChoice === w ? 600 : 400,
              }}
            >
              {w === "all" ? "All time" : `Last ${w}`}
            </button>
          ))}
        </div>
      </div>

      {error && (
        <div style={{ ...emptyStyle, color: "var(--danger)" }}>{error}</div>
      )}
      {loading && !error && <div style={emptyStyle}>Loading…</div>}
      {!loading && !error && cards.length === 0 && (
        <div style={emptyStyle}>
          No cards in this window. Adjust the time range or run{" "}
          <code>claudepot activity reindex</code>.
        </div>
      )}

      {!loading && !error && cards.length > 0 && (
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(auto-fit, minmax(360px, 1fr))",
            gap: "var(--sp-16)",
          }}
        >
          <Card title={`${agg.total.toLocaleString()} cards`} subtitle={`since ${windowLabel(windowChoice)}`}>
            <SeverityBar agg={agg} />
          </Card>
          <Card title="By day" subtitle="last 30 days">
            <Sparkbars data={daySeries(agg.byDay, 30)} />
          </Card>
          <Card title="Top templates">
            <Ranking data={topN(agg.byTemplate, 8)} totalForBar={agg.total} />
          </Card>
          <Card title="Top plugins">
            <Ranking data={topN(agg.byPlugin, 8)} totalForBar={agg.total} />
          </Card>
          <Card title="By kind">
            <Ranking data={topN(agg.byKind, 10)} totalForBar={agg.total} />
          </Card>
          <Card title="Sessions with most failures">
            <Ranking
              data={topN(agg.bySession, 8).map(([k, v]) => [basename(k), v])}
              totalForBar={agg.total}
            />
          </Card>
        </div>
      )}
    </div>
  );
}

// ── Aggregation ──────────────────────────────────────────────────

function aggregate(cards: ActivityCard[]): Aggregates {
  const bySeverity = new Map<SeverityLabel, number>();
  const byKind = new Map<CardKindLabel, number>();
  const byTemplate = new Map<string, number>();
  const byPlugin = new Map<string, number>();
  const bySession = new Map<string, number>();
  const byDay = new Map<string, number>();
  for (const c of cards) {
    bump(bySeverity, c.severity);
    bump(byKind, c.kind);
    if (c.help?.template_id) bump(byTemplate, c.help.template_id);
    if (c.plugin) bump(byPlugin, c.plugin);
    bump(bySession, c.session_path);
    bump(byDay, isoDay(c.ts_ms));
  }
  return {
    total: cards.length,
    bySeverity,
    byKind,
    byTemplate,
    byPlugin,
    bySession,
    byDay,
  };
}

function bump<K>(m: Map<K, number>, k: K) {
  m.set(k, (m.get(k) ?? 0) + 1);
}

function topN<K>(m: Map<K, number>, n: number): [K, number][] {
  return Array.from(m.entries())
    .sort((a, b) => b[1] - a[1])
    .slice(0, n);
}

function daySeries(byDay: Map<string, number>, days: number): number[] {
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

function basename(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

function windowLabel(w: "7d" | "30d" | "all"): string {
  if (w === "all") return "the beginning";
  if (w === "7d") return "7 days ago";
  return "30 days ago";
}

// ── Components ───────────────────────────────────────────────────

function Card({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle?: string;
  children: React.ReactNode;
}) {
  return (
    <section
      style={{
        background: "var(--bg-elev)",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-md)",
        padding: "var(--sp-16)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-12)",
      }}
    >
      <div>
        <h3
          style={{
            margin: 0,
            fontSize: "var(--fs-sm)",
            fontWeight: 600,
            color: "var(--fg)",
          }}
        >
          {title}
        </h3>
        {subtitle && (
          <div
            style={{
              fontSize: "var(--fs-xs)",
              color: "var(--muted)",
              marginTop: "var(--sp-2)",
            }}
          >
            {subtitle}
          </div>
        )}
      </div>
      {children}
    </section>
  );
}

function Sparkbars({ data }: { data: number[] }) {
  const max = Math.max(1, ...data);
  const w = 8;
  const gap = 2;
  const h = 56;
  return (
    <svg
      role="img"
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

function SeverityBar({ agg }: { agg: Aggregates }) {
  const order: { sev: SeverityLabel; color: string }[] = [
    { sev: "ERROR", color: "var(--danger, #dc2626)" },
    { sev: "WARN", color: "var(--warn, #d97706)" },
    { sev: "NOTICE", color: "var(--accent, #6b7280)" },
    { sev: "INFO", color: "var(--muted, #9ca3af)" },
  ];
  const total = agg.total || 1;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
      <div
        style={{
          display: "flex",
          width: "100%",
          height: 12,
          borderRadius: "var(--radius-sm)",
          overflow: "hidden",
          background: "var(--bg)",
          border: "1px solid var(--border)",
        }}
      >
        {order.map(({ sev, color }) => {
          const n = agg.bySeverity.get(sev) ?? 0;
          if (n === 0) return null;
          return (
            <div
              key={sev}
              style={{
                width: `${(n / total) * 100}%`,
                background: color,
              }}
              title={`${sev}: ${n}`}
            />
          );
        })}
      </div>
      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: "var(--sp-12)",
          fontSize: "var(--fs-xs)",
          color: "var(--muted)",
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
                width: 8,
                height: 8,
                borderRadius: 2,
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

function Ranking({
  data,
  totalForBar,
}: {
  data: [string, number][];
  totalForBar: number;
}) {
  if (data.length === 0)
    return (
      <div style={{ fontSize: "var(--fs-xs)", color: "var(--muted)" }}>
        No data.
      </div>
    );
  const max = Math.max(1, ...data.map(([, v]) => v));
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-6)" }}>
      {data.map(([k, v]) => (
        <div
          key={k}
          style={{ display: "flex", flexDirection: "column", gap: 2 }}
        >
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              fontSize: "var(--fs-xs)",
              color: "var(--fg)",
            }}
          >
            <span
              style={{
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                marginRight: "var(--sp-8)",
              }}
              title={k}
            >
              {k}
            </span>
            <span style={{ fontVariantNumeric: "tabular-nums", color: "var(--muted)" }}>
              {v.toLocaleString()}
            </span>
          </div>
          <div
            style={{
              height: 4,
              borderRadius: 2,
              background: "var(--bg)",
              overflow: "hidden",
            }}
          >
            <div
              style={{
                width: `${(v / max) * 100}%`,
                height: "100%",
                background: "var(--accent, #6366f1)",
                opacity: 0.7,
              }}
            />
          </div>
        </div>
      ))}
      {data.length > 0 && totalForBar > 0 && (
        <div
          style={{
            fontSize: 10,
            color: "var(--muted)",
            marginTop: "var(--sp-4)",
          }}
        >
          {data.reduce((s, [, v]) => s + v, 0).toLocaleString()} of{" "}
          {totalForBar.toLocaleString()}
        </div>
      )}
    </div>
  );
}

const btnStyle: React.CSSProperties = {
  fontSize: "var(--fs-sm)",
  padding: "var(--sp-4) var(--sp-12)",
  border: "1px solid var(--border)",
  borderRadius: "var(--radius-sm)",
  background: "var(--bg)",
  color: "var(--fg)",
  cursor: "pointer",
  fontFamily: "inherit",
};

const emptyStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  color: "var(--muted)",
  fontSize: "var(--fs-sm)",
  padding: "var(--sp-32)",
  textAlign: "center",
};
