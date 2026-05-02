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
import { api } from "../../api";
import { formatRelative } from "../../lib/formatRelative";
import type {
  LocalUsageReport,
  PriceTierId,
  ProjectUsageRow,
  UsageWindowSpec,
} from "../../types";

/** Picker options for the pricing tier — keeps the labels stable
 *  alongside the wire-form ids. Order mirrors the Rust
 *  `PriceTier::all()` so the default lands at the top. */
const TIER_OPTIONS: { value: PriceTierId; label: string }[] = [
  { value: "anthropic_api", label: "Anthropic API" },
  { value: "vertex_global", label: "Vertex Global" },
  { value: "vertex_regional", label: "Vertex Regional" },
  { value: "aws_bedrock", label: "AWS Bedrock" },
];

function labelForTier(t: PriceTierId): string {
  return TIER_OPTIONS.find((o) => o.value === t)?.label ?? t;
}

type WindowChoice = "7d" | "30d" | "90d" | "all";

const WINDOW_OPTIONS: { value: WindowChoice; label: string }[] = [
  { value: "7d", label: "Last 7 days" },
  { value: "30d", label: "Last 30 days" },
  { value: "90d", label: "Last 90 days" },
  { value: "all", label: "All time" },
];

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

/**
 * Prompt-cache hit rate for a row of input-side token totals.
 *
 * Definition matches Anthropic's billing model: every prompt token is
 * categorised as fresh `input`, `cache_creation` (writing the prefix
 * for future hits), or `cache_read` (served from cache). Hit rate is
 * the read share of that pie.
 *
 * Returns `null` when the denominator is zero (no input-side tokens
 * yet — usually a never-active session). Caller renders `—`.
 */
export function cacheHitRate(row: {
  tokens_input: number;
  tokens_cache_creation: number;
  tokens_cache_read: number;
}): number | null {
  const denom =
    row.tokens_input + row.tokens_cache_creation + row.tokens_cache_read;
  if (denom === 0) return null;
  return row.tokens_cache_read / denom;
}

/** Pretty-print a hit rate as `"83%"`, or `"—"` for null. */
export function formatHitRate(r: number | null): string {
  if (r == null) return "—";
  return `${Math.round(r * 100)}%`;
}

/**
 * Strip the leading `claude-` family prefix when rendering a model
 * id in tight column space. `claude-opus-4-7` → `opus-4-7`. Falls
 * back to the raw id for unknown shapes.
 */
export function shortModelId(id: string): string {
  if (id.startsWith("claude-")) return id.slice("claude-".length);
  return id;
}

export function CostTab() {
  const [choice, setChoice] = useState<WindowChoice>("7d");
  const [report, setReport] = useState<LocalUsageReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sortKey, setSortKey] = useState<SortKey>("cost");
  const [sortDir, setSortDir] = useState<SortDir>("desc");
  const seqRef = useRef(0);

  const fetchReport = useCallback(
    async (c: WindowChoice) => {
      const seq = ++seqRef.current;
      setLoading(true);
      try {
        const r = await api.localUsageAggregate(toSpec(c));
        if (seq !== seqRef.current) return;
        setReport(r);
        setError(null);
      } catch (e) {
        if (seq !== seqRef.current) return;
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (seq === seqRef.current) setLoading(false);
      }
    },
    [],
  );

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
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [choice, fetchReport]);

  const setTier = useCallback(
    async (t: PriceTierId) => {
      try {
        await api.pricingTierSet(t);
        // The fresh report carries the updated tier label in its
        // `pricing_tier` field, so a single re-fetch keeps the UI
        // in sync without a separate state path.
        await fetchReport(choice);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
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
        pricingTier={report?.pricing_tier ?? null}
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
        Window
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
            {o.label}
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
        Tier
      </label>
      <select
        id="cost-tab-tier"
        value={pricingTier ?? "anthropic_api"}
        onChange={(e) => onTier(e.target.value as PriceTierId)}
        title="Platform you're billed through. Drives the cost label and (where verified) the rate multiplier."
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
            {o.label}
          </option>
        ))}
      </select>
      {pricingSource && (
        <span
          title={pricingError ?? `Price table source: ${pricingSource}`}
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
          {pricingError ? " · stale" : ""}
        </span>
      )}
      {loading && (
        <span
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
          }}
        >
          loading…
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
  const t = report?.totals;
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
        label="Total cost"
        value={
          t && t.cost_usd != null
            ? `$${t.cost_usd.toFixed(2)}`
            : loading
              ? dash
              : t
                ? "n/a"
                : dash
        }
        sub="install-wide"
      />
      <Tile
        label="Tokens in"
        value={renderTokens(t?.tokens_input)}
        sub={
          t
            ? `cache hit ${formatHitRate(cacheHitRate(t))}`
            : renderTokens(undefined, "cache read")
        }
      />
      <Tile
        label="Tokens out"
        value={renderTokens(t?.tokens_output)}
        sub={renderTokens(t?.tokens_cache_creation, "cache write")}
      />
      <Tile
        label="Sessions"
        value={t ? String(t.session_count) : dash}
        sub={
          t && t.unpriced_sessions > 0
            ? `${t.unpriced_sessions} unpriced`
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

function formatCompact(n: number): string {
  if (n < 1_000) return String(n);
  if (n < 1_000_000) return `${(n / 1_000).toFixed(1)}k`;
  if (n < 1_000_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  return `${(n / 1_000_000_000).toFixed(2)}B`;
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
        No sessions in this window.
      </div>
    );
  }
  return (
    <table
      style={{
        width: "100%",
        borderCollapse: "collapse",
        fontSize: "var(--fs-xs)",
        fontVariantNumeric: "tabular-nums",
      }}
    >
      <thead>
        <tr style={{ background: "var(--bg-sunken)" }}>
          <ThSort
            value="project"
            current={sortKey}
            dir={sortDir}
            onSort={onSort}
            align="left"
          >
            Project
          </ThSort>
          <ThSort
            value="sessions"
            current={sortKey}
            dir={sortDir}
            onSort={onSort}
          >
            Sess
          </ThSort>
          <ThSort
            value="last"
            current={sortKey}
            dir={sortDir}
            onSort={onSort}
          >
            Last active
          </ThSort>
          <ThSort
            value="input"
            current={sortKey}
            dir={sortDir}
            onSort={onSort}
          >
            Input
          </ThSort>
          <ThSort
            value="output"
            current={sortKey}
            dir={sortDir}
            onSort={onSort}
          >
            Output
          </ThSort>
          <ThSort
            value="cache_hit"
            current={sortKey}
            dir={sortDir}
            onSort={onSort}
          >
            Cache hit
          </ThSort>
          <Th align="left">Models</Th>
          <ThSort
            value="cost"
            current={sortKey}
            dir={sortDir}
            onSort={onSort}
          >
            Cost
          </ThSort>
          <Th align="center">⚠</Th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <Row key={r.project_path} row={r} />
        ))}
      </tbody>
    </table>
  );
}

function Row({ row }: { row: ProjectUsageRow }) {
  const cost =
    row.cost_usd == null ? "n/a" : `$${row.cost_usd.toFixed(2)}`;
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
    <tr style={{ borderBottom: "var(--bw-hair) solid var(--line)" }}>
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
            ? "no input-side tokens"
            : `${formatCompact(row.tokens_cache_read)} cache-read of ${formatCompact(row.tokens_input + row.tokens_cache_creation + row.tokens_cache_read)} prompt tokens`
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
            title={`${warn} session(s) had no priced model`}
            style={{ color: "var(--warn)" }}
          >
            ⚠
          </span>
        ) : null}
      </Td>
    </tr>
  );
}

/** Inline badge group for the row's model-mix column. Sorted by
 *  session-count descending so the most-used model shows first.
 *  Renders nothing for empty mixes — keeps the column quiet on
 *  user-only sessions. */
function ModelBadges({ mix }: { mix: Record<string, number> }) {
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
          title={`${model} · ${count} session${count === 1 ? "" : "s"}`}
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

function Th({
  children,
  align,
}: {
  children: React.ReactNode;
  align?: "left" | "right" | "center";
}) {
  return (
    <th
      style={{
        textAlign: align ?? "right",
        padding: "var(--sp-6) var(--sp-8)",
        fontWeight: 500,
        fontSize: "var(--fs-2xs)",
        color: "var(--fg-muted)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
        borderBottom: "var(--bw-hair) solid var(--line)",
      }}
    >
      {children}
    </th>
  );
}

function ThSort({
  value,
  current,
  dir,
  onSort,
  children,
  align,
}: {
  value: SortKey;
  current: SortKey;
  dir: SortDir;
  onSort: (k: SortKey) => void;
  children: React.ReactNode;
  align?: "left" | "right" | "center";
}) {
  const active = current === value;
  const arrow = active ? (dir === "asc" ? " ▲" : " ▼") : "";
  return (
    <th
      style={{
        textAlign: align ?? "right",
        padding: "var(--sp-6) var(--sp-8)",
        fontWeight: 500,
        fontSize: "var(--fs-2xs)",
        color: active ? "var(--fg)" : "var(--fg-muted)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
        borderBottom: "var(--bw-hair) solid var(--line)",
        cursor: "pointer",
        userSelect: "none",
      }}
      onClick={() => onSort(value)}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSort(value);
        }
      }}
      aria-sort={active ? (dir === "asc" ? "ascending" : "descending") : "none"}
    >
      {children}
      {arrow}
    </th>
  );
}

function Td({
  children,
  align,
  title,
  style,
}: {
  children: React.ReactNode;
  align?: "left" | "right" | "center";
  title?: string;
  style?: React.CSSProperties;
}) {
  return (
    <td
      title={title}
      style={{
        textAlign: align ?? "left",
        padding: "var(--sp-6) var(--sp-8)",
        color: "var(--fg)",
        ...style,
      }}
    >
      {children}
    </td>
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
        ⚠ {u} of {total} session{total === 1 ? "" : "s"} used a model not
        in the price table — token counts above include them; cost
        excludes them.
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
        Refresh prices
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
  const cmp = (a: ProjectUsageRow, b: ProjectUsageRow): number => {
    switch (key) {
      case "cost":
        return nullableNumberCmp(a.cost_usd, b.cost_usd);
      case "sessions":
        return a.session_count - b.session_count;
      case "last":
        return nullableNumberCmp(a.last_active_ms, b.last_active_ms);
      case "input":
        return a.tokens_input - b.tokens_input;
      case "output":
        return a.tokens_output - b.tokens_output;
      case "cache_hit":
        return nullableNumberCmp(cacheHitRate(a), cacheHitRate(b));
      case "project":
        return a.project_path.localeCompare(b.project_path);
    }
  };
  const sorted = [...rows].sort(cmp);
  return dir === "asc" ? sorted : sorted.reverse();
}

/** Render the project's basename — the CWD's leaf folder name. CC
 *  project CWDs share long `/Users/<user>/...` prefixes that waste
 *  column width without telling the user anything new; the leaf
 *  folder is what they recognise ("claudepot-app", "vmark"). The
 *  full path is on the row's `title` for hover disclosure.
 *  Windows-aware for `\` separators. */
function displayPath(p: string): string {
  if (!p) return p;
  const trimmed = p.replace(/[/\\]+$/, "");
  const segs = trimmed.split(/[/\\]/).filter(Boolean);
  return segs[segs.length - 1] ?? trimmed;
}

/** Compare two `T | null` numerics so nulls sort to the end in ascending
 *  order. Without this, a sort by `cost` puts the unpriced-cost rows at
 *  the top of the descending view, hiding the actually-expensive ones. */
function nullableNumberCmp(
  a: number | null,
  b: number | null,
): number {
  if (a == null && b == null) return 0;
  if (a == null) return -1;
  if (b == null) return 1;
  return a - b;
}
