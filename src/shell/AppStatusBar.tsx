import { Glyph } from "../components/primitives/Glyph";
import { NF } from "../icons";

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
  const segments: (string | null)[] = [
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
          text, no accent color, no glyph. */}
      {stats.model && (
        <span style={{ color: "var(--fg-faint)" }}>{stats.model}</span>
      )}
    </div>
  );
}
