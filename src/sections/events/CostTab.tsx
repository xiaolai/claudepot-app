// Activities → Cost.
//
// Local cost report: token totals + USD cost rolled up by project,
// derived from CC transcripts on disk via
// `aggregate_local_usage`. No network call; pricing comes from the
// bundled / cached price table. Mirrors the CLI surface
// `claudepot usage report` so a script and the GUI see the same
// numbers.
//
// Layout (top → bottom):
//   row 1 — controls: window selector + pricing-source pill
//   row 2 — four summary tiles: TOTAL COST, INPUT, OUTPUT, SESSIONS
//   row 3 — sortable table of per-project rows
//   row 4 — footer note when any session lacked a priced model
//
// Cost is install-wide. Per-account attribution is intentionally
// not surfaced (CC transcripts don't carry an account id, and
// claudepot keeps no swap-event log to reconstruct one); the
// pricing-source pill plus the unpriced-session note are the
// honesty signals on the figure.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { Table, Th, ThSort, Tr, Td } from "../../components/primitives";
import { formatRelative } from "../../lib/formatRelative";
import { i18n } from "../../lib/i18n";
import { renderError } from "../../lib/i18n-error";
import type {
  LocalUsageReport,
  PriceTierId,
  ProjectUsageRow,
  TopCostlyPrompts,
  UsageWindowSpec,
} from "../../types";
import { cacheHitRate, formatHitRate, shortModelId } from "./CostTabHelpers";
import { displayPath, formatCompact } from "./format";
import { TopPromptsPanel } from "./TopPromptsPanel";

// Re-exported for backward compatibility with tests that imported
// these helpers directly from CostTab. New callers should import
// from CostTabHelpers.
export { cacheHitRate, formatHitRate, shortModelId };

/** How many rows the top-prompts panel surfaces. Server caps at 50;
 *  5 keeps the panel a glance, not a deep dive. The user can drill
 *  into the per-session detail for more. */
const TOP_PROMPTS_LIMIT = 5;

/** Picker options for the pricing tier — keeps the label keys stable
 *  alongside the wire-form ids. Order mirrors the Rust
 *  `PriceTier::all()` so the default lands at the top. Labels resolve
 *  at render time so a language switch takes effect immediately. */
const TIER_OPTIONS = [
  { value: "anthropic_api", labelKey: "cost.tiers.anthropicApi" },
  { value: "vertex_global", labelKey: "cost.tiers.vertexGlobal" },
  { value: "vertex_regional", labelKey: "cost.tiers.vertexRegional" },
  { value: "aws_bedrock", labelKey: "cost.tiers.awsBedrock" },
] as const satisfies readonly { value: PriceTierId; labelKey: string }[];

function labelForTier(tier: PriceTierId): string {
  const opt = TIER_OPTIONS.find((o) => o.value === tier);
  return opt ? i18n.t(opt.labelKey, { ns: "activities" }) : tier;
}

type WindowChoice = "7d" | "30d" | "90d" | "all";

const WINDOW_OPTIONS = [
  { value: "7d", labelKey: "cost.window7d" },
  { value: "30d", labelKey: "cost.window30d" },
  { value: "90d", labelKey: "cost.window90d" },
  { value: "all", labelKey: "cost.windowAll" },
] as const satisfies readonly { value: WindowChoice; labelKey: string }[];

function toSpec(c: WindowChoice): UsageWindowSpec {
  if (c === "all") return { kind: "all" };
  const days = parseInt(c, 10);
  return { kind: "lastDays", days };
}

type SortKey =
  | "cost"
  | "sessions"
  | "last"
  | "input"
  | "output"
  | "cache_hit"
  | "project";
type SortDir = "asc" | "desc";

export function CostTab() {
  const [choice, setChoice] = useState<WindowChoice>("7d");
  const [report, setReport] = useState<LocalUsageReport | null>(null);
  const [topPrompts, setTopPrompts] = useState<TopCostlyPrompts | null>(null);
  // Active pricing tier hydrated independently of the report so the
  // Tier picker doesn't flicker through the default value on cold
  // start when a user has previously chosen Bedrock / Vertex. Falls
  // back to the report's `pricing_tier` echo (and ultimately to
  // `anthropic_api`) until the standalone fetch lands.
  const [activeTier, setActiveTier] = useState<PriceTierId | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sortKey, setSortKey] = useState<SortKey>("cost");
  const [sortDir, setSortDir] = useState<SortDir>("desc");
  const seqRef = useRef(0);

  const fetchReport = useCallback(
    async (c: WindowChoice) => {
      const seq = ++seqRef.current;
      setLoading(true);
      // Serialize the two backend calls so the second one (top-N) can
      // skip the redundant index refresh — the aggregate just did
      // it. Promise.all triggered two filesystem walks per tab tick,
      // wasting a stat-per-transcript pass each time.
      try {
        const r = await api.localUsageAggregate(toSpec(c));
        if (seq !== seqRef.current) return;
        const tp = await api.topCostlyPrompts(toSpec(c), TOP_PROMPTS_LIMIT, {
          refreshIndex: false,
        });
        if (seq !== seqRef.current) return;
        setReport(r);
        setTopPrompts(tp);
        setError(null);
      } catch (e) {
        if (seq !== seqRef.current) return;
        // Drop stale data on error — otherwise the UI shows old
        // summary tiles + old project rows alongside a fresh error
        // banner, which is worse than an empty pane that says
        // "couldn't load."
        setReport(null);
        setTopPrompts(null);
        setError(renderError(e));
      } finally {
        if (seq === seqRef.current) setLoading(false);
      }
    },
    [],
  );

  // Hydrate the active tier on mount via the dedicated getter. Done
  // once per component lifetime — subsequent setTier calls update
  // local state directly, and the report-echo path keeps the
  // value in sync if a different surface mutates the preference.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const t = await api.pricingTierGet();
        if (!cancelled) setActiveTier(t);
      } catch {
        // Failure is non-fatal — the report's `pricing_tier` echo
        // will populate the picker on the first successful fetch,
        // which lands within ~100ms of mount in the normal path.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    void fetchReport(choice);
  }, [choice, fetchReport]);

  const onSort = useCallback(
    (k: SortKey) => {
      if (k === sortKey) {
        setSortDir((d) => (d === "asc" ? "desc" : "asc"));
      } else {
        setSortKey(k);
        // Numeric columns default to descending — that's the more
        // useful first view (highest cost / busiest project first).
        // The project name column defaults to ascending alphabetic.
        setSortDir(k === "project" ? "asc" : "desc");
      }
    },
    [sortKey],
  );

  const sortedRows = useMemo(
    () => sortRows(report?.rows ?? [], sortKey, sortDir),
    [report?.rows, sortKey, sortDir],
  );

  const refreshPrices = useCallback(async () => {
    try {
      await api.pricingRefresh();
      await fetchReport(choice);
    } catch (e) {
      setError(renderError(e));
    }
  }, [choice, fetchReport]);

  const setTier = useCallback(
    async (t: PriceTierId) => {
      try {
        await api.pricingTierSet(t);
        setActiveTier(t);
        // Re-fetch so the table's dollar figures reflect the new
        // tier's rate multiplier; the report's pricing_tier echo
        // cross-checks our local state on the way back.
        await fetchReport(choice);
      } catch (e) {
        setError(renderError(e));
      }
    },
    [choice, fetchReport],
  );

  return (
    <div
      style={{
        padding: "var(--sp-12) var(--sp-16)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-12)",
        minWidth: 0,
        // The Activities section wraps each tab in a clipped flex
        // container; without these, a long project list gets cut off
        // below the fold instead of scrolling. Mirrors UsageView's
        // pattern (flex: 1; overflow: auto; minHeight: 0).
        flex: 1,
        overflow: "auto",
        minHeight: 0,
      }}
    >
      <Controls
        choice={choice}
        onChoice={setChoice}
        pricingSource={report?.pricing_source ?? null}
        pricingError={report?.pricing_error ?? null}
        // Prefer the explicitly-hydrated tier; fall back to the
        // report echo for users on a slow first-load path. Keeps
        // the picker stable through state transitions even when the
        // report is briefly null (e.g. during a re-fetch).
        pricingTier={activeTier ?? report?.pricing_tier ?? null}
        onTier={setTier}
        loading={loading}
      />
      <SummaryTiles report={report} loading={loading} />
      {error && (
        <div
          role="alert"
          style={{
            color: "var(--danger)",
            fontSize: "var(--fs-xs)",
          }}
        >
          {error}
        </div>
      )}
      {!error && report && (
        <CostTable
          rows={sortedRows}
          sortKey={sortKey}
          sortDir={sortDir}
          onSort={onSort}
        />
      )}
      {!error && topPrompts && topPrompts.turns.length > 0 && (
        <TopPromptsPanel data={topPrompts} />
      )}
      <UnpricedFooter report={report} onRefreshPrices={refreshPrices} />
    </div>
  );
}

// ───────────────────────────────────────────────────────────── controls

function Controls({
  choice,
  onChoice,
  pricingSource,
  pricingError,
  pricingTier,
  onTier,
  loading,
}: {
  choice: WindowChoice;
  onChoice: (c: WindowChoice) => void;
  pricingSource: string | null;
  pricingError: string | null;
  pricingTier: PriceTierId | null;
  onTier: (t: PriceTierId) => void;
  loading: boolean;
}) {
  const { t } = useTranslation("activities");
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-10)",
        flexWrap: "wrap",
      }}
    >
      <label
        htmlFor="cost-tab-window"
        style={{
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
        }}
      >
        {t("cost.window")}
      </label>
      <select
        id="cost-tab-window"
        value={choice}
        onChange={(e) => onChoice(e.target.value as WindowChoice)}
        style={{
          fontSize: "var(--fs-xs)",
          padding: "var(--sp-3) var(--sp-8)",
          background: "var(--bg-raised)",
          border: "var(--bw-hair) solid var(--line-strong)",
          borderRadius: "var(--r-1)",
          color: "var(--fg)",
          fontFamily: "inherit",
        }}
      >
        {WINDOW_OPTIONS.map((o) => (
          <option key={o.value} value={o.value}>
            {t(o.labelKey)}
          </option>
        ))}
      </select>
      <label
        htmlFor="cost-tab-tier"
        style={{
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
        }}
      >
        {t("cost.tier")}
      </label>
      <select
        id="cost-tab-tier"
        value={pricingTier ?? "anthropic_api"}
        onChange={(e) => onTier(e.target.value as PriceTierId)}
        title={t("cost.tierTitle")}
        style={{
          fontSize: "var(--fs-xs)",
          padding: "var(--sp-3) var(--sp-8)",
          background: "var(--bg-raised)",
          border: "var(--bw-hair) solid var(--line-strong)",
          borderRadius: "var(--r-1)",
          color: "var(--fg)",
          fontFamily: "inherit",
        }}
      >
        {TIER_OPTIONS.map((o) => (
          <option key={o.value} value={o.value}>
            {t(o.labelKey)}
          </option>
        ))}
      </select>
      {pricingSource && (
        <span
          title={
            pricingError ?? t("cost.priceSourceTitle", { source: pricingSource })
          }
          style={{
            fontSize: "var(--fs-2xs)",
            color: pricingError ? "var(--warn)" : "var(--fg-faint)",
            background: "var(--bg-sunken)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-1)",
            padding: "var(--sp-2) var(--sp-6)",
            letterSpacing: "var(--ls-wide)",
            textTransform: "uppercase",
          }}
        >
          {pricingTier ? `${labelForTier(pricingTier)} · ` : ""}
          {pricingSource}
          {pricingError ? ` · ${t("cost.stale")}` : ""}
        </span>
      )}
      {loading && (
        <span
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
          }}
        >
          {t("cost.loading")}
        </span>
      )}
    </div>
  );
}

// ─────────────────────────────────────────────────────────── summary tiles

function SummaryTiles({
  report,
  loading,
}: {
  report: LocalUsageReport | null;
  loading: boolean;
}) {
  const { t } = useTranslation("activities");
  const tot = report?.totals;
  const dash = "—";
  return (
    <div
      style={{
        display: "grid",
        // 4 equal-width tiles when there's room; collapse gracefully
        // on narrow Activities panes via auto-fit. The per-tile floor
        // uses the banner.min.width token (tokens.banner.min.width) — closest semantic
        // match in the catalog: "minimum width of a content cell that
        // should still read cleanly."
        gridTemplateColumns:
          "repeat(auto-fit, minmax(var(--banner-min-width), 1fr))",
        gap: "var(--sp-10)",
      }}
    >
      <Tile
        label={t("cost.tileTotalCost")}
        value={
          tot && tot.cost_usd != null
            ? `${tot.estimated_sessions > 0 ? "≈ " : ""}$${tot.cost_usd.toFixed(2)}`
            : loading
              ? dash
              : tot
                ? t("cost.na")
                : dash
        }
        // A family-estimated session's dollars are inside this total,
        // so the total is not a clean quote and must not read as one.
        sub={
          tot && tot.estimated_sessions > 0
            ? t("cost.installWideEstimated", { value: tot.estimated_sessions })
            : t("cost.installWide")
        }
      />
      <Tile
        label={t("cost.tileTokensIn")}
        value={renderTokens(tot?.tokens_input)}
        sub={
          tot
            ? t("cost.cacheHit", { rate: formatHitRate(cacheHitRate(tot)) })
            : renderTokens(undefined, t("cost.cacheRead"))
        }
      />
      <Tile
        label={t("cost.tileTokensOut")}
        value={renderTokens(tot?.tokens_output)}
        sub={renderTokens(tot?.tokens_cache_creation, t("cost.cacheWrite"))}
      />
      <Tile
        label={t("cost.tileSessions")}
        // Render `—` for zero so the empty-window state matches the
        // project-wide "render-if-nonzero" rule. A literal `0` reads
        // as a real value and competes with the "no sessions in this
        // window" notice in the table below.
        value={tot && tot.session_count > 0 ? String(tot.session_count) : dash}
        sub={
          tot && tot.unpriced_sessions > 0
            ? t("cost.unpricedCount", { value: tot.unpriced_sessions })
            : undefined
        }
      />
    </div>
  );
}

function renderTokens(n: number | undefined, suffix?: string): string {
  if (n == null || n === 0) return "—";
  const formatted = formatCompact(n);
  return suffix ? `${formatted} ${suffix}` : formatted;
}

function Tile({
  label,
  value,
  sub,
}: {
  label: string;
  value: string;
  sub?: string;
}) {
  return (
    <div
      style={{
        background: "var(--bg-raised)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        padding: "var(--sp-10) var(--sp-12)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-2)",
        minWidth: 0,
      }}
    >
      <div
        style={{
          fontSize: "var(--fs-2xs)",
          color: "var(--fg-faint)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
        }}
      >
        {label}
      </div>
      <div
        style={{
          fontSize: "var(--fs-lg)",
          fontVariantNumeric: "tabular-nums",
          color: "var(--fg)",
        }}
      >
        {value}
      </div>
      {sub && (
        <div
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-muted)",
          }}
        >
          {sub}
        </div>
      )}
    </div>
  );
}

// ─────────────────────────────────────────────────────────── cost table

function CostTable({
  rows,
  sortKey,
  sortDir,
  onSort,
}: {
  rows: ProjectUsageRow[];
  sortKey: SortKey;
  sortDir: SortDir;
  onSort: (k: SortKey) => void;
}) {
  const { t } = useTranslation("activities");
  if (rows.length === 0) {
    return (
      <div
        style={{
          fontSize: "var(--fs-xs)",
          color: "var(--fg-faint)",
          padding: "var(--sp-12)",
          textAlign: "center",
        }}
      >
        {t("cost.noSessions")}
      </div>
    );
  }
  return (
    <Table density="compact" style={{ fontSize: "var(--fs-xs)" }}>
      <thead>
        <tr style={{ background: "var(--bg-sunken)" }}>
          <ThSort
            value="project"
            current={sortKey}
            dir={sortDir}
            onSort={onSort}
            align="left"
          >
            {t("cost.colProject")}
          </ThSort>
          <ThSort
            value="sessions"
            current={sortKey}
            dir={sortDir}
            onSort={onSort}
            align="right"
          >
            {t("cost.colSessions")}
          </ThSort>
          <ThSort
            value="last"
            current={sortKey}
            dir={sortDir}
            onSort={onSort}
            align="right"
          >
            {t("cost.colLastActive")}
          </ThSort>
          <ThSort
            value="input"
            current={sortKey}
            dir={sortDir}
            onSort={onSort}
            align="right"
          >
            {t("cost.colInput")}
          </ThSort>
          <ThSort
            value="output"
            current={sortKey}
            dir={sortDir}
            onSort={onSort}
            align="right"
          >
            {t("cost.colOutput")}
          </ThSort>
          <ThSort
            value="cache_hit"
            current={sortKey}
            dir={sortDir}
            onSort={onSort}
            align="right"
          >
            {t("cost.colCacheHit")}
          </ThSort>
          <Th align="left">{t("cost.colModels")}</Th>
          <ThSort
            value="cost"
            current={sortKey}
            dir={sortDir}
            onSort={onSort}
            align="right"
          >
            {t("cost.colCost")}
          </ThSort>
          <Th align="center">⚠</Th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <Row key={r.project_path} row={r} />
        ))}
      </tbody>
    </Table>
  );
}

function Row({ row }: { row: ProjectUsageRow }) {
  const { t } = useTranslation("activities");
  // Mark a row whose total includes family-estimated sessions. The
  // dollars are real enough to sum, but the rate behind some of them
  // was inferred from the model's family, so `≈` keeps it from
  // reading as a quote.
  const estimated = row.estimated_sessions > 0;
  const cost =
    row.cost_usd == null
      ? t("cost.na")
      : `${estimated ? "≈ " : ""}$${row.cost_usd.toFixed(2)}`;
  const last =
    row.last_active_ms != null
      ? formatRelative(row.last_active_ms, { ago: false })
      : "—";
  const warn = row.unpriced_sessions > 0 ? row.unpriced_sessions : null;
  const hit = cacheHitRate(row);
  // Most projects share a long `/Users/<user>/...` prefix; the
  // basename + parent (one or two trailing segments) is the
  // discriminating part. Render that as the cell text, and put the
  // full path in `title` for a hover disclosure. Falls back to the
  // raw path when there are too few segments to abbreviate.
  const display = displayPath(row.project_path);
  return (
    <Tr>
      <Td
        title={row.project_path}
        style={{
          maxWidth: 0,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {display}
      </Td>
      <Td align="right">{row.session_count}</Td>
      <Td align="right">{last}</Td>
      <Td align="right">{formatCompact(row.tokens_input)}</Td>
      <Td align="right">{formatCompact(row.tokens_output)}</Td>
      <Td
        align="right"
        title={
          hit == null
            ? t("cost.noInputTokens")
            : t("cost.cacheHitTitle", {
                read: formatCompact(row.tokens_cache_read),
                total: formatCompact(
                  row.tokens_input +
                    row.tokens_cache_creation +
                    row.tokens_cache_read,
                ),
              })
        }
      >
        {formatHitRate(hit)}
      </Td>
      <Td align="left">
        <ModelBadges mix={row.models_by_session} />
      </Td>
      <Td align="right">{cost}</Td>
      <Td align="center">
        {warn != null ? (
          <span
            title={t("cost.unpricedWarnTitle", { value: warn })}
            style={{ color: "var(--warn)" }}
          >
            ⚠
          </span>
        ) : null}
      </Td>
    </Tr>
  );
}

/** Inline badge group for the row's model-mix column. Sorted by
 *  session-count descending so the most-used model shows first.
 *  Renders nothing for empty mixes — keeps the column quiet on
 *  user-only sessions. */
function ModelBadges({ mix }: { mix: Record<string, number> }) {
  const { t } = useTranslation("activities");
  const entries = Object.entries(mix).sort(
    (a, b) => b[1] - a[1] || a[0].localeCompare(b[0]),
  );
  if (entries.length === 0) {
    return (
      <span style={{ color: "var(--fg-faint)", fontSize: "var(--fs-2xs)" }}>
        —
      </span>
    );
  }
  return (
    <span
      style={{
        display: "inline-flex",
        gap: "var(--sp-4)",
        flexWrap: "wrap",
      }}
    >
      {entries.map(([model, count]) => (
        <span
          key={model}
          title={t("cost.modelBadgeTitle", { model, count })}
          style={{
            display: "inline-flex",
            alignItems: "baseline",
            gap: "var(--sp-3)",
            background: "var(--bg-sunken)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-1)",
            padding: "var(--sp-1) var(--sp-4)",
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-muted)",
            fontVariantNumeric: "tabular-nums",
            whiteSpace: "nowrap",
          }}
        >
          <span style={{ color: "var(--fg)" }}>{shortModelId(model)}</span>
          <span style={{ color: "var(--fg-faint)" }}>·{count}</span>
        </span>
      ))}
    </span>
  );
}

// ─────────────────────────────────────────────────────────── footer

function UnpricedFooter({
  report,
  onRefreshPrices,
}: {
  report: LocalUsageReport | null;
  onRefreshPrices: () => void;
}) {
  const { t } = useTranslation("activities");
  if (!report || report.totals.unpriced_sessions === 0) return null;
  const u = report.totals.unpriced_sessions;
  const total = report.totals.session_count;
  return (
    <div
      role="note"
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-10)",
        background: "color-mix(in oklch, var(--warn) 8%, transparent)",
        border: "var(--bw-hair) solid var(--line)",
        borderLeft: "var(--bw-accent) solid var(--warn)",
        borderRadius: "var(--r-2)",
        padding: "var(--sp-8) var(--sp-12)",
        fontSize: "var(--fs-xs)",
        color: "var(--fg-muted)",
      }}
    >
      <span>
        {t("cost.unpricedFooter", { unpriced: u, count: total })}
      </span>
      <button
        type="button"
        onClick={onRefreshPrices}
        style={{
          padding: "var(--sp-3) var(--sp-8)",
          fontSize: "var(--fs-2xs)",
          background: "var(--bg-raised)",
          border: "var(--bw-hair) solid var(--line-strong)",
          borderRadius: "var(--r-1)",
          color: "var(--fg)",
          cursor: "pointer",
          whiteSpace: "nowrap",
          fontFamily: "inherit",
        }}
      >
        {t("cost.refreshPrices")}
      </button>
    </div>
  );
}

// ──────────────────────────────────────────────────────────── sort

function sortRows(
  rows: ProjectUsageRow[],
  key: SortKey,
  dir: SortDir,
): ProjectUsageRow[] {
  // Partition nulls out FIRST so they always land at the end
  // regardless of `dir`. Reversing the post-sort array flipped the
  // null position with the rest of the data, which made the
  // ascending view of cost / cache-hit / last-active put unpriced
  // rows above every real row. The partition keeps "nulls last" as
  // an invariant the caller can rely on.
  const nullKey = (r: ProjectUsageRow): boolean => {
    switch (key) {
      case "cost":
        return r.cost_usd == null;
      case "last":
        return r.last_active_ms == null;
      case "cache_hit":
        return cacheHitRate(r) == null;
      default:
        return false;
    }
  };
  const realCmp = (a: ProjectUsageRow, b: ProjectUsageRow): number => {
    switch (key) {
      case "cost":
        // Both are non-null in this branch — coerce safely.
        return (a.cost_usd ?? 0) - (b.cost_usd ?? 0);
      case "sessions":
        return a.session_count - b.session_count;
      case "last":
        return (a.last_active_ms ?? 0) - (b.last_active_ms ?? 0);
      case "input":
        return a.tokens_input - b.tokens_input;
      case "output":
        return a.tokens_output - b.tokens_output;
      case "cache_hit":
        return (cacheHitRate(a) ?? 0) - (cacheHitRate(b) ?? 0);
      case "project":
        return a.project_path.localeCompare(b.project_path);
    }
  };
  const real: ProjectUsageRow[] = [];
  const nullsAtEnd: ProjectUsageRow[] = [];
  for (const r of rows) {
    if (nullKey(r)) nullsAtEnd.push(r);
    else real.push(r);
  }
  real.sort(realCmp);
  if (dir === "desc") real.reverse();
  return [...real, ...nullsAtEnd];
}

// `displayPath` and `formatCompact` moved to `./format.ts` — shared
// with TopPromptsPanel (audit 2026-07 F11).

// `nullableNumberCmp` was removed when sortRows started partitioning
// nulls explicitly. The previous impl returned -1 for `a == null` and
// then relied on the caller reversing the array for descending order,
// which silently inverted the "nulls last" invariant in ascending
// mode and floated unpriced rows above every priced one. The new
// partition is direction-independent and easier to test.

// `TopPromptsPanel` was extracted to `./TopPromptsPanel.tsx` to keep
// this file under the loc-guardian limit. Imported at the top.
