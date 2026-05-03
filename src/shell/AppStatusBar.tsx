import { useEffect } from "react";
import { useSessionLive } from "../hooks/useSessionLive";
import { useAppState } from "../providers/AppStateProvider";
import { RunningOpsChip } from "../components/RunningOpsChip";
import { PendingJournalsChip } from "../components/PendingJournalsChip";
import { ServiceStatusDot } from "./ServiceStatusDot";
import type {
  LiveSessionSummary,
  PendingJournalsSummary,
  RunningOpInfo,
} from "../types";

/** How long the dismissed-toast echo lives in the status bar before
 *  fading out. Long enough for the user to re-read what just scrolled
 *  by, short enough that the echo doesn't outlast its relevance.
 *
 *  Lives in JS rather than CSS because two consumers need the same
 *  number: the keyframe animation duration AND the `setTimeout` that
 *  unmounts the segment. CSS variables don't compose cleanly into
 *  setTimeout, so the JS constant is the single source and the
 *  animation duration interpolates from it. */
const TOAST_ECHO_MS = 6000;

export interface AppStatusBarStats {
  /** Total projects. `null` hides the segment. */
  projects: number | null;
  /** Total sessions. `null` hides the segment. */
  sessions: number | null;
}

export interface AppStatusBarProps {
  stats: AppStatusBarStats;
  /** In-flight long-running ops; renders the running-ops chip when nonzero. */
  runningOps?: RunningOpInfo[];
  /** Re-open progress modal for an op clicked in the chip popover. */
  onReopenOp?: (opId: string) => void;
  /** Pending rename-journal counts; renders the pending chip when actionable. */
  pendingSummary?: PendingJournalsSummary | null;
  /** Click target for the pending chip — typically jumps to Projects → Repair. */
  onOpenRepair?: () => void;
  /** Click target for the live-sessions segment — typically jumps to Activity. */
  onOpenLive?: () => void;
}

/**
 * Bottom tokens.s[6] chrome — the single ambient-state surface for the app.
 *
 * Layout, left → right:
 *   1. Live sessions segment (`● 3 live · OPUS 2, SON 1`) — text/link
 *      when `onOpenLive` is wired, plain text otherwise.
 *   2. Aggregate counts — `N projects · N sessions` — passed in via `stats`.
 *      Each segment is `null`-elidable so we never render `0 projects`.
 *   3. Right cluster of action chips: `[● N op]` running-ops chip +
 *      `[⚠ N pending]` pending-journals chip. Each chip resolves to
 *      a real UI destination per design.md "render-if-nonzero" rule.
 *
 * Center floats the dismissed-toast echo over the existing flex
 * layout so it doesn't jostle the segment positions.
 *
 * Why no `branch` or `model` fields: Claudepot has no app-wide
 * concept of a "current project" (it's a switcher, not an editor),
 * and CC selects model per-session — both would be misleading.
 */
export function AppStatusBar({
  stats,
  runningOps,
  onReopenOp,
  pendingSummary,
  onOpenRepair,
  onOpenLive,
}: AppStatusBarProps) {
  const live = useSessionLive();
  const liveSegment = formatLiveSegment(live);
  const { lastDismissed, clearLastDismissed, toasts } = useAppState();

  // Echo only shows when no toast is currently visible — otherwise the
  // user would see the same message twice (once as a toast, once as
  // the echo). When a new toast pushes in, the echo is suppressed
  // immediately and resumes on the next dismissal cycle.
  const echoVisible = !!lastDismissed && toasts.length === 0;

  // Schedule the auto-clear. Re-keyed on `at` so each new dismissal
  // gets a full window. If a toast pushes mid-window, `echoVisible`
  // flips to false but the timer keeps running — when the toast
  // dismisses we just record a fresh `at` and the echo restarts.
  useEffect(() => {
    if (!lastDismissed) return;
    const remaining =
      lastDismissed.at + TOAST_ECHO_MS - Date.now();
    if (remaining <= 0) {
      clearLastDismissed();
      return;
    }
    const t = setTimeout(clearLastDismissed, remaining);
    return () => clearTimeout(t);
  }, [lastDismissed, clearLastDismissed]);

  // Each count segment carries a `title` so the bar's terse glyph-y
  // text reveals plain English on hover, and an `aria-label` so
  // screen readers announce the same. Native title is fine here:
  // the bar is for ambient context, not primary action.
  const countSegments: { text: string; title: string }[] = [];
  if (stats.projects != null && stats.projects > 0) {
    countSegments.push({
      text: `${stats.projects} project${stats.projects === 1 ? "" : "s"}`,
      title: `${stats.projects} CC project${stats.projects === 1 ? "" : "s"} indexed in ~/.claude/projects`,
    });
  }
  if (stats.sessions != null && stats.sessions > 0) {
    countSegments.push({
      text: `${stats.sessions} session${stats.sessions === 1 ? "" : "s"}`,
      title: `${stats.sessions} session transcript${stats.sessions === 1 ? "" : "s"} on disk`,
    });
  }

  const hasRunningOps =
    !!runningOps && runningOps.some((o) => o.status === "running");
  const hasPending =
    !!pendingSummary && pendingSummary.pending + pendingSummary.stale > 0;
  // ServiceStatusDot self-decides visibility from preferences. The
  // right cluster is always rendered as a wrapper so the dot — which
  // sits at the cluster's far-left — keeps a stable position even
  // when no running ops or pending journals exist.
  const hasRightCluster = true;

  return (
    <div
      style={{
        position: "relative",
        height: "var(--statusbar-height)",
        flexShrink: 0,
        borderTop: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-sunken)",
        display: "flex",
        alignItems: "center",
        padding: "0 var(--sp-12)",
        gap: "var(--sp-16)",
        fontSize: "var(--fs-2xs)",
        color: "var(--fg-faint)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
      }}
    >
      {liveSegment && (
        <LiveSegment text={liveSegment} onClick={onOpenLive} />
      )}

      {countSegments.map((seg, i) => (
        <span
          key={seg.text}
          title={seg.title}
          aria-label={seg.title}
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--sp-6)",
          }}
        >
          {(liveSegment || i > 0) && (
            <span aria-hidden style={{ marginRight: "var(--sp-10)" }}>·</span>
          )}
          {seg.text}
        </span>
      ))}

      <span style={{ flex: 1 }} />

      {hasRightCluster && (
        <span
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: "var(--sp-6)",
            // Cancel the parent's wide letter-spacing + uppercase for
            // the chip cluster — chips own their own typography (see
            // statusbar-chips.css). The bar's wide-tracking is for
            // text segments only.
            textTransform: "none",
            letterSpacing: "var(--ls-normal)",
          }}
        >
          {hasRunningOps && onReopenOp && (
            <RunningOpsChip
              ops={runningOps ?? []}
              onReopen={onReopenOp}
            />
          )}
          {hasPending && onOpenRepair && (
            <PendingJournalsChip
              summary={pendingSummary ?? null}
              onOpen={onOpenRepair}
            />
          )}
          <ServiceStatusDot />
        </span>
      )}

      {/* Toast echo — absolutely centered over the bar so it doesn't
          jostle the existing flex layout. Re-keyed on `at` so each new
          dismissal restarts the fade animation cleanly. The error tone
          carries a tokens.sp[2] left rule like the live toast does, which keeps
          the visual link without saturating the bar. */}
      {echoVisible && lastDismissed && (
        <div
          key={lastDismissed.at}
          aria-hidden
          style={{
            position: "absolute",
            left: "50%",
            top: "50%",
            transform: "translate(-50%, -50%)",
            maxWidth: "var(--toast-echo-max-width)",
            padding:
              lastDismissed.kind === "error"
                ? "0 var(--sp-8) 0 calc(var(--sp-8) - var(--bw-hair))"
                : "0 var(--sp-8)",
            borderLeft:
              lastDismissed.kind === "error"
                ? "var(--bw-strong) solid var(--danger)"
                : "none",
            color: "var(--fg-muted)",
            textTransform: "none",
            letterSpacing: "var(--ls-normal)",
            fontSize: "var(--fs-2xs)",
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
            pointerEvents: "none",
            animation: `statusbar-echo-fade ${TOAST_ECHO_MS}ms ease forwards`,
          }}
        >
          {lastDismissed.text}
        </div>
      )}

      {/* Echo fade keyframes. Stays opaque for ~85% of the window then
          eases out — a slow fade reads as "passing memory" rather
          than a flash that vanishes. Inline so the style ships with
          the only consumer; living in tokens.css would orphan a rule
          no one else references. */}
      <style>{`
        @keyframes statusbar-echo-fade {
          0%   { opacity: 0; }
          5%   { opacity: 0.9; }
          80%  { opacity: 0.9; }
          100% { opacity: 0; }
        }
        @media (prefers-reduced-motion: reduce) {
          @keyframes statusbar-echo-fade {
            0%, 100% { opacity: 0.9; }
          }
        }
      `}</style>
    </div>
  );
}

/** Live-sessions segment. Renders as a button-shaped link when a
 *  click handler is wired (jumps to the Activity section's live
 *  filter); otherwise stays as plain text. Either way the bar's
 *  uppercase + wide-tracking is preserved so it reads as one of the
 *  ambient segments rather than a chip. */
function LiveSegment({
  text,
  onClick,
}: {
  text: string;
  onClick?: () => void;
}) {
  // The live segment text reads as opaque jargon to a new user
  // ("● 3 live · OPUS 2, SON 1"). Tooltips spell out what the dot
  // means and that the right-hand cluster groups by model family.
  const tip =
    "Sessions Claude Code is currently writing to. Suffix groups by model family.";
  if (!onClick) {
    return (
      <span
        title={tip}
        aria-label={tip}
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-6)",
        }}
      >
        {text}
      </span>
    );
  }
  return (
    <button
      type="button"
      onClick={onClick}
      title={`${tip} Click to open Activities → Live.`}
      aria-label={`${tip} Click to open Activities → Live.`}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-6)",
        background: "transparent",
        border: 0,
        padding: 0,
        margin: 0,
        height: "auto",
        font: "inherit",
        fontSize: "inherit",
        color: "inherit",
        letterSpacing: "inherit",
        textTransform: "inherit",
        cursor: "pointer",
      }}
    >
      {text}
    </button>
  );
}

/** Build the "● N live · OPUS 2, SON 1" segment. Returns null when
 *  no sessions are live so the segment is render-if-nonzero. When
 *  every session has an unknown model, renders just "● N live" — the
 *  "?" family rendered as a letterform read as an error indicator.
 *  The live count already captures the total; the mix is supplemental. */
export function formatLiveSegment(
  sessions: LiveSessionSummary[],
): string | null {
  if (sessions.length === 0) return null;
  const mix = modelMix(sessions);
  if (mix.length === 0) {
    return `● ${sessions.length} live`;
  }
  return `● ${sessions.length} live · ${mix.join(", ")}`;
}

/** Group live sessions by 3-letter model family and format as
 *  "OPUS 2, SON 1" in descending count order. Sessions whose model is
 *  `null` (no assistant turn yet) are omitted — the status bar doesn't
 *  label them as "? N" because a solitary question mark reads as an
 *  error; the live-count segment still counts them. Unrecognised
 *  non-null models cluster under their raw id trimmed to 8 chars. */
export function modelMix(sessions: LiveSessionSummary[]): string[] {
  const counts = new Map<string, number>();
  for (const s of sessions) {
    const key = familyKey(s.model);
    if (key == null) continue;
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  return [...counts.entries()]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([k, n]) => `${k} ${n}`);
}

function familyKey(model: string | null): string | null {
  if (!model) return null;
  if (model.includes("opus")) return "OPUS";
  if (model.includes("sonnet")) return "SON";
  if (model.includes("haiku")) return "HAI";
  return model.length > 8 ? model.slice(0, 7) + "…" : model;
}
