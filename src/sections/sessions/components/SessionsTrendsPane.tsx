import { useEffect, useState } from "react";
import { api } from "../../../api";
import type { ActivityTrends } from "../../../types";

/**
 * Trends pane inside Sessions. Moved from the (now-removed)
 * Activity section as part of the C-1 A consolidation — live session
 * aggregates and historical trends are both facets of the same
 * sessions domain.
 *
 * Queries the backend activity_trends store for bucketed
 * active-session counts + error totals over a 24h / 7d / 30d window.
 */
type Window = "24h" | "7d" | "30d";

const WINDOW_MS: Record<Window, number> = {
  "24h": 24 * 60 * 60 * 1000,
  "7d": 7 * 24 * 60 * 60 * 1000,
  "30d": 30 * 24 * 60 * 60 * 1000,
};

const WINDOW_BUCKETS: Record<Window, number> = {
  "24h": 24,
  "7d": 28,
  "30d": 30,
};

export function SessionsTrendsPane() {
  const [window, setWindow] = useState<Window>("24h");
  const [trends, setTrends] = useState<ActivityTrends | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    const now = Date.now();
    const from = now - WINDOW_MS[window];
    api
      .activityTrends(from, now, WINDOW_BUCKETS[window])
      .then((t) => {
        if (!cancelled) {
          setTrends(t);
          setLoading(false);
        }
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
  }, [window]);

  return (
    <section style={{ padding: "var(--sp-24)" }}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          marginBottom: "var(--sp-10)",
        }}
      >
        <h2
          style={{
            fontSize: "var(--fs-xs)",
            fontWeight: 600,
            color: "var(--fg-muted)",
            textTransform: "uppercase",
            letterSpacing: "var(--ls-wide)",
            margin: 0,
          }}
        >
          Trends
        </h2>
        <div
          role="tablist"
          style={{
            display: "inline-flex",
            gap: "var(--sp-2)",
            padding: "var(--sp-2)",
            background: "var(--bg-sunken)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
          }}
        >
          {(["24h", "7d", "30d"] as Window[]).map((w) => {
            const current = window === w;
            return (
              <button
                key={w}
                type="button"
                role="tab"
                aria-selected={current}
                onClick={() => setWindow(w)}
                style={{
                  padding: "var(--sp-2) var(--sp-8)",
                  fontSize: "var(--fs-2xs)",
                  color: current ? "var(--fg)" : "var(--fg-muted)",
                  background: current ? "var(--bg-raised)" : "transparent",
                  border: "none",
                  borderRadius: "var(--r-1)",
                  cursor: "pointer",
                  fontVariantNumeric: "tabular-nums",
                }}
              >
                {w}
              </button>
            );
          })}
        </div>
      </div>

      {loading ? (
        <div style={{ color: "var(--fg-faint)", fontSize: "var(--fs-sm)" }}>
          Loading…
        </div>
      ) : error ? (
        <div style={{ color: "var(--fg-muted)", fontSize: "var(--fs-sm)" }}>
          Couldn't load metrics: {error}
        </div>
      ) : trends ? (
        <TrendsCards trends={trends} />
      ) : null}
    </section>
  );
}

function TrendsCards({ trends }: { trends: ActivityTrends }) {
  const peak = Math.max(1, ...trends.active_series);
  const totalActive = trends.active_series.reduce((a, b) => a + b, 0);
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-12)",
      }}
    >
      <div
        style={{
          padding: "var(--sp-14) var(--sp-16)",
          border: "var(--bw-hair) solid var(--line)",
          borderRadius: "var(--r-2)",
          background: "var(--bg)",
        }}
      >
        <div
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            textTransform: "uppercase",
            letterSpacing: "var(--ls-wide)",
            marginBottom: "var(--sp-6)",
          }}
        >
          Sessions active per bucket
        </div>
        <Sparkline values={trends.active_series} peak={peak} />
        <div
          style={{
            marginTop: "var(--sp-6)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-muted)",
            fontVariantNumeric: "tabular-nums",
          }}
        >
          peak {peak} · total observations {totalActive}
        </div>
      </div>
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fit, minmax(180px, 1fr))",
          gap: "var(--sp-10)",
        }}
      >
        {trends.error_count > 0 && (
          <StatCard
            label="Error ticks"
            value={String(trends.error_count)}
            sub="ticks where the errored overlay was on"
          />
        )}
        <StatCard
          label="Window"
          value={formatRange(trends.from_ms, trends.to_ms)}
          sub={`${trends.active_series.length} buckets`}
        />
      </div>
    </div>
  );
}

function StatCard({
  label,
  value,
  sub,
}: {
  label: string;
  value: string;
  sub: string;
}) {
  return (
    <div
      style={{
        padding: "var(--sp-10) var(--sp-12)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        background: "var(--bg)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-2)",
      }}
    >
      <span
        style={{
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
          textTransform: "uppercase",
          letterSpacing: "var(--ls-wide)",
        }}
      >
        {label}
      </span>
      <span
        style={{
          fontSize: "var(--fs-lg)",
          fontWeight: 500,
          fontVariantNumeric: "tabular-nums",
        }}
      >
        {value}
      </span>
      <span style={{ fontSize: "var(--fs-xs)", color: "var(--fg-muted)" }}>
        {sub}
      </span>
    </div>
  );
}

function Sparkline({ values, peak }: { values: number[]; peak: number }) {
  const width = 400;
  const height = 40;
  const step = values.length > 0 ? width / values.length : 0;
  return (
    <svg
      width="100%"
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="none"
      role="img"
      aria-label={`${values.length}-bucket sparkline, peak ${peak}`}
      style={{ display: "block", height: "var(--sp-40)" }}
    >
      {values.map((v, i) => {
        const h = peak > 0 ? (v / peak) * (height - 4) : 0;
        return (
          <rect
            key={i}
            x={i * step + 1}
            y={height - h - 2}
            width={Math.max(1, step - 2)}
            height={Math.max(0, h)}
            fill={v > 0 ? "var(--accent)" : "var(--line)"}
          />
        );
      })}
    </svg>
  );
}

function formatRange(fromMs: number, toMs: number): string {
  const span = toMs - fromMs;
  const hours = Math.round(span / (60 * 60 * 1000));
  if (hours < 48) return `${hours}h`;
  return `${Math.round(hours / 24)}d`;
}
