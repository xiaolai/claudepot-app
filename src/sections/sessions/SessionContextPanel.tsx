import { useEffect, useMemo, useState } from "react";
import { api } from "../../api";
import { Glyph } from "../../components/primitives/Glyph";
import { IconButton } from "../../components/primitives/IconButton";
import { NF } from "../../icons";
import type {
  ContextCategory,
  ContextInjection,
  ContextStats,
} from "../../types";
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
        {stats && (
          <>
            <Totals stats={stats} />
            <PhasePicker
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

function Totals({ stats }: { stats: ContextStats }) {
  const t = stats.totals;
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
        Model reported {stats.reported_total_tokens.toLocaleString()} total
      </div>
    </section>
  );
}

function PhasePicker({
  stats,
  value,
  onChange,
}: {
  stats: ContextStats;
  value: number | null;
  onChange: (v: number | null) => void;
}) {
  if (stats.phases.length <= 1) return null;
  return (
    <section style={{ marginBottom: "var(--sp-18)" }}>
      <div
        style={{
          fontSize: "var(--fs-3xs)",
          color: "var(--fg-faint)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
          marginBottom: "var(--sp-6)",
        }}
      >
        Phase
      </div>
      <div style={{ display: "flex", flexWrap: "wrap", gap: "var(--sp-4)" }}>
        <PhaseButton
          active={value == null}
          onClick={() => onChange(null)}
          label="All"
        />
        {stats.phases.map((p) => (
          <PhaseButton
            key={p.phase_number}
            active={value === p.phase_number}
            onClick={() => onChange(p.phase_number)}
            label={`#${p.phase_number}`}
          />
        ))}
      </div>
    </section>
  );
}

function PhaseButton({
  active,
  onClick,
  label,
}: {
  active: boolean;
  onClick: () => void;
  label: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        padding: "var(--sp-2) var(--sp-8)",
        fontSize: "var(--fs-xs)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-1)",
        background: active ? "var(--accent-soft)" : "transparent",
        color: active ? "var(--accent-ink)" : "var(--fg-muted)",
        cursor: "pointer",
      }}
    >
      {label}
    </button>
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

function colorFor(cat: ContextCategory): string {
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
