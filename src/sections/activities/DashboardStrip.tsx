import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { i18n } from "../../lib/i18n";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
import { sessionEventMs } from "../../lib/sessionTime";
import type { PriceTableDto, SessionRow } from "../../types";
import {
  estimatedRateHint,
  formatUsd,
  sessionCostEstimate,
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
  const { t } = useTranslation("activities");
  // Background CC supervisor + detached worker count. Surfaces in
  // the "Live" card when > 0 so the user sees the full picture of
  // active CC processes, not just the foreground ones their terminals
  // are attached to. See `dev-docs/cc-daemon-research.md`.
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
      setRefreshError(renderError(e));
    } finally {
      setRefreshing(false);
    }
  };

  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
        gap: "var(--sp-12)",
        padding: "var(--sp-16) var(--sp-24)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-sunken)",
      }}
    >

      <StatCard label={t("dashboard.today")}>
        {allSessions === null ? (
          <Subline>{t("dashboard.loading")}</Subline>
        ) : rollups.today.sessions === 0 ? (
          <IdleValue>{t("dashboard.noActivity")}</IdleValue>
        ) : (
          <>
            <BigValue
              value={rollups.today.sessions}
              suffix={t("dashboard.sessionsSuffix", {
                count: rollups.today.sessions,
              })}
            />
            <Subline>
              {rollups.today.tokens > 0 && (
                <>
                  {t("dashboard.tokens", {
                    value: formatTokensHuman(rollups.today.tokens),
                  })}
                </>
              )}
              {rollups.today.costUsd != null && (
                <>
                  {rollups.today.tokens > 0 ? " · " : ""}
                  <span title={rollups.today.costEstimated ? estimatedRateHint() : undefined}>
                    {rollups.today.costEstimated ? "≈ " : ""}
                    {formatUsd(rollups.today.costUsd)}
                  </span>{" "}
                  {t("dashboard.onApi")}
                </>
              )}
            </Subline>
          </>
        )}
      </StatCard>

      <StatCard
        label={t("dashboard.thisMonth")}
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
          <Subline>{t("dashboard.loading")}</Subline>
        ) : rollups.month.sessions === 0 ? (
          <IdleValue>{t("dashboard.noActivity")}</IdleValue>
        ) : (
          <>
            <BigValue
              value={rollups.month.sessions}
              suffix={t("dashboard.sessionsSuffix", {
                count: rollups.month.sessions,
              })}
            />
            <Subline>
              {rollups.month.tokens > 0 && (
                <>
                  {t("dashboard.tokens", {
                    value: formatTokensHuman(rollups.month.tokens),
                  })}
                </>
              )}
              {rollups.month.costUsd != null && (
                <>
                  {rollups.month.tokens > 0 ? " · " : ""}
                  <span title={rollups.month.costEstimated ? estimatedRateHint() : undefined}>
                    {rollups.month.costEstimated ? "≈ " : ""}
                    {formatUsd(rollups.month.costUsd)}
                  </span>{" "}
                  {t("dashboard.onApi")}
                </>
              )}
            </Subline>
          </>
        )}
      </StatCard>
    </div>
  );
}

// ---------- pure stat derivation ----------


interface Rollup {
  sessions: number;
  tokens: number;
  costUsd: number | null;
  /** At least one contributing session was priced from a family
   *  estimate, so `costUsd` is not a clean quote. */
  costEstimated: boolean;
}

interface DayMonthRollups {
  today: Rollup;
  month: Rollup;
}

/**
 * Client-side rollup. Each session is priced at the rate in force on
 * its own day, so a month spanning a price change totals what those
 * sessions actually cost rather than re-scoring them all at today's
 * rate.
 *
 * Sessions from a model family we don't price contribute tokens but
 * not cost. Sessions whose exact model isn't listed are priced from
 * their family's rate and flip `costEstimated`, which the card renders
 * as a leading `≈` — an unmarked guess would read as a quote.
 */
function deriveDayMonthRollups(
  rows: SessionRow[],
  table: PriceTableDto | null,
): DayMonthRollups {
  const now = new Date();
  const startOfDay = startOfLocalDayMs(now);
  const startOfMonth = startOfLocalMonthMs(now);

  const today: Rollup = {
    sessions: 0,
    tokens: 0,
    costUsd: null,
    costEstimated: false,
  };
  const month: Rollup = {
    sessions: 0,
    tokens: 0,
    costUsd: null,
    costEstimated: false,
  };
  let todayCostSum = 0;
  let monthCostSum = 0;
  let todayHadKnownCost = false;
  let monthHadKnownCost = false;

  for (const row of rows) {
    // The transcript's own event time, not the file's mtime. A move,
    // re-index, or `slim` rewrite changes mtime without any work
    // happening — bucketing by it would move a session between months
    // and, now that rates are dated, re-price it at a rate that wasn't
    // in force when the tokens were spent. `usage_local` buckets on
    // `last_ts` for the same reason; this keeps the two agreeing.
    const eventMs = sessionEventMs(row.last_ts, row.last_modified_ms);
    if (eventMs == null) continue;
    if (eventMs < startOfMonth) continue;

    const inMonth = eventMs >= startOfMonth;
    const inToday = eventMs >= startOfDay;
    const total = row.tokens.total ?? 0;

    if (inToday) {
      today.sessions += 1;
      today.tokens += total;
    }
    if (inMonth) {
      month.sessions += 1;
      month.tokens += total;
    }
    // One cost path for the whole app: `sessionCostEstimate` owns the
    // dominant-model choice and the token shaping, and prices at the
    // rate in force on the session's own day. Re-deriving either here
    // is how this rollup and the transcript header drifted apart.
    const c = sessionCostEstimate(table, row.models, row.tokens, eventMs);
    if (c == null) continue;
    if (inToday) {
      todayCostSum += c.usd;
      todayHadKnownCost = true;
      if (c.confidence === "family_estimate") today.costEstimated = true;
    }
    if (inMonth) {
      monthCostSum += c.usd;
      monthHadKnownCost = true;
      if (c.confidence === "family_estimate") month.costEstimated = true;
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
  // Resolved per render, not at module load, so the chip follows a
  // language switch. `kind` itself is a wire value and stays raw when
  // it is one this map doesn't know.
  const labelByKind: Record<string, string> = {
    bundled: i18n.t("dashboard.rate.builtIn", { ns: "activities" }),
    cached: i18n.t("dashboard.rate.cached", { ns: "activities" }),
    live: i18n.t("dashboard.rate.fresh", { ns: "activities" }),
  };
  const tone =
    kind === "live"
      ? "var(--accent)"
      : kind === "cached"
      ? "var(--fg-muted)"
      : "var(--fg-faint)";
  const ns = { ns: "activities" } as const;
  const titleParts: string[] = [
    i18n.t("dashboard.rate.source", { ...ns, kind }),
    i18n.t("dashboard.rate.asOf", { ...ns, timestamp }),
  ];
  if (table.source.url) {
    titleParts.push(
      i18n.t("dashboard.rate.from", { ...ns, url: table.source.url }),
    );
  }
  if (table.last_fetch_error) {
    titleParts.push(
      i18n.t("dashboard.rate.lastError", {
        ...ns,
        error: table.last_fetch_error,
      }),
    );
  }
  if (error) {
    titleParts.push(i18n.t("dashboard.rate.clickError", { ...ns, error }));
  }
  titleParts.push(i18n.t("dashboard.rate.clickToRefresh", ns));
  const effectiveTone = error ? "var(--danger)" : tone;
  const labelText = loading
    ? i18n.t("dashboard.rate.refreshing", ns)
    : error
    ? i18n.t("dashboard.rate.refreshFailed", ns)
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
        padding: "var(--sp-2) var(--sp-6)",
        fontSize: "var(--fs-2xs)",
        fontWeight: 500,
        letterSpacing: "0.04em",
        color: effectiveTone,
        background: "transparent",
        border: `var(--bw-hair) solid ${error ? "var(--danger)" : "var(--line)"}`,
        borderRadius: "var(--r-1)",
        cursor: loading ? "progress" : undefined,
        opacity: loading ? 0.6 : 1,
      }}
    >
      {labelText}
    </button>
  );
}
