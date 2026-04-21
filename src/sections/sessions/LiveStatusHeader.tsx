import { useEffect, useMemo, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api } from "../../api";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import { useSessionLive } from "../../hooks/useSessionLive";
import type { LiveDelta, LiveSessionSummary } from "../../types";

/**
 * LiveStatusHeader — rendered above the historical `SessionDetail`
 * content when the selected session is currently live. Shows:
 *   * status chip (busy / waiting / idle) with overlay color for
 *     errored / stuck
 *   * model + waiting_for verb
 *   * current-action card with the task-summary text (or tool head-
 *     line fallback), already redacted on the Rust side
 *   * cumulative turn elapsed (animated via requestAnimationFrame)
 *
 * The component subscribes to both the aggregate bus (for snapshot
 * hydration) and the per-session `live::<sessionId>` delta channel
 * (for task-summary / status / overlay changes that beat the 500ms
 * aggregate republish).
 */

interface Props {
  sessionId: string;
}

export function LiveStatusHeader({ sessionId }: Props) {
  const aggregate = useSessionLive();
  const summary = useMemo(
    () => aggregate.find((s) => s.session_id === sessionId) ?? null,
    [aggregate, sessionId],
  );
  const [liveCurrentAction, setLiveCurrentAction] = useState<string | null>(
    null,
  );

  // Subscribe to the per-session detail channel so we get
  // TaskSummary deltas faster than the 500ms aggregate republish.
  // Handle every delta kind (not just TaskSummary + Ended) so a
  // live-header that missed intermediate aggregate emits stays in
  // sync. On `resync_required`, re-pull the session's snapshot to
  // reset local state; on `ended`, drop local overrides.
  //
  // Cleanup path calls session_live_unsubscribe so the backend
  // bridge task is cancelled when the pane unmounts — otherwise
  // a remount hits AlreadySubscribed.
  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | null = null;

    api
      .sessionLiveSubscribe(sessionId)
      .then(() => listen<LiveDelta>(`live::${sessionId}`, async (ev) => {
        if (cancelled) return;
        const d = ev.payload;
        if (d.resync_required) {
          // Pull the authoritative session snapshot and resync
          // local state from it before applying the delta payload.
          try {
            const snap = await api.sessionLiveSessionSnapshot(sessionId);
            if (!cancelled && snap) {
              setLiveCurrentAction(snap.current_action);
            }
          } catch {
            /* fall through */
          }
        }
        switch (d.kind) {
          case "task_summary_changed":
            setLiveCurrentAction(d.summary);
            break;
          case "ended":
            setLiveCurrentAction(null);
            break;
          case "status_changed":
          case "overlay_changed":
          case "model_changed":
            // Status / overlay / model transitions are reflected
            // through the aggregate bus at 500ms; we don't need
            // to maintain a second copy here. The aggregate hook
            // (useSessionLive) drives the chip row.
            break;
        }
      }))
      .then((fn) => {
        if (cancelled) {
          fn?.();
        } else {
          unlisten = fn ?? null;
        }
      })
      .catch(() => {
        // Already-subscribed or no-tauri env — aggregate view still
        // works because the aggregate republish at 500ms carries the
        // same info.
      });

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
      // Ask the backend to drop its forwarding task so a later
      // remount can re-subscribe cleanly.
      api.sessionLiveUnsubscribe(sessionId).catch(() => {
        /* best-effort */
      });
    };
  }, [sessionId]);

  // Nothing to render if this session isn't currently live.
  if (!summary) return null;

  return (
    <section
      aria-label="Live session status"
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-8)",
        padding: "var(--sp-12) var(--sp-16)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-raised)",
      }}
    >
      <StatusChipRow summary={summary} />
      <CurrentActionCard
        action={liveCurrentAction ?? summary.current_action}
        status={summary.status}
        waitingFor={summary.waiting_for}
      />
      {summary.errored || summary.stuck ? (
        <OverlayBanner errored={summary.errored} stuck={summary.stuck} />
      ) : null}
    </section>
  );
}

// ── Bits ──────────────────────────────────────────────────────────

function StatusChipRow({ summary }: { summary: LiveSessionSummary }) {
  const statusTone = STATUS_TONE[summary.status];
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        flexWrap: "wrap",
        gap: "var(--sp-10)",
        fontSize: "var(--fs-xs)",
        color: "var(--fg-muted)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
      }}
    >
      <Chip tone={statusTone}>{summary.status}</Chip>
      {summary.model ? <Chip tone="neutral">{summary.model}</Chip> : null}
      {summary.waiting_for ? (
        <Chip tone="neutral">{summary.waiting_for}</Chip>
      ) : null}
      <div style={{ flex: 1 }} />
      <ElapsedCounter idleMs={summary.idle_ms} />
    </div>
  );
}

function CurrentActionCard({
  action,
  status,
  waitingFor,
}: {
  action: string | null;
  status: LiveSessionSummary["status"];
  waitingFor: string | null;
}) {
  const body =
    action ??
    (status === "waiting" && waitingFor
      ? `waiting — ${waitingFor}`
      : status === "idle"
        ? "idle — awaiting your prompt"
        : "working…");
  return (
    <div
      style={{
        display: "flex",
        alignItems: "flex-start",
        gap: "var(--sp-10)",
        padding: "var(--sp-10) var(--sp-12)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        background: "var(--bg)",
      }}
    >
      <span
        aria-hidden
        style={{
          marginTop: "2px",
          color:
            status === "busy" ? "var(--accent)" : "var(--fg-faint)",
        }}
      >
        <Glyph g={NF.bolt} />
      </span>
      <div
        style={{
          flex: 1,
          fontSize: "var(--fs-sm)",
          lineHeight: 1.4,
          color: "var(--fg)",
        }}
      >
        {body}
      </div>
    </div>
  );
}

function OverlayBanner({ errored, stuck }: { errored: boolean; stuck: boolean }) {
  const messages: string[] = [];
  if (errored) messages.push("errors in the last minute");
  if (stuck) messages.push("tool call has been running > 10 min");
  return (
    <div
      role="alert"
      style={{
        padding: "var(--sp-6) var(--sp-10)",
        border: "var(--bw-hair) solid var(--accent-warn, orange)",
        borderRadius: "var(--r-1)",
        fontSize: "var(--fs-xs)",
        color: "var(--accent-warn, orange)",
        background: "var(--bg)",
      }}
    >
      {messages.join(" · ")}
    </div>
  );
}

function ElapsedCounter({ idleMs }: { idleMs: number }) {
  // Base on the timestamp the backend published, then locally
  // advance via rAF so the display updates every second without
  // requiring a backend tick. When the backend publishes a new
  // idle_ms the base resets.
  const [tickMs, setTickMs] = useState(0);
  useEffect(() => {
    setTickMs(0);
    const start = performance.now();
    let rafId: number | null = null;
    const tick = () => {
      setTickMs(performance.now() - start);
      rafId = requestAnimationFrame(tick);
    };
    rafId = requestAnimationFrame(tick);
    return () => {
      if (rafId !== null) cancelAnimationFrame(rafId);
    };
  }, [idleMs]);
  const totalSec = Math.floor((idleMs + tickMs) / 1000);
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  const text = m > 0 ? `${m}:${String(s).padStart(2, "0")}` : `${s}s`;
  return (
    <span
      style={{
        fontVariantNumeric: "tabular-nums",
        color: "var(--fg-muted)",
      }}
    >
      {text}
    </span>
  );
}

// ── Styling helpers ───────────────────────────────────────────────

type ChipTone = "accent" | "neutral" | "warn";

const STATUS_TONE: Record<LiveSessionSummary["status"], ChipTone> = {
  busy: "accent",
  waiting: "warn",
  idle: "neutral",
};

function Chip({ tone, children }: { tone: ChipTone; children: string }) {
  const palette: Record<ChipTone, { fg: string; border: string }> = {
    accent: { fg: "var(--accent)", border: "var(--accent)" },
    warn: {
      fg: "var(--accent-warn, orange)",
      border: "var(--accent-warn, orange)",
    },
    neutral: { fg: "var(--fg-muted)", border: "var(--line)" },
  };
  const p = palette[tone];
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        padding: "2px var(--sp-6)",
        border: `var(--bw-hair) solid ${p.border}`,
        borderRadius: "var(--r-1)",
        color: p.fg,
        fontSize: "var(--fs-xs)",
        fontWeight: 500,
        letterSpacing: "var(--ls-wide)",
      }}
    >
      {children}
    </span>
  );
}
