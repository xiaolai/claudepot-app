import { useEffect } from "react";
import { Glyph } from "../components/primitives/Glyph";
import { useSessionLive } from "../hooks/useSessionLive";
import { useAppState } from "../providers/AppStateProvider";
import { NF } from "../icons";
import type { LiveSessionSummary } from "../types";

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
  /** Git branch name or similar. Undefined hides the branch segment. */
  branch?: string;
  /** Total projects. `null` hides the segment. */
  projects: number | null;
  /** Total sessions. `null` hides the segment. */
  sessions: number | null;
  /** Formatted monthly token count (already grouped). `null` hides. */
  tokens?: string | null;
  /** Active model label. */
  model?: string;
}

/**
 * Bottom 24px chrome — status dots and counts. All stats are
 * optional; if you pass `null` the segment is dropped so we never
 * render "0 projects · 0 sessions".
 *
 * Also hosts the dismissed-toast echo: when a transient notification
 * finishes, its text lingers here for `TOAST_ECHO_MS` so the user can
 * re-read what scrolled by. The echo is suppressed while a toast is
 * still on screen (one signal per surface — the live toast IS the
 * signal) and clears itself when the window elapses.
 */
export function AppStatusBar({ stats }: { stats: AppStatusBarStats }) {
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

  const segments: (string | null)[] = [
    liveSegment,
    stats.projects != null && stats.projects > 0
      ? `${stats.projects} project${stats.projects === 1 ? "" : "s"}`
      : null,
    stats.sessions != null && stats.sessions > 0
      ? `${stats.sessions} session${stats.sessions === 1 ? "" : "s"}`
      : null,
    stats.tokens ? `${stats.tokens} tokens this month` : null,
  ];
  const visible = segments.filter(Boolean) as string[];

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
      {stats.branch && (
        <>
          <span
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-6)",
            }}
          >
            <Glyph g={NF.branch} style={{ fontSize: "var(--fs-2xs)" }} />
            {stats.branch}
          </span>
          {visible.length > 0 && <span>·</span>}
        </>
      )}

      {visible.map((seg, i) => (
        <span
          key={seg}
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--sp-6)",
          }}
        >
          {i > 0 && <span style={{ marginRight: "var(--sp-10)" }}>·</span>}
          {seg}
        </span>
      ))}

      <span style={{ flex: 1 }} />

      {/* Model name is reference info, not a notification — subtle
          text, no accent color, no glyph. Uppercase + letter-spacing
          from the parent makes the string feel louder than its tone,
          so we drop those for the model segment and render it in
          --fg-ghost to sit quietly. */}
      {stats.model && (
        <span
          style={{
            color: "var(--fg-ghost)",
            textTransform: "none",
            letterSpacing: "var(--ls-normal)",
          }}
        >
          {stats.model}
        </span>
      )}

      {/* Toast echo — absolutely centered over the bar so it doesn't
          jostle the existing flex layout. Re-keyed on `at` so each new
          dismissal restarts the fade animation cleanly. The error tone
          carries a 2px left rule like the live toast does, which keeps
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
