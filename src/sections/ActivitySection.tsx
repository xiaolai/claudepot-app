import { useMemo } from "react";
import { ScreenHeader } from "../shell/ScreenHeader";
import { useSessionLive } from "../hooks/useSessionLive";
import type { LiveSessionSummary } from "../types";

/**
 * Activity section — `⌘5`. M4 ships the "Now" view only: a list of
 * currently-live sessions plus live aggregates (token burn, model
 * mix, spend-per-hour).
 *
 * The "Trends" view (24h / 7d / 30d) lands later — it needs a
 * durable metrics store (`sessions.db` columns or a new time-series
 * file) that's out of scope for this push.
 *
 * Paper-mono: one primary concept per card, hairline borders, semantic
 * tokens only, no charts until we have real data to plot.
 */
export function ActivitySection() {
  const live = useSessionLive();

  return (
    <>
      <ScreenHeader
        title="Activity"
        subtitle={
          live.length === 0
            ? "No live sessions."
            : `${live.length} live session${live.length === 1 ? "" : "s"}`
        }
      />

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
        {live.length === 0 ? (
          <EmptyState />
        ) : (
          <>
            <LiveSessionCards sessions={live} />
            <AggregateStats sessions={live} />
          </>
        )}
      </div>
    </>
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

function LiveSessionCards({ sessions }: { sessions: LiveSessionSummary[] }) {
  return (
    <section>
      <Heading>Live now</Heading>
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-10)",
        }}
      >
        {sessions.map((s) => (
          <SessionCard key={s.session_id} summary={s} />
        ))}
      </div>
    </section>
  );
}

function SessionCard({ summary }: { summary: LiveSessionSummary }) {
  const label = projectLabel(summary.cwd);
  return (
    <article
      style={{
        display: "grid",
        gridTemplateColumns: "8px 1fr auto",
        alignItems: "center",
        columnGap: "var(--sp-12)",
        padding: "var(--sp-12) var(--sp-16)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        background: "var(--bg)",
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
            marginTop: "2px",
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
          gap: "2px",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
          fontVariantNumeric: "tabular-nums",
        }}
      >
        <span>{formatElapsedMs(summary.idle_ms)}</span>
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
        ? "var(--accent-warn, orange)"
        : "var(--accent)";
  return (
    <span
      aria-hidden
      style={{
        display: "inline-block",
        width: "8px",
        height: "8px",
        borderRadius: "50%",
        background: palette[status],
        border: `1.5px solid ${ring}`,
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
          sub={`${countByStatus(sessions, "busy")} busy · ${countByStatus(sessions, "waiting")} waiting · ${countByStatus(sessions, "idle")} idle`}
        />
        <StatCard
          label="Model mix"
          value={mix[0] ?? "—"}
          sub={mix.slice(1).join(" · ") || "single family"}
        />
        <StatCard
          label="Spend / h"
          value="—"
          sub="pricing available; per-hour rate needs token burn rate (follow-on)"
        />
        <StatCard
          label="Cache-hit %"
          value="—"
          sub="exposed once usage deltas carry cache_read_tokens"
        />
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
        gap: "2px",
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

export function projectLabel(cwd: string): string {
  const trimmed = cwd.replace(/\/+$/, "");
  const idx = trimmed.lastIndexOf("/");
  const base = idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
  return base || cwd;
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
    const k = familyShort(s.model);
    counts.set(k, (counts.get(k) ?? 0) + 1);
  }
  return [...counts.entries()]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([k, n]) => `${k} ${n}`);
}
