import { useEffect, useMemo, useState } from "react";
import { api } from "../../api";
import { useSessionLive } from "../../hooks/useSessionLive";
import type { PriceTableDto, SessionRow } from "../../types";
import {
  costFromUsage,
  formatUsd,
  usePriceTable,
} from "../../costs";

/**
 * Dashboard strip for the Activities section — the at-a-glance
 * "what's happening right now / today / this month" summary that
 * sits above the existing cross-project session list.
 *
 * Three stat cards + a rate-source chip:
 *   - **Live**      — running sessions, model mix
 *   - **Today**     — sessions started · tokens · API-equivalent $
 *   - **This month**— same, monthly scope
 *
 * All figures are derived on the client from two feeds:
 *   - `useSessionLive()`   — live snapshot of running sessions
 *   - `api.sessionListAll()` — full list of sessions with per-row
 *     token counts and last-modified timestamps
 *
 * Cost numbers intentionally match the transcript header's framing —
 * "on API" means "what pay-per-call would have cost you." Subscription
 * users read this as the amount they DIDN'T pay.
 */
export function DashboardStrip() {
  const live = useSessionLive();
  const { table: priceTable, loading: priceLoading } = usePriceTable();
  const [allSessions, setAllSessions] = useState<SessionRow[] | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  // Surface transport-level refresh failures. The backend's
  // `pricing_refresh` command returns the last-good table with
  // `last_fetch_error` set on scrape failure, so the chip's tooltip
  // handles that case. This state is for the outer layer — Tauri
  // IPC unavailable, app crash, etc. — where no response came back
  // at all and the user needs visible confirmation their click
  // didn't land.
  const [refreshError, setRefreshError] = useState<string | null>(null);
  // Local table state backs the rate-source chip's click-to-refresh.
  // Seeded from the hook's initial load, then only written by
  // explicit refreshes — we do NOT overwrite on subsequent hook
  // emissions, otherwise a slow initial fetch that resolves after
  // a user-triggered refresh would clobber the fresh numbers with
  // the older hook copy (race found in audit).
  const [table, setTable] = useState<PriceTableDto | null>(null);

  useEffect(() => {
    setTable((prev) => prev ?? priceTable);
  }, [priceTable]);

  useEffect(() => {
    let cancelled = false;
    void api
      .sessionListAll()
      .then((rows) => {
        if (!cancelled) setAllSessions(rows);
      })
      .catch(() => {
        if (!cancelled) setAllSessions([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const liveStats = useMemo(() => deriveLiveStats(live), [live]);
  const rollups = useMemo(
    () => deriveDayMonthRollups(allSessions ?? [], table),
    [allSessions, table],
  );

  const onRefreshRates = async () => {
    setRefreshing(true);
    setRefreshError(null);
    try {
      const fresh = await api.pricingRefresh();
      setTable(fresh);
    } catch (e) {
      // Transport-level failure: the IPC call never returned a
      // table. Surface a short message so the user sees their
      // click landed. Scrape-level failures are already expressed
      // via the returned table's `last_fetch_error` field and the
      // chip tooltip — those don't reach this catch.
      setRefreshError(String(e));
    } finally {
      setRefreshing(false);
    }
  };

  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
        gap: "var(--sp-12)",
        padding: "var(--sp-16) var(--sp-24)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-sunken)",
      }}
    >
      <StatCard label="Live">
        {liveStats.running > 0 ? (
          <>
            <BigValue value={liveStats.running} suffix="running" />
            {liveStats.models.length > 0 && (
              <Subline>{liveStats.models.join(" · ")}</Subline>
            )}
          </>
        ) : (
          <IdleValue>idle</IdleValue>
        )}
      </StatCard>

      <StatCard label="Today">
        {allSessions === null ? (
          <Subline>loading…</Subline>
        ) : rollups.today.sessions === 0 ? (
          <IdleValue>no activity yet</IdleValue>
        ) : (
          <>
            <BigValue
              value={rollups.today.sessions}
              suffix={`session${rollups.today.sessions === 1 ? "" : "s"}`}
            />
            <Subline>
              {formatTokensHuman(rollups.today.tokens)} tokens
              {rollups.today.costUsd != null && (
                <> · {formatUsd(rollups.today.costUsd)} on API</>
              )}
            </Subline>
          </>
        )}
      </StatCard>

      <StatCard
        label="This month"
        right={
          <RateSourceChip
            table={table}
            loading={priceLoading || refreshing}
            error={refreshError}
            onRefresh={() => void onRefreshRates()}
          />
        }
      >
        {allSessions === null ? (
          <Subline>loading…</Subline>
        ) : rollups.month.sessions === 0 ? (
          <IdleValue>no activity yet</IdleValue>
        ) : (
          <>
            <BigValue
              value={rollups.month.sessions}
              suffix={`session${rollups.month.sessions === 1 ? "" : "s"}`}
            />
            <Subline>
              {formatTokensHuman(rollups.month.tokens)} tokens
              {rollups.month.costUsd != null && (
                <> · {formatUsd(rollups.month.costUsd)} on API</>
              )}
            </Subline>
          </>
        )}
      </StatCard>
    </div>
  );
}

// ---------- pure stat derivation ----------

interface LiveStats {
  running: number;
  /** Model mix chips like `Opus 4.7 · 2`. Only models with ≥1 session. */
  models: string[];
}

function deriveLiveStats(
  live: ReturnType<typeof useSessionLive>,
): LiveStats {
  // Count sessions that are actively working OR paused on a user
  // prompt. An idle session that just wrapped up isn't "live" for
  // the dashboard's purposes.
  const active = live.filter(
    (s) => s.status === "busy" || s.status === "waiting",
  );
  const byModel = new Map<string, number>();
  for (const s of active) {
    const m = compactModelLabel(s.model);
    if (!m) continue;
    byModel.set(m, (byModel.get(m) ?? 0) + 1);
  }
  return {
    running: active.length,
    models: Array.from(byModel.entries()).map(
      ([m, n]) => `${m}${n > 1 ? ` × ${n}` : ""}`,
    ),
  };
}

interface Rollup {
  sessions: number;
  tokens: number;
  costUsd: number | null;
}

interface DayMonthRollups {
  today: Rollup;
  month: Rollup;
}

/**
 * Client-side rollup. Sessions with unknown model ids contribute
 * tokens to both rollups but don't contribute to cost — we intentionally
 * leave `costUsd` as-is (unknown models would produce a lowball
 * estimate if we substituted a rate).
 */
function deriveDayMonthRollups(
  rows: SessionRow[],
  table: PriceTableDto | null,
): DayMonthRollups {
  const now = new Date();
  const startOfDay = startOfLocalDayMs(now);
  const startOfMonth = startOfLocalMonthMs(now);

  const today: Rollup = { sessions: 0, tokens: 0, costUsd: null };
  const month: Rollup = { sessions: 0, tokens: 0, costUsd: null };
  let todayCostSum = 0;
  let monthCostSum = 0;
  let todayHadKnownCost = false;
  let monthHadKnownCost = false;

  for (const row of rows) {
    const ts = row.last_modified_ms;
    if (ts == null) continue;
    if (ts < startOfMonth) continue;

    const inMonth = ts >= startOfMonth;
    const inToday = ts >= startOfDay;
    const total = row.tokens.total ?? 0;

    if (inToday) {
      today.sessions += 1;
      today.tokens += total;
    }
    if (inMonth) {
      month.sessions += 1;
      month.tokens += total;
    }
    // Cost — pick first model. See `sessionCostEstimate` for the same
    // approximation used in the transcript header.
    const primaryModel = row.models[0];
    if (!primaryModel) continue;
    const c = costFromUsage(table, primaryModel, {
      input: row.tokens.input,
      output: row.tokens.output,
      cache_read: row.tokens.cache_read,
      cache_creation: row.tokens.cache_creation,
    });
    if (c == null) continue;
    if (inToday) {
      todayCostSum += c;
      todayHadKnownCost = true;
    }
    if (inMonth) {
      monthCostSum += c;
      monthHadKnownCost = true;
    }
  }
  today.costUsd = todayHadKnownCost ? todayCostSum : null;
  month.costUsd = monthHadKnownCost ? monthCostSum : null;
  return { today, month };
}

function startOfLocalDayMs(d: Date): number {
  const x = new Date(d.getFullYear(), d.getMonth(), d.getDate(), 0, 0, 0, 0);
  return x.getTime();
}

function startOfLocalMonthMs(d: Date): number {
  const x = new Date(d.getFullYear(), d.getMonth(), 1, 0, 0, 0, 0);
  return x.getTime();
}

/** Compact model label for the Live card. `claude-opus-4-7` →
 *  `Opus 4.7`. Keeps the chip short enough to fit several side-by-side. */
function compactModelLabel(raw: string | null): string | null {
  if (!raw) return null;
  const stripped = raw
    .replace(/^claude-/, "")
    .replace(/-\d{8,}$/, "")
    .replace(/-(preview|latest|experimental)$/, "");
  const parts = stripped.split("-");
  if (parts.length === 0) return null;
  const family = parts[0].charAt(0).toUpperCase() + parts[0].slice(1);
  const version = parts.slice(1).join(".");
  return version ? `${family} ${version}` : family;
}

function formatTokensHuman(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

// ---------- presentational atoms ----------

function StatCard({
  label,
  right,
  children,
}: {
  label: string;
  right?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-4)",
        padding: "var(--sp-10) var(--sp-14)",
        background: "var(--bg)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        minWidth: 0,
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: "var(--sp-6)",
        }}
      >
        <span
          className="mono-cap"
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            letterSpacing: "0.08em",
            textTransform: "uppercase",
          }}
        >
          {label}
        </span>
        {right}
      </div>
      {children}
    </section>
  );
}

function BigValue({
  value,
  suffix,
}: {
  value: number;
  suffix: string;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "baseline",
        gap: "var(--sp-6)",
      }}
    >
      <span
        style={{
          fontSize: "var(--fs-xl)",
          fontWeight: 600,
          color: "var(--fg)",
          fontVariantNumeric: "tabular-nums",
        }}
      >
        {value}
      </span>
      <span
        style={{
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
        }}
      >
        {suffix}
      </span>
    </div>
  );
}

function Subline({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        fontSize: "var(--fs-xs)",
        color: "var(--fg-muted)",
        overflow: "hidden",
        textOverflow: "ellipsis",
        whiteSpace: "nowrap",
      }}
    >
      {children}
    </div>
  );
}

function IdleValue({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        fontSize: "var(--fs-sm)",
        color: "var(--fg-faint)",
        fontStyle: "italic",
      }}
    >
      {children}
    </div>
  );
}

/**
 * Tiny chip that shows the provenance of the rate table and lets the
 * user force a refresh. Placed in the "This month" card because cost
 * is the figure users care about validating — clicking here signals
 * "show me the freshest number right now."
 */
function RateSourceChip({
  table,
  loading,
  error,
  onRefresh,
}: {
  table: PriceTableDto | null;
  loading: boolean;
  /** Transport-level error from the last refresh click, if any. */
  error?: string | null;
  onRefresh: () => void;
}) {
  if (!table) return null;
  const { kind, timestamp } = table.source;
  const shortTs = (() => {
    // timestamps come through as either "YYYY-MM-DD HH:MM:SSZ" or
    // just a verification date; we only need the date part for
    // at-a-glance display. Tooltip carries the full string.
    const m = timestamp.match(/^(\d{4}-\d{2}-\d{2})/);
    return m ? m[1] : timestamp;
  })();
  const labelByKind: Record<string, string> = {
    bundled: "built-in",
    cached: "cached",
    live: "fresh",
  };
  const tone =
    kind === "live"
      ? "var(--accent)"
      : kind === "cached"
      ? "var(--fg-muted)"
      : "var(--fg-faint)";
  const titleParts: string[] = [
    `Rate source: ${kind}`,
    `As of: ${timestamp}`,
  ];
  if (table.source.url) titleParts.push(`From: ${table.source.url}`);
  if (table.last_fetch_error) {
    titleParts.push(`Last refresh error: ${table.last_fetch_error}`);
  }
  if (error) titleParts.push(`Click error: ${error}`);
  titleParts.push("Click to refresh now.");
  const effectiveTone = error ? "var(--danger)" : tone;
  const labelText = loading
    ? "refreshing…"
    : error
    ? "refresh failed"
    : `${labelByKind[kind] ?? kind} · ${shortTs}`;
  return (
    <button
      type="button"
      onClick={onRefresh}
      disabled={loading}
      className="pm-focus"
      title={titleParts.join("\n")}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-4)",
        padding: "2px var(--sp-6)",
        fontSize: "var(--fs-2xs)",
        fontWeight: 500,
        letterSpacing: "0.04em",
        color: effectiveTone,
        background: "transparent",
        border: `var(--bw-hair) solid ${error ? "var(--danger)" : "var(--line)"}`,
        borderRadius: "var(--r-sm)",
        cursor: loading ? "progress" : "pointer",
        opacity: loading ? 0.6 : 1,
      }}
    >
      {labelText}
    </button>
  );
}
