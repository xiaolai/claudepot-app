import type { ContextCategory, ContextStats } from "../../../types";
import { formatTokens } from "../format";

/**
 * Stacked-bar breakdown of the visible context window by content
 * category. Reads `ContextStats["totals"]` (already computed by the
 * Rust side) and renders one row per category with a percent bar.
 *
 * `reportedTotal` is the model's own count for the whole session;
 * shown when no phase filter is active. When a phase IS selected,
 * `phaseLabel` (a 0-based phase number) replaces the total because
 * the per-phase stats don't carry a session-wide reported count.
 */
export function ContextTotals({
  totals,
  reportedTotal,
  phaseLabel,
}: {
  totals: ContextStats["totals"];
  reportedTotal: number | null;
  phaseLabel: number | null;
}) {
  const t = totals;
  const total =
    t.claude_md +
    t.mentioned_file +
    t.tool_output +
    t.thinking_text +
    t.team_coordination +
    t.user_message;
  const rows: { key: ContextCategory; label: string; value: number }[] = [
    { key: "claude-md", label: "CLAUDE.md", value: t.claude_md },
    { key: "mentioned-file", label: "Mentioned files", value: t.mentioned_file },
    { key: "tool-output", label: "Tool output", value: t.tool_output },
    { key: "thinking-text", label: "Thinking/text", value: t.thinking_text },
    {
      key: "team-coordination",
      label: "Team coord.",
      value: t.team_coordination,
    },
    { key: "user-message", label: "User messages", value: t.user_message },
  ];

  return (
    <section style={{ marginBottom: "var(--sp-18)" }}>
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          marginBottom: "var(--sp-10)",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
        }}
      >
        <span>Visible</span>
        <span className="mono">{formatTokens(total)} tok</span>
      </div>
      {rows.map((row) => {
        const pct = total > 0 ? (row.value / total) * 100 : 0;
        return (
          <div
            key={row.key}
            data-testid={`category-${row.key}`}
            style={{ marginBottom: "var(--sp-6)" }}
          >
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                fontSize: "var(--fs-xs)",
                color: "var(--fg)",
                marginBottom: 2,
              }}
            >
              <span>{row.label}</span>
              <span className="mono" style={{ color: "var(--fg-muted)" }}>
                {formatTokens(row.value)} · {pct.toFixed(1)}%
              </span>
            </div>
            <div
              style={{
                height: 4,
                background: "var(--bg-sunken)",
                borderRadius: "var(--r-1)",
                overflow: "hidden",
              }}
              aria-hidden
            >
              <div
                style={{
                  width: `${Math.max(pct, row.value > 0 ? 1 : 0)}%`,
                  height: "100%",
                  background: colorFor(row.key),
                }}
              />
            </div>
          </div>
        );
      })}
      <div
        style={{
          marginTop: "var(--sp-10)",
          fontSize: "var(--fs-3xs)",
          color: "var(--fg-faint)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
        }}
      >
        {reportedTotal != null
          ? `Model reported ${reportedTotal.toLocaleString()} total`
          : `Phase #${phaseLabel} (session total hidden)`}
      </div>
    </section>
  );
}

/**
 * Stable category → color map shared with the injection list. Lives
 * here because Totals is its primary user; the injection list imports
 * it back so a single palette change updates both surfaces.
 */
export function colorFor(cat: ContextCategory): string {
  switch (cat) {
    case "claude-md":
      return "var(--accent)";
    case "mentioned-file":
      return "var(--ok)";
    case "tool-output":
      return "var(--info, var(--fg-muted))";
    case "thinking-text":
      return "var(--fg-muted)";
    case "team-coordination":
      return "var(--warn)";
    case "user-message":
      return "var(--fg)";
  }
}
