import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import type {
  ActivityCard,
  CardKindLabel,
  CardsCount,
  CardsRecentQuery,
  SeverityLabel,
} from "../types";
import {
  aggregate,
  daySeries,
  SeverityMix,
  Sparkbars,
  TopKinds,
} from "./events/charts";

/**
 * Events section — per-event forensic surface plus an at-a-glance
 * aggregate strip across the top.
 *
 * Layout (top → bottom, then left → right):
 *   row 1 — header (title, total/new counts, reindex/refresh/mark-seen)
 *   row 2 — metrics strip (cards-by-day · severity mix · top kinds)
 *           derived from a *filter-independent* fetch so the strip
 *           always reflects "what's happening overall" while the
 *           list below shows the user's drill-down
 *   row 3 — two-pane: filter rail (left) · card stream (right)
 *
 * Pre-merge there were two tabs (`Events` and `Trends`) showing the
 * same dataset at two zoom levels. Standard log-viewer pattern is
 * one surface with a chart-on-top — see `dev-docs/activity-cards-
 * design.md` §9.
 *
 * Live updates: the classifier persists cards into
 * `~/.claudepot/sessions.db`; live tail emits `CardEmitted` deltas.
 * This component refetches on every delta from `live-all` plus a
 * 5-second polling fallback for cross-session visibility. The
 * aggregate strip refetches on the same cadence (cheap — same
 * `cards_recent` command, just with a higher limit and no filters).
 *
 * Suppression rules and severity meanings live in
 * `dev-docs/activity-cards-design.md` §2 / §6 — when in doubt,
 * read those before adding a filter UI.
 */

// All filter-vocab values match `CardKindLabel` / `SeverityLabel`
// in src/types.ts. Updated in lock-step with the Rust catalog.
const KIND_OPTIONS: { value: CardKindLabel; label: string }[] = [
  { value: "hook", label: "Hook failures" },
  { value: "hook-slow", label: "Slow hooks" },
  { value: "tool-error", label: "Tool errors" },
  { value: "agent", label: "Agent returns" },
  { value: "agent-stranded", label: "Agent stranded" },
  { value: "milestone", label: "Milestones" },
];

const SEVERITY_OPTIONS: {
  value: "info" | "notice" | "warn" | "error";
  label: string;
}[] = [
  { value: "info", label: "All" },
  { value: "notice", label: "Notice+" },
  { value: "warn", label: "Warn+" },
  { value: "error", label: "Error only" },
];

const DEFAULT_LIMIT = 200;
// Aggregate fetch is filter-independent — the metrics strip should
// always show the same overview regardless of the user's drill-down.
// 10k matches the limit the previous standalone `Trends` tab used and
// is comfortably above current scale (~4k cards on the reference
// machine). Bump this if the index grows past 50k and the
// client-side aggregation becomes noticeable.
const AGG_LIMIT = 10_000;
const AGG_QUERY: CardsRecentQuery = { minSeverity: "info", limit: AGG_LIMIT };

export function EventsSection() {
  const [cards, setCards] = useState<ActivityCard[]>([]);
  const [counts, setCounts] = useState<CardsCount | null>(null);
  const [aggCards, setAggCards] = useState<ActivityCard[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [reindexing, setReindexing] = useState(false);
  const [filters, setFilters] = useState<CardsRecentQuery>({
    minSeverity: "warn",
    limit: DEFAULT_LIMIT,
  });

  const refresh = useCallback(async () => {
    try {
      setError(null);
      // Three parallel fetches: filtered list, new-since counter, and
      // the unfiltered aggregate set that backs the metrics strip.
      const [list, c, agg] = await Promise.all([
        api.cardsRecent(filters),
        api.cardsCountNewSince(filters),
        api.cardsRecent(AGG_QUERY),
      ]);
      setCards(list);
      setCounts(c);
      setAggCards(agg);
      setLoading(false);
    } catch (e) {
      setError(String(e));
      setLoading(false);
    }
  }, [filters]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Live updates — subscribe to the `live::*` channel pattern. Tauri
  // doesn't support channel wildcards, so we listen on the public
  // `live-all` channel for session lifecycle and refresh on tick;
  // per-session CardEmitted deltas land on `live::<sid>` channels
  // which the user's existing Sessions section already subscribes to.
  // For cross-session card visibility, a 5-second poll is the
  // simplest correct approach; live deltas for the *selected* session
  // arrive via the subscriber inside the session detail viewer.
  useEffect(() => {
    const unsubP = listen("live-all", () => {
      void refresh();
    });
    const t = setInterval(() => void refresh(), 5_000);
    return () => {
      void unsubP.then((u) => u());
      clearInterval(t);
    };
  }, [refresh]);

  const handleReindex = useCallback(async () => {
    setReindexing(true);
    try {
      await api.cardsReindex();
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setReindexing(false);
    }
  }, [refresh]);

  const markAllSeen = useCallback(async () => {
    if (!cards.length) return;
    const newest = cards[0];
    await api.cardsSetLastSeen(newest.id);
    await refresh();
  }, [cards, refresh]);

  const handleCardClick = useCallback(async (card: ActivityCard) => {
    try {
      const nav = await api.cardsNavigate(card.id);
      if (!nav) return;
      // Hand off target session via a window CustomEvent — the App
      // shell switches to the Activities section and seeds the
      // Sessions detail with this path. Per-line scroll-to-offset
      // is deferred (the shell doesn't yet thread byteOffset down
      // through SessionDetail); landing on the right transcript is
      // the MVP.
      window.dispatchEvent(
        new CustomEvent("claudepot:navigate-section", {
          detail: { id: "activities", sessionPath: nav.sessionPath },
        }),
      );
    } catch (e) {
      setError(String(e));
    }
  }, []);

  return (
    <div
      style={{
        display: "flex",
        height: "100%",
        minHeight: 0,
        background: "var(--bg)",
      }}
    >
      <FilterRail filters={filters} onChange={setFilters} />
      <div
        style={{
          flex: 1,
          minWidth: 0,
          display: "flex",
          flexDirection: "column",
          minHeight: 0,
        }}
      >
        <Header
          counts={counts}
          reindexing={reindexing}
          onReindex={handleReindex}
          onMarkAllSeen={markAllSeen}
          onRefresh={() => void refresh()}
        />
        <MetricsStrip cards={aggCards} loading={loading} />
        <CardStream
          cards={cards}
          loading={loading}
          error={error}
          lastSeenId={counts?.lastSeenId ?? null}
          onCardClick={handleCardClick}
        />
      </div>
    </div>
  );
}

// ── Metrics strip ────────────────────────────────────────────────

/**
 * Three-cell aggregate summary that sits above the card stream.
 *   cell 1 — total + cards-by-day sparkbar (last 30d)
 *   cell 2 — severity mix bar + counts
 *   cell 3 — top 5 kinds with proportional bars
 *
 * Filter-independent: derives from a separate unfiltered fetch
 * (AGG_QUERY) so the user always sees the global picture while
 * filtering the list below. Hidden when the aggregate set is empty
 * — first paint, after a reindex, or no cards at all.
 */
function MetricsStrip({ cards, loading }: { cards: ActivityCard[]; loading: boolean }) {
  const agg = useMemo(() => aggregate(cards), [cards]);
  const series = useMemo(() => daySeries(agg.byDay, 30), [agg.byDay]);

  // Hide cleanly when there's nothing to show — the empty state on
  // the card list below carries the "no cards yet" message; we don't
  // need a parallel empty banner here.
  if (loading || agg.total === 0) return null;

  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "1fr 1fr 1fr",
        gap: "var(--sp-12)",
        padding: "var(--sp-10) var(--sp-16)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-sunken)",
      }}
    >
      <MetricCell title={`${agg.total.toLocaleString()} cards`} subtitle="last 30 days">
        <Sparkbars data={series} />
      </MetricCell>
      <MetricCell title="Severity mix">
        <SeverityMix agg={agg} />
      </MetricCell>
      <MetricCell title="Top kinds">
        <TopKinds agg={agg} limit={5} labelFor={kindLabel} />
      </MetricCell>
    </div>
  );
}

function MetricCell({
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
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-6)",
        minWidth: 0,
        color: "var(--fg)",
      }}
    >
      <div>
        <span
          className="mono-cap"
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-muted)",
          }}
        >
          {title}
        </span>
        {subtitle && (
          <span
            style={{
              fontSize: "var(--fs-2xs)",
              color: "var(--fg-faint)",
              marginLeft: "var(--sp-6)",
            }}
          >
            {subtitle}
          </span>
        )}
      </div>
      {children}
    </section>
  );
}

// ── Sub-components ───────────────────────────────────────────────

interface FilterRailProps {
  filters: CardsRecentQuery;
  onChange: (next: CardsRecentQuery) => void;
}

function FilterRail({ filters, onChange }: FilterRailProps) {
  const togglekind = (k: CardKindLabel) => {
    const cur = new Set(filters.kinds ?? []);
    if (cur.has(k)) cur.delete(k);
    else cur.add(k);
    onChange({ ...filters, kinds: Array.from(cur) });
  };
  return (
    <aside
      style={{
        width: 240,
        flexShrink: 0,
        borderRight: "1px solid var(--border)",
        padding: "var(--sp-16)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-20)",
        overflowY: "auto",
      }}
    >
      <FilterGroup label="Severity">
        <select
          value={filters.minSeverity ?? "info"}
          onChange={(e) =>
            onChange({
              ...filters,
              minSeverity: e.target.value as CardsRecentQuery["minSeverity"],
            })
          }
          style={selectStyle}
        >
          {SEVERITY_OPTIONS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </FilterGroup>
      <FilterGroup label="Kind">
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-4)" }}>
          {KIND_OPTIONS.map((opt) => {
            const checked = filters.kinds?.includes(opt.value) ?? false;
            return (
              <label
                key={opt.value}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--sp-8)",
                  cursor: "pointer",
                  fontSize: "var(--fs-sm)",
                  color: "var(--fg)",
                }}
              >
                <input
                  type="checkbox"
                  checked={checked}
                  onChange={() => togglekind(opt.value)}
                />
                {opt.label}
              </label>
            );
          })}
        </div>
      </FilterGroup>
      <FilterGroup label="Plugin">
        <input
          type="text"
          value={filters.plugin ?? ""}
          placeholder="grill, mermaid-preview, …"
          onChange={(e) =>
            onChange({ ...filters, plugin: e.target.value || undefined })
          }
          style={inputStyle}
        />
      </FilterGroup>
      <FilterGroup label="Project (cwd prefix)">
        <input
          type="text"
          value={filters.projectPathPrefix ?? ""}
          placeholder="/Users/x/proj"
          onChange={(e) =>
            onChange({
              ...filters,
              projectPathPrefix: e.target.value || undefined,
            })
          }
          style={inputStyle}
        />
      </FilterGroup>
      <FilterGroup label="Window">
        <select
          value={filters.sinceMs ? String(rangeBucket(filters.sinceMs)) : "all"}
          onChange={(e) => {
            const v = e.target.value;
            const sinceMs =
              v === "all"
                ? undefined
                : Date.now() - parseInt(v, 10);
            onChange({ ...filters, sinceMs });
          }}
          style={selectStyle}
        >
          <option value="all">All time</option>
          <option value={String(60 * 60 * 1000)}>Last 1 h</option>
          <option value={String(24 * 60 * 60 * 1000)}>Last 24 h</option>
          <option value={String(7 * 24 * 60 * 60 * 1000)}>Last 7 d</option>
        </select>
      </FilterGroup>
    </aside>
  );
}

function FilterGroup({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-4)" }}>
      <div
        style={{
          fontSize: "var(--fs-xs)",
          textTransform: "uppercase",
          letterSpacing: "0.06em",
          color: "var(--muted)",
        }}
      >
        {label}
      </div>
      {children}
    </div>
  );
}

interface HeaderProps {
  counts: CardsCount | null;
  reindexing: boolean;
  onReindex: () => void;
  onMarkAllSeen: () => void;
  onRefresh: () => void;
}

function Header({
  counts,
  reindexing,
  onReindex,
  onMarkAllSeen,
  onRefresh,
}: HeaderProps) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding: "var(--sp-12) var(--sp-16)",
        borderBottom: "1px solid var(--border)",
        gap: "var(--sp-12)",
      }}
    >
      <div style={{ display: "flex", alignItems: "baseline", gap: "var(--sp-12)" }}>
        <h2
          style={{
            margin: 0,
            fontSize: "var(--fs-md)",
            fontWeight: 600,
            color: "var(--fg)",
          }}
        >
          Events
        </h2>
        {counts && (
          <span
            style={{
              fontSize: "var(--fs-sm)",
              color: "var(--muted)",
            }}
          >
            {counts.total.toLocaleString()} total
            {counts.new > 0 && (
              <>
                {" · "}
                <span
                  style={{
                    color: "var(--accent)",
                    fontWeight: 600,
                  }}
                >
                  {counts.new} new
                </span>
              </>
            )}
          </span>
        )}
      </div>
      <div style={{ display: "flex", gap: "var(--sp-8)" }}>
        <button
          onClick={onMarkAllSeen}
          disabled={!counts?.new}
          style={btnStyle}
          title="Set last-seen to the newest card; clears the badge"
        >
          Mark all seen
        </button>
        <button onClick={onRefresh} style={btnStyle} title="Re-fetch from the index">
          Refresh
        </button>
        <button
          onClick={onReindex}
          disabled={reindexing}
          style={btnStyle}
          title="Walk every JSONL and rebuild the index"
        >
          {reindexing ? "Reindexing…" : "Reindex"}
        </button>
      </div>
    </div>
  );
}

interface CardStreamProps {
  cards: ActivityCard[];
  loading: boolean;
  error: string | null;
  lastSeenId: number | null;
  onCardClick: (card: ActivityCard) => void;
}

function CardStream({ cards, loading, error, lastSeenId, onCardClick }: CardStreamProps) {
  if (loading) {
    return (
      <div style={emptyStyle}>Loading…</div>
    );
  }
  if (error) {
    return (
      <div style={{ ...emptyStyle, color: "var(--danger)" }}>
        {error}
      </div>
    );
  }
  if (cards.length === 0) {
    return (
      <div style={emptyStyle}>
        No cards. Try lowering the severity filter, or click <em>Reindex</em> to
        backfill from your JSONL history.
      </div>
    );
  }
  return (
    <ul
      style={{
        margin: 0,
        padding: 0,
        listStyle: "none",
        overflowY: "auto",
        flex: 1,
        minHeight: 0,
      }}
    >
      {cards.map((c) => (
        <CardRow
          key={c.id}
          card={c}
          isNew={lastSeenId !== null && c.id > lastSeenId}
          onClick={() => onCardClick(c)}
        />
      ))}
    </ul>
  );
}

interface CardRowProps {
  card: ActivityCard;
  isNew: boolean;
  onClick: () => void;
}

function CardRow({ card, isNew, onClick }: CardRowProps) {
  return (
    <li
      onClick={onClick}
      style={{
        padding: "var(--sp-12) var(--sp-16)",
        borderBottom: "1px solid var(--border)",
        cursor: "pointer",
        display: "flex",
        gap: "var(--sp-12)",
        alignItems: "flex-start",
        transition: "background 120ms",
        background: isNew ? "var(--bg-elev)" : "transparent",
      }}
      onMouseEnter={(e) => {
        e.currentTarget.style.background = "var(--bg-elev)";
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = isNew ? "var(--bg-elev)" : "transparent";
      }}
    >
      <SeverityChip severity={card.severity} />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "baseline",
            gap: "var(--sp-12)",
          }}
        >
          <div
            style={{
              fontSize: "var(--fs-sm)",
              fontWeight: 500,
              color: "var(--fg)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {card.title}
          </div>
          <time
            style={{
              fontSize: "var(--fs-xs)",
              color: "var(--muted)",
              flexShrink: 0,
              fontVariantNumeric: "tabular-nums",
            }}
          >
            {formatTime(card.ts_ms)}
          </time>
        </div>
        {card.subtitle && (
          <div
            style={{
              fontSize: "var(--fs-xs)",
              color: "var(--muted)",
              marginTop: "var(--sp-2)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {card.subtitle}
          </div>
        )}
        {card.help?.rendered && (
          <div
            style={{
              fontSize: "var(--fs-xs)",
              color: "var(--fg)",
              marginTop: "var(--sp-4)",
              padding: "var(--sp-4) var(--sp-8)",
              background: "var(--bg-elev)",
              borderLeft: "2px solid var(--accent)",
              borderRadius: "var(--radius-sm)",
            }}
          >
            ↳ {card.help.rendered}
          </div>
        )}
        <div
          style={{
            display: "flex",
            gap: "var(--sp-8)",
            marginTop: "var(--sp-6)",
            fontSize: "var(--fs-xs)",
            color: "var(--muted)",
            flexWrap: "wrap",
          }}
        >
          <span>{kindLabel(card.kind)}</span>
          <span>·</span>
          <span title={card.cwd}>{basename(card.cwd)}</span>
          {card.git_branch && (
            <>
              <span>·</span>
              <span>{card.git_branch}</span>
            </>
          )}
          {card.plugin && (
            <>
              <span>·</span>
              <span>plugin:{card.plugin}</span>
            </>
          )}
          {card.source_ref && (
            <>
              <span>·</span>
              <span title={card.source_ref.path}>
                {card.source_ref.scope}: {basename(card.source_ref.path)}
                {card.source_ref.line ? `:${card.source_ref.line}` : ""}
              </span>
            </>
          )}
        </div>
      </div>
    </li>
  );
}

function SeverityChip({ severity }: { severity: SeverityLabel }) {
  const { bg, fg } = severityColors(severity);
  return (
    <div
      style={{
        width: 60,
        flexShrink: 0,
        padding: "var(--sp-2) var(--sp-6)",
        borderRadius: "var(--radius-sm)",
        background: bg,
        color: fg,
        fontSize: 10,
        fontWeight: 600,
        textAlign: "center",
        textTransform: "uppercase",
        letterSpacing: "0.04em",
        lineHeight: 1.4,
      }}
    >
      {severity}
    </div>
  );
}

// ── Helpers ──────────────────────────────────────────────────────

function severityColors(s: SeverityLabel): { bg: string; fg: string } {
  switch (s) {
    case "ERROR":
      return { bg: "var(--danger-bg, rgba(220, 38, 38, 0.15))", fg: "var(--danger, #dc2626)" };
    case "WARN":
      return { bg: "var(--warn-bg, rgba(217, 119, 6, 0.15))", fg: "var(--warn, #d97706)" };
    case "NOTICE":
      return { bg: "var(--bg-elev)", fg: "var(--fg)" };
    case "INFO":
    default:
      return { bg: "var(--bg-elev)", fg: "var(--muted)" };
  }
}

function kindLabel(k: CardKindLabel): string {
  return KIND_OPTIONS.find((o) => o.value === k)?.label ?? k;
}

function formatTime(ms: number): string {
  const d = new Date(ms);
  const now = new Date();
  const sameDay = d.toDateString() === now.toDateString();
  if (sameDay) {
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  return d.toLocaleString([], {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function basename(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

function rangeBucket(sinceMs: number): number {
  // Reverse-derive the dropdown bucket from a since-ms value so the
  // selector reflects the active filter on rerender. Approximate match
  // against the four offered windows.
  const delta = Date.now() - sinceMs;
  const buckets = [
    60 * 60 * 1000,
    24 * 60 * 60 * 1000,
    7 * 24 * 60 * 60 * 1000,
  ];
  let best = buckets[0];
  let bestDist = Math.abs(delta - best);
  for (const b of buckets) {
    const d = Math.abs(delta - b);
    if (d < bestDist) {
      best = b;
      bestDist = d;
    }
  }
  return best;
}

const inputStyle: React.CSSProperties = {
  fontSize: "var(--fs-sm)",
  padding: "var(--sp-4) var(--sp-8)",
  border: "1px solid var(--border)",
  borderRadius: "var(--radius-sm)",
  background: "var(--bg)",
  color: "var(--fg)",
  fontFamily: "inherit",
};

const selectStyle: React.CSSProperties = {
  ...inputStyle,
  cursor: "pointer",
};

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
  flex: 1,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  color: "var(--muted)",
  fontSize: "var(--fs-sm)",
  padding: "var(--sp-32)",
  textAlign: "center",
};
