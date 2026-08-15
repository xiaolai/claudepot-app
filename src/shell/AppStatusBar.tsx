import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { i18n } from "../lib/i18n";
import { useSessionLive } from "../hooks/useSessionLive";
import { useAppState } from "../providers/AppStateProvider";
import { RunningOpsChip } from "../components/RunningOpsChip";
import { PendingJournalsChip } from "../components/PendingJournalsChip";
import { ServiceStatusDot } from "./ServiceStatusDot";
import { Glyph } from "../components/primitives/Glyph";
import { NF } from "../icons";
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
  /** Current sidebar-collapse state — drives the leftmost toggle's glyph
   *  and aria-label. Omitting the prop hides the toggle entirely. */
  sidebarCollapsed?: boolean;
  /** Toggle the sidebar's collapsed/expanded state. */
  onToggleSidebar?: () => void;
}

/**
 * Bottom 24px chrome — the single ambient-state surface for the app.
 *
 * ("tokens.s[6]" stood here in place of "24px" since 0610b1e0 — an old
 * codemod rewrote a literal inside prose. It was the only instance.)
 *
 * Layout, left → right:
 *   1. Live sessions segment (`● 3 live`) — a count, as plain text.
 *      Ambient state, never a control: the sidebar strip directly above
 *      already opens the live list.
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
  sidebarCollapsed,
  onToggleSidebar,
}: AppStatusBarProps) {
  const { t } = useTranslation("shell");
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
      text: t("statusbar.projects", { count: stats.projects }),
      title: t("statusbar.projectsTitle", { count: stats.projects }),
    });
  }
  if (stats.sessions != null && stats.sessions > 0) {
    countSegments.push({
      text: t("statusbar.sessions", { count: stats.sessions }),
      title: t("statusbar.sessionsTitle", { count: stats.sessions }),
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
      {onToggleSidebar && (
        // Sidebar toggle anchored at the bar's far-left. Duplicates
        // the chevron in the sidebar itself so the affordance is
        // reachable even when the sidebar is collapsed to a rail
        // and the user's pointer is nowhere near it. Tooltip and
        // aria-label flip with state; the ⌘\ hint stays so the
        // keyboard shortcut is discoverable from here too.
        <button
          type="button"
          onClick={onToggleSidebar}
          title={
            sidebarCollapsed
              ? t("sidebar.expandTitle")
              : t("sidebar.collapseTitle")
          }
          aria-label={
            sidebarCollapsed ? t("sidebar.expand") : t("sidebar.collapse")
          }
          aria-pressed={sidebarCollapsed === true}
          className="pm-focus"
          style={{
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            width: "var(--sp-20)",
            height: "var(--sp-20)",
            padding: 0,
            background: "transparent",
            border: "var(--bw-hair) solid transparent",
            borderRadius: "var(--r-1)",
            color: "var(--fg-muted)",
            cursor: "pointer",
            // Pull the button slightly left so it sits against the
            // bar's left edge inset, matching where a status-bar
            // platform glyph usually anchors.
            marginLeft: "calc(var(--sp-4) * -1)",
          }}
        >
          <Glyph
            g={sidebarCollapsed ? NF.sidebarOpen : NF.sidebarClose}
            style={{ fontSize: "var(--fs-xs)" }}
          />
        </button>
      )}

      {liveSegment && (
        <LiveSegment text={liveSegment} />
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
function LiveSegment({ text }: { text: string }) {
  // Ambient, not a control. It used to be a button that jumped to
  // Activities — but the sidebar strip directly above it already opens
  // the live list, and Activities independently remembers its last tab,
  // so clicking "live" could land on Cost.
  const { t } = useTranslation("shell");
  const tip = t("statusbar.liveTip");
  return (
    <span
      title={tip}
      aria-label={tip}
      style={{ display: "flex", alignItems: "center", gap: "var(--sp-6)" }}
    >
      {text}
    </span>
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
  // Count only. The model mix (`· OPUS 2, SON 1`) moved out because
  // five surfaces rendered the same `useSessionLive` data and none of
  // them said which was authoritative. The status bar's job is now the
  // glanceable number; the sidebar strip owns the list, and Activities
  // owns history and cost. The bar's own comment already conceded the
  // mix "reads as opaque jargon to a new user".
  //
  // Plain function, not a component — reads the global i18n instance
  // directly. The rendering component re-renders on language change
  // (its useTranslation subscription), re-invoking this.
  return i18n.t("shell:statusbar.live", { count: sessions.length });
}

