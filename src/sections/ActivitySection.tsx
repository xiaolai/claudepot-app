import { useEffect, useMemo, useState } from "react";
import { api } from "../api";
import { ScreenHeader } from "../shell/ScreenHeader";
import { useSessionLive } from "../hooks/useSessionLive";
import type { ActivityTrends, LiveSessionSummary } from "../types";

/**
 * Activity section — `⌘5`. Two views: "Now" lists currently-live
 * sessions with live aggregates (token burn, model mix); "Trends"
 * shows 24h / 7d / 30d rollups from the metrics store.
 *
 * Paper-mono: one primary concept per card, hairline borders,
 * semantic tokens only.
 */
type Mode = "now" | "trends";

interface ActivitySectionProps {
  /** Optional: route to the Sessions deep-link (M1) or live pane
   *  (M2+) when the user clicks a live-session card. */
  onOpenSession?: (s: LiveSessionSummary) => void;
}

export function ActivitySection({ onOpenSession }: ActivitySectionProps) {
  const live = useSessionLive();
  const [mode, setMode] = useState<Mode>("now");
  // Local mirror of the activity_enabled preference. `null` while
  // the initial fetch is in flight so we don't flash "disabled" at
  // users who already opted in.
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [enabling, setEnabling] = useState(false);
  useEffect(() => {
    let cancelled = false;
    api
      .preferencesGet()
      .then((p) => {
        if (!cancelled) setEnabled(p.activity_enabled);
      })
      .catch(() => {
        // Non-Tauri env — treat as enabled so local dev doesn't dead-end.
        if (!cancelled) setEnabled(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const enableActivity = async () => {
    setEnabling(true);
    try {
      await api.preferencesSetActivity({ enabled: true, consentSeen: true });
      await api.sessionLiveStart();
      setEnabled(true);
      // Let the shell's sidebar drop the "Off" chip without polling.
      window.dispatchEvent(
        new CustomEvent("cp-activity-enabled-changed", {
          detail: { enabled: true },
        }),
      );
    } catch {
      // Swallow — the settings pane surfaces the underlying error.
    } finally {
      setEnabling(false);
    }
  };

  return (
    <>
      <ScreenHeader
        title="Activity"
        subtitle={
          enabled === false
            ? "Activity is off."
            : live.length === 0
              ? "No live sessions."
              : `${live.length} live session${live.length === 1 ? "" : "s"}`
        }
      />

      <div
        style={{
          padding: "var(--sp-14) var(--sp-32) 0",
          borderBottom: "var(--bw-hair) solid var(--line)",
          background: "var(--bg)",
          flexShrink: 0,
        }}
      >
        <ModeToggle mode={mode} onChange={setMode} />
      </div>

      <div
        style={{
          flex: 1,
          minHeight: 0,
          overflow: "auto",
          padding: "var(--sp-20) var(--sp-28)",
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-20)",
        }}
      >
        {enabled === false ? (
          <DisabledState busy={enabling} onEnable={enableActivity} />
        ) : mode === "now" ? (
          live.length === 0 ? (
            <EmptyState />
          ) : (
            <>
              <LiveSessionCards
                sessions={sortSessions(live)}
                onOpenSession={onOpenSession}
              />
              <AggregateStats sessions={live} />
            </>
          )
        ) : (
          <TrendsPane />
        )}
      </div>
    </>
  );
}

function DisabledState({
  busy,
  onEnable,
}: {
  busy: boolean;
  onEnable: () => void;
}) {
  return (
    <div
      style={{
        padding: "var(--sp-32)",
        border: "var(--bw-hair) dashed var(--line)",
        borderRadius: "var(--r-2)",
        textAlign: "center",
        color: "var(--fg-muted)",
        fontSize: "var(--fs-sm)",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: "var(--sp-10)",
      }}
    >
      <p
        style={{
          margin: 0,
          fontSize: "var(--fs-md)",
          fontWeight: 500,
          color: "var(--fg)",
        }}
      >
        Activity is off.
      </p>
      <p
        style={{
          margin: 0,
          fontSize: "var(--fs-xs)",
          color: "var(--fg-faint)",
          maxWidth: "var(--content-cap-sm)",
        }}
      >
        When on, Claudepot polls your live Claude Code sessions and
        streams per-session deltas. Nothing leaves your machine.
      </p>
      <button
        type="button"
        onClick={onEnable}
        disabled={busy}
        className="pm-focus"
        style={{
          marginTop: "var(--sp-4)",
          padding: "var(--sp-6) var(--sp-14)",
          fontSize: "var(--fs-sm)",
          fontWeight: 500,
          color: "var(--on-color)",
          background: "var(--accent)",
          border: "var(--bw-hair) solid var(--accent)",
          borderRadius: "var(--r-2)",
          cursor: busy ? "default" : "pointer",
          opacity: busy ? "var(--opacity-dimmed)" : 1,
        }}
      >
        {busy ? "Enabling…" : "Enable Activity"}
      </button>
    </div>
  );
}

function ModeToggle({
  mode,
  onChange,
}: {
  mode: Mode;
  onChange: (m: Mode) => void;
}) {
  const opts: { id: Mode; label: string }[] = [
    { id: "now", label: "Now" },
    { id: "trends", label: "Trends" },
  ];
  return (
    <div
      role="tablist"
      style={{
        display: "inline-flex",
        gap: "var(--sp-2)",
        padding: "var(--sp-2)",
        background: "var(--bg-sunken)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        marginBottom: "var(--sp-12)",
      }}
    >
      {opts.map((o) => {
        const current = mode === o.id;
        return (
          <button
            key={o.id}
            type="button"
            role="tab"
            aria-selected={current}
            onClick={() => onChange(o.id)}
            style={{
              padding: "var(--sp-4) var(--sp-12)",
              fontSize: "var(--fs-xs)",
              fontWeight: 500,
              color: current ? "var(--fg)" : "var(--fg-muted)",
              background: current ? "var(--bg-raised)" : "transparent",
              border: current
                ? "var(--bw-hair) solid var(--line)"
                : "var(--bw-hair) solid transparent",
              borderRadius: "var(--r-1)",
              letterSpacing: "var(--ls-wide)",
              textTransform: "uppercase",
              cursor: "pointer",
            }}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

function EmptyState() {
  return (
    <div
      style={{
        padding: "var(--sp-32)",
        border: "var(--bw-hair) dashed var(--line)",
        borderRadius: "var(--r-2)",
        textAlign: "center",
        color: "var(--fg-muted)",
        fontSize: "var(--fs-sm)",
      }}
    >
      <p style={{ margin: 0, marginBottom: "var(--sp-8)" }}>
        No live Claude sessions right now.
      </p>
      <p
        style={{
          margin: 0,
          fontSize: "var(--fs-xs)",
          color: "var(--fg-faint)",
        }}
      >
        Start <code>claude</code> in any project to see it light up
        here.
      </p>
    </div>
  );
}

// ── Session cards ──────────────────────────────────────────────────

function LiveSessionCards({
  sessions,
  onOpenSession,
}: {
  sessions: LiveSessionSummary[];
  onOpenSession?: (s: LiveSessionSummary) => void;
}) {
  return (
    <section>
      <Heading>Live now</Heading>
      <div
        role={onOpenSession ? "listbox" : undefined}
        aria-label={onOpenSession ? "Live sessions" : undefined}
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-10)",
        }}
      >
        {sessions.map((s) => (
          <SessionCard
            key={s.session_id}
            summary={s}
            onOpen={onOpenSession}
          />
        ))}
      </div>
    </section>
  );
}

function SessionCard({
  summary,
  onOpen,
}: {
  summary: LiveSessionSummary;
  onOpen?: (s: LiveSessionSummary) => void;
}) {
  const label = projectLabel(summary.cwd);
  const alerting = summary.errored || summary.stuck;
  const interactive = !!onOpen && !!summary.transcript_path;
  const handleActivate = () => {
    if (interactive) onOpen!(summary);
  };
  return (
    <article
      role={interactive ? "option" : undefined}
      tabIndex={interactive ? 0 : undefined}
      aria-selected={interactive ? false : undefined}
      onClick={interactive ? handleActivate : undefined}
      onKeyDown={
        interactive
          ? (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                handleActivate();
              }
            }
          : undefined
      }
      className={interactive ? "pm-focus" : undefined}
      style={{
        display: "grid",
        gridTemplateColumns: "var(--sp-8) 1fr auto",
        alignItems: "center",
        columnGap: "var(--sp-12)",
        padding: "var(--sp-12) var(--sp-16)",
        border: "var(--bw-hair) solid var(--line)",
        borderLeft: alerting
          ? "var(--bw-strong) solid var(--warn)"
          : "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        background: "var(--bg)",
        cursor: interactive ? "pointer" : "default",
      }}
    >
      <StatusDot status={summary.status} errored={summary.errored} />
      <div style={{ minWidth: 0 }}>
        <div
          style={{
            fontSize: "var(--fs-sm)",
            fontWeight: 500,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {label}
        </div>
        <div
          style={{
            fontSize: "var(--fs-xs)",
            color: "var(--fg-muted)",
            marginTop: "var(--sp-2)",
          }}
        >
          {describeAction(summary)}
        </div>
      </div>
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "flex-end",
          gap: "var(--sp-2)",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
          fontVariantNumeric: "tabular-nums",
        }}
      >
        <span>{formatElapsedMs(summary.idle_ms)}</span>
        {summary.errored && (
          <span
            style={{
              color: "var(--warn)",
              fontWeight: 600,
              fontSize: "var(--fs-2xs)",
              letterSpacing: "var(--ls-wide)",
              textTransform: "uppercase",
            }}
          >
            errors ↑
          </span>
        )}
        {summary.stuck && !summary.errored && (
          <span
            style={{
              color: "var(--warn)",
              fontWeight: 600,
              fontSize: "var(--fs-2xs)",
              letterSpacing: "var(--ls-wide)",
              textTransform: "uppercase",
            }}
          >
            stuck
          </span>
        )}
        <span style={{ color: "var(--fg-faint)" }}>
          {familyShort(summary.model)}
        </span>
      </div>
    </article>
  );
}

function StatusDot({
  status,
  errored,
}: {
  status: LiveSessionSummary["status"];
  errored: boolean;
}) {
  const palette: Record<LiveSessionSummary["status"], string> = {
    busy: "var(--accent)",
    waiting: "transparent",
    idle: "transparent",
  };
  const ring =
    status === "idle"
      ? "var(--fg-faint)"
      : errored
        ? "var(--warn)"
        : "var(--accent)";
  return (
    <span
      aria-hidden
      style={{
        display: "inline-block",
        width: "var(--sp-8)",
        height: "var(--sp-8)",
        borderRadius: "50%",
        background: palette[status],
        border: `var(--bw-kbd) solid ${ring}`,
      }}
    />
  );
}

// ── Aggregate stats ────────────────────────────────────────────────

function AggregateStats({ sessions }: { sessions: LiveSessionSummary[] }) {
  const mix = useMemo(() => aggregateModelMix(sessions), [sessions]);
  return (
    <section>
      <Heading>Aggregates</Heading>
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fit, minmax(180px, 1fr))",
          gap: "var(--sp-10)",
        }}
      >
        <StatCard
          label="Live sessions"
          value={String(sessions.length)}
          sub={statusBreakdown(sessions)}
        />
        {mix.length > 0 && (
          <StatCard
            label="Model mix"
            value={mix[0] ?? ""}
            sub={mix.slice(1).join(" · ") || "single family"}
          />
        )}
      </div>
    </section>
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

// ── Trends ────────────────────────────────────────────────────────

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

function TrendsPane() {
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
    <section>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          marginBottom: "var(--sp-10)",
        }}
      >
        <Heading>Trends</Heading>
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

function Sparkline({
  values,
  peak,
}: {
  values: number[];
  peak: number;
}) {
  // Minimal inline-SVG sparkline — no external chart lib. Fixed
  // aspect so heights scale with peak, not with bucket count.
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

function Heading({ children }: { children: string }) {
  return (
    <h2
      style={{
        fontSize: "var(--fs-xs)",
        fontWeight: 600,
        color: "var(--fg-muted)",
        textTransform: "uppercase",
        letterSpacing: "var(--ls-wide)",
        margin: 0,
        marginBottom: "var(--sp-10)",
      }}
    >
      {children}
    </h2>
  );
}

// ── Pure helpers (exported for unit tests) ─────────────────────────

/** Priority tier for live-session sorting.
 *  0 = alerting (errored/stuck) — needs attention now
 *  1 = busy — actively working
 *  2 = waiting — paused for user input
 *  3 = idle — parked */
function sessionTier(s: LiveSessionSummary): number {
  if (s.errored || s.stuck) return 0;
  if (s.status === "busy") return 1;
  if (s.status === "waiting") return 2;
  return 3;
}

/** Sort live sessions: alerting → busy → waiting → idle.
 *  Within each tier, ascending idle_ms so the most recently
 *  active session floats to the top. Pure — returns a new array. */
export function sortSessions(
  sessions: LiveSessionSummary[],
): LiveSessionSummary[] {
  return [...sessions].sort((a, b) => {
    const dt = sessionTier(a) - sessionTier(b);
    if (dt !== 0) return dt;
    return a.idle_ms - b.idle_ms;
  });
}

export function projectLabel(cwd: string): string {
  const trimmed = cwd.replace(/[/\\]+$/, "");
  if (!trimmed) return cwd;
  const idx = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  const base = idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
  return base || trimmed;
}

export function familyShort(model: string | null): string {
  if (!model) return "—";
  if (model.includes("opus")) return "OPUS";
  if (model.includes("sonnet")) return "SON";
  if (model.includes("haiku")) return "HAI";
  return model.length > 10 ? model.slice(0, 9) + "…" : model;
}

export function describeAction(s: LiveSessionSummary): string {
  if (s.current_action) return s.current_action;
  if (s.status === "waiting" && s.waiting_for) return `waiting — ${s.waiting_for}`;
  if (s.status === "idle") return "idle — awaiting prompt";
  return "working…";
}

export function formatElapsedMs(ms: number): string {
  if (ms < 1000) return "—";
  if (ms < 10_000) return `${Math.floor(ms / 1000)}s`;
  const totalSec = Math.floor(ms / 1000);
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  if (m < 60) return `${m}:${String(s).padStart(2, "0")}`;
  const h = Math.floor(m / 60);
  return `${h}h${m % 60}m`;
}

export function countByStatus(
  sessions: LiveSessionSummary[],
  status: LiveSessionSummary["status"],
): number {
  return sessions.filter((s) => s.status === status).length;
}

/** Render-if-nonzero status breakdown: joins only non-zero counts
 *  so we never ship "0 busy · 0 waiting · 3 idle" — per design.md,
 *  zero-valued segments get filtered before the join. Falls back
 *  to an em dash when every count is zero (the parent gates on
 *  sessions.length > 0, so this branch is defensive only). */
export function statusBreakdown(sessions: LiveSessionSummary[]): string {
  const parts: string[] = [];
  const busy = countByStatus(sessions, "busy");
  const waiting = countByStatus(sessions, "waiting");
  const idle = countByStatus(sessions, "idle");
  if (busy > 0) parts.push(`${busy} busy`);
  if (waiting > 0) parts.push(`${waiting} waiting`);
  if (idle > 0) parts.push(`${idle} idle`);
  return parts.length > 0 ? parts.join(" · ") : "—";
}

/** Same shape as AppStatusBar.modelMix but formatted for the
 *  Activity card (e.g. 'OPUS 2'). Duplicated intentionally — the
 *  StatusBar version applies to peripheral surfaces with tighter
 *  label length, this one can expand. Kept separate so future
 *  styling divergence doesn't require a format-flag parameter. */
export function aggregateModelMix(
  sessions: LiveSessionSummary[],
): string[] {
  const counts = new Map<string, number>();
  for (const s of sessions) {
    if (!s.model) continue;
    const k = familyShort(s.model);
    counts.set(k, (counts.get(k) ?? 0) + 1);
  }
  return [...counts.entries()]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([k, n]) => `${k} ${n}`);
}
