import { forwardRef, useCallback, useRef, useState } from "react";
import { SectionLabel } from "../components/primitives/SectionLabel";
import { useSessionLive } from "../hooks/useSessionLive";
import type { LiveSessionSummary, LiveStatus } from "../types";

/**
 * Live Activity strip — inserted between the primary nav and the
 * `~/.claude` tree. Render-if-nonzero: the entire strip
 * (heading + rows) is suppressed when no sessions are active,
 * honoring the paper-mono rule that zero-value surfaces don't ship.
 *
 * Each row shows: status dot · project basename · model · current
 * tool · elapsed. Click opens the corresponding session via the
 * `onOpenSession` callback; the parent decides whether to route to
 * the static Sessions browser (M1) or the Live pane (M2+).
 *
 * Keyboard: `j` and `k` cycle focus up and down within the strip
 * when it's mounted and the user isn't editing an input. Enter and
 * Space on the focused row invoke the same click handler.
 */

interface Props {
  /** Invoked when the user activates a row. The parent chooses the
   * routing (Sessions deep-link in M1; SessionLivePane in M2). */
  onOpenSession: (session: LiveSessionSummary) => void;
}

export function SidebarLiveStrip({ onOpenSession }: Props) {
  const sessions = useSessionLive();
  // `focusedIdx` drives the j/k navigation state; the value itself
  // is consumed inside the keydown handler via the functional
  // setter. useRef instead of useState would avoid a re-render per
  // cycle, but the re-render is one row and the hook rule buys us
  // consistency with the rest of the app's state model.
  const [focusedIdx, setFocusedIdx] = useState<number>(-1);
  void focusedIdx;
  const rowRefs = useRef<(HTMLButtonElement | null)[]>([]);

  const count = sessions.length;

  // Keyboard navigation. Local onKeyDown on the listbox — no
  // window-level listener — so `j` / `k` pressed anywhere outside
  // the strip (browse-mode screen readers, prose in form fields,
  // mini-editors in other surfaces) is never intercepted. Only
  // active while a row inside the listbox owns focus.
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (count === 0) return;
      if (e.key !== "j" && e.key !== "k") return;
      e.preventDefault();
      setFocusedIdx((prev) => {
        const base = prev < 0 ? 0 : prev;
        const next =
          e.key === "j"
            ? Math.min(count - 1, base + 1)
            : Math.max(0, base - 1);
        rowRefs.current[next]?.focus();
        return next;
      });
    },
    [count],
  );

  const openByIndex = useCallback(
    (idx: number) => {
      const s = sessions[idx];
      if (s) onOpenSession(s);
    },
    [sessions, onOpenSession],
  );

  // Render-if-nonzero — strip disappears entirely when empty.
  if (count === 0) return null;

  return (
    <>
      <SectionLabel>
        <span style={{ display: "flex", justifyContent: "space-between", width: "100%" }}>
          <span style={{ color: "var(--fg-ghost)" }}>LIVE</span>
          <span style={{ color: "var(--fg-faint)" }}>{count}</span>
        </span>
      </SectionLabel>
      <div
        role="listbox"
        aria-label="Live Claude sessions"
        onKeyDown={handleKeyDown}
        style={{ padding: "0 var(--sp-8)", marginBottom: "var(--sp-8)" }}
      >
        {sessions.map((s, i) => (
          <LiveRow
            key={s.session_id}
            ref={(el) => {
              rowRefs.current[i] = el;
            }}
            summary={s}
            onClick={() => openByIndex(i)}
            onFocus={() => setFocusedIdx(i)}
          />
        ))}
      </div>
    </>
  );
}

// ── Row ───────────────────────────────────────────────────────────

interface RowProps {
  summary: LiveSessionSummary;
  onClick: () => void;
  onFocus: () => void;
}

const LiveRow = forwardRef<HTMLButtonElement, RowProps>(function LiveRow(
  { summary, onClick, onFocus },
  ref,
) {
  const label = projectLabel(summary.cwd);
  const elapsed = formatElapsed(summary.idle_ms);
  const model = shortenModel(summary.model);
  const statusTitle = buildStatusTitle(summary);

  return (
    <button
      ref={ref}
      type="button"
      role="option"
      aria-selected={false}
      aria-label={`${label}: ${statusTitle}`}
      onClick={onClick}
      onFocus={onFocus}
      className="pm-focus"
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        width: "100%",
        padding: "var(--sp-6) var(--sp-8)",
        border: "none",
        background: "transparent",
        textAlign: "left",
        cursor: "pointer",
        borderRadius: "var(--r-1)",
        color: "var(--fg)",
        fontSize: "var(--fs-xs)",
      }}
      onMouseOver={(e) => {
        e.currentTarget.style.background = "var(--bg-raised)";
      }}
      onMouseOut={(e) => {
        e.currentTarget.style.background = "transparent";
      }}
    >
      <StatusDot status={summary.status} errored={summary.errored} />
      <span
        style={{
          flex: 1,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
          fontWeight: 500,
        }}
      >
        {label}
      </span>
      {model ? (
        <span
          style={{
            color: "var(--fg-faint)",
            textTransform: "uppercase",
            letterSpacing: "var(--ls-wide)",
            fontSize: "var(--fs-2xs)",
          }}
        >
          {model}
        </span>
      ) : null}
      <span style={{ color: "var(--fg-muted)", fontVariantNumeric: "tabular-nums" }}>
        {summary.status === "waiting" && summary.waiting_for
          ? summary.waiting_for
          : summary.current_action
            ? truncate(summary.current_action, 22)
            : summary.status}
      </span>
      <span
        style={{
          color: "var(--fg-ghost)",
          fontVariantNumeric: "tabular-nums",
          minWidth: "3ch",
          textAlign: "right",
        }}
      >
        {elapsed}
      </span>
    </button>
  );
});

interface DotProps {
  status: LiveStatus;
  errored: boolean;
}

function StatusDot({ status, errored }: DotProps) {
  const palette = STATUS_DOT[status];
  const ring = errored ? "var(--warn)" : palette.outline;
  return (
    <span
      aria-hidden
      style={{
        width: "var(--sp-8)",
        height: "var(--sp-8)",
        borderRadius: "50%",
        background: palette.color,
        border: `1.5px solid ${ring}`,
        flexShrink: 0,
      }}
    />
  );
}

const STATUS_DOT: Record<
  LiveStatus,
  { color: string; outline: string }
> = {
  busy: { color: "var(--accent)", outline: "var(--accent)" },
  waiting: { color: "transparent", outline: "var(--accent)" },
  idle: { color: "transparent", outline: "var(--fg-faint)" },
};

// ── Pure helpers (unit-testable via export) ────────────────────────

/** Last path segment, falling back to the full cwd if empty. */
export function projectLabel(cwd: string): string {
  const trimmed = cwd.replace(/\/+$/, "");
  const idx = trimmed.lastIndexOf("/");
  const base = idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
  return base || cwd;
}

/** Render millisecond durations as `Mm:SS` or `<Xs` for short. */
export function formatElapsed(ms: number): string {
  if (ms < 1000) return "—";
  if (ms < 10_000) return `${Math.floor(ms / 1000)}s`;
  const totalSec = Math.floor(ms / 1000);
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  if (m < 60) return `${m}:${String(s).padStart(2, "0")}`;
  const h = Math.floor(m / 60);
  return `${h}h${m % 60}m`;
}

/** Model ids from CC can be dated (`claude-haiku-4-5-20251001`);
 *  show a short 3-letter marker that fits next to the project
 *  label. Raw strings outside the known family pass through. */
export function shortenModel(model: string | null): string {
  if (!model) return "";
  if (model.includes("opus")) return "OPUS";
  if (model.includes("sonnet")) return "SON";
  if (model.includes("haiku")) return "HAI";
  return model.length > 10 ? model.slice(0, 8) + "…" : model;
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  return s.slice(0, max - 1) + "…";
}

function buildStatusTitle(s: LiveSessionSummary): string {
  const parts: string[] = [s.status];
  if (s.waiting_for && s.status === "waiting") parts.push(s.waiting_for);
  if (s.current_action) parts.push(s.current_action);
  if (s.errored) parts.push("errored");
  if (s.stuck) parts.push("stuck");
  return parts.join(" · ");
}
