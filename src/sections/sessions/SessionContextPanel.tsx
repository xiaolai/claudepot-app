import { useEffect, useMemo, useState } from "react";
import { api } from "../../api";
import { Glyph } from "../../components/primitives/Glyph";
import { IconButton } from "../../components/primitives/IconButton";
import { NF } from "../../icons";
import type {
  ContextInjection,
  ContextStats,
} from "../../types";
import { ContextPhasePicker } from "./components/ContextPhasePicker";
import { ContextTotals, colorFor } from "./components/ContextTotals";
import { formatTokens } from "./format";

/**
 * Right-hand "Visible Context" panel — once rendered, lets the user
 * see which category of content is dominating the context window and
 * drill into individual injections.
 *
 * Fetches `ContextStats` on mount / filePath change. The Rust side
 * does the math; we just present it.
 */
export function SessionContextPanel({
  filePath,
  onClose,
  refreshSignal,
}: {
  filePath: string;
  onClose: () => void;
  refreshSignal: number;
}) {
  const [stats, setStats] = useState<ContextStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [phaseFilter, setPhaseFilter] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    api
      .sessionContextAttribution(filePath)
      .then((s) => {
        if (!cancelled) {
          setStats(s);
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
  }, [filePath, refreshSignal]);

  const filteredInjections = useMemo(() => {
    if (!stats) return [];
    if (phaseFilter == null) return stats.injections;
    return stats.injections.filter((i) => i.phase === phaseFilter);
  }, [stats, phaseFilter]);

  /**
   * Totals narrow with the phase picker so the bars, percentages, and
   * the "Visible tokens" number all reflect the same slice. Without
   * this the picker was a lie: it filtered the list underneath but
   * left whole-session totals up top.
   */
  const filteredTotals = useMemo(() => {
    if (!stats) return null;
    if (phaseFilter == null) return stats.totals;
    const t: typeof stats.totals = {
      claude_md: 0,
      mentioned_file: 0,
      tool_output: 0,
      thinking_text: 0,
      team_coordination: 0,
      user_message: 0,
    };
    for (const inj of filteredInjections) {
      switch (inj.category) {
        case "claude-md":
          t.claude_md += inj.tokens;
          break;
        case "mentioned-file":
          t.mentioned_file += inj.tokens;
          break;
        case "tool-output":
          t.tool_output += inj.tokens;
          break;
        case "thinking-text":
          t.thinking_text += inj.tokens;
          break;
        case "team-coordination":
          t.team_coordination += inj.tokens;
          break;
        case "user-message":
          t.user_message += inj.tokens;
          break;
      }
    }
    return t;
  }, [stats, phaseFilter, filteredInjections]);

  return (
    <aside
      data-testid="session-context-panel"
      aria-label="Visible context"
      style={{
        width: 360,
        borderLeft: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-raised)",
        display: "flex",
        flexDirection: "column",
        flexShrink: 0,
      }}
    >
      <header
        style={{
          padding: "var(--sp-14) var(--sp-18)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
        }}
      >
        <Glyph g={NF.layers} color="var(--fg-muted)" />
        <h3
          style={{
            margin: 0,
            fontSize: "var(--fs-sm)",
            fontWeight: 600,
            color: "var(--fg)",
            flex: 1,
          }}
        >
          Visible context
        </h3>
        <IconButton
          glyph={NF.x}
          onClick={onClose}
          title="Close panel"
          aria-label="Close visible context panel"
        />
      </header>

      <div
        style={{
          flex: 1,
          overflow: "auto",
          padding: "var(--sp-14) var(--sp-18)",
        }}
      >
        {loading && <LoadingLine text="Computing context…" />}
        {error && <ErrorLine text={error} />}
        {stats && filteredTotals && (
          <>
            <ContextTotals
              totals={filteredTotals}
              // The backend only gives us a whole-session
              // reported-total; when the user filters to a single
              // phase, showing that number alongside the phase's
              // own bars was misleading. Hide it for phase view.
              reportedTotal={
                phaseFilter == null ? stats.reported_total_tokens : null
              }
              phaseLabel={phaseFilter}
            />
            <ContextPhasePicker
              stats={stats}
              value={phaseFilter}
              onChange={setPhaseFilter}
            />
            <InjectionList injections={filteredInjections} />
          </>
        )}
      </div>
    </aside>
  );
}


function InjectionList({ injections }: { injections: ContextInjection[] }) {
  if (injections.length === 0) {
    return (
      <div
        style={{
          fontSize: "var(--fs-xs)",
          color: "var(--fg-ghost)",
          fontStyle: "italic",
        }}
      >
        No injections for this phase.
      </div>
    );
  }
  // Sort by tokens descending — biggest consumers first.
  const sorted = [...injections].sort((a, b) => b.tokens - a.tokens);
  return (
    <section>
      <div
        style={{
          fontSize: "var(--fs-3xs)",
          color: "var(--fg-faint)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
          marginBottom: "var(--sp-6)",
        }}
      >
        Top injections
      </div>
      <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
        {sorted.slice(0, 50).map((i, idx) => (
          <li
            key={`${i.event_index}-${idx}`}
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-8)",
              padding: "var(--sp-4) 0",
              borderBottom: "var(--bw-hair) solid var(--line)",
              fontSize: "var(--fs-xs)",
            }}
          >
            <span
              aria-hidden
              style={{
                width: 6,
                height: 6,
                borderRadius: "50%",
                background: colorFor(i.category),
                flexShrink: 0,
              }}
            />
            <span
              style={{
                flex: 1,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                color: "var(--fg)",
              }}
              title={i.label}
            >
              {i.label}
            </span>
            <span className="mono" style={{ color: "var(--fg-muted)" }}>
              {formatTokens(i.tokens)}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}

function LoadingLine({ text }: { text: string }) {
  return (
    <div style={{ color: "var(--fg-muted)", fontSize: "var(--fs-sm)" }}>
      {text}
    </div>
  );
}

function ErrorLine({ text }: { text: string }) {
  return (
    <div style={{ color: "var(--warn)", fontSize: "var(--fs-sm)" }}>
      Couldn't load context: {text}
    </div>
  );
}

