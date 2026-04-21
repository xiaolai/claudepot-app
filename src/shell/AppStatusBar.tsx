import { Glyph } from "../components/primitives/Glyph";
import { useSessionLive } from "../hooks/useSessionLive";
import { NF } from "../icons";
import type { LiveSessionSummary } from "../types";

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
 */
export function AppStatusBar({ stats }: { stats: AppStatusBarStats }) {
  const live = useSessionLive();
  const liveSegment = formatLiveSegment(live);

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
    </div>
  );
}

/** Build the "● N live · Opus 2, Sonnet 1" segment. Returns null
 *  when no sessions are live so the segment is render-if-nonzero. */
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
 *  "OPUS 2, SON 1" in descending count order. Unknown models
 *  cluster under their raw id trimmed to 8 chars. */
export function modelMix(sessions: LiveSessionSummary[]): string[] {
  const counts = new Map<string, number>();
  for (const s of sessions) {
    const key = familyKey(s.model);
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  return [...counts.entries()]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([k, n]) => `${k} ${n}`);
}

function familyKey(model: string | null): string {
  if (!model) return "?";
  if (model.includes("opus")) return "OPUS";
  if (model.includes("sonnet")) return "SON";
  if (model.includes("haiku")) return "HAI";
  return model.length > 8 ? model.slice(0, 7) + "…" : model;
}
