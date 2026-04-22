import type { ContextStats } from "../../../types";

/**
 * Compact phase selector for the context panel. Hidden when there's
 * only one phase (no choice to make). The "All" pill maps to a `null`
 * filter; numbered pills map to their phase number.
 */
export function ContextPhasePicker({
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
        background: active ? "var(--accent-soft)" : "transparent",
        color: active ? "var(--accent-ink)" : "var(--fg-muted)",
        border: `var(--bw-hair) solid ${active ? "var(--accent-border)" : "var(--line)"}`,
        borderRadius: "var(--r-1)",
        cursor: "pointer",
      }}
    >
      {label}
    </button>
  );
}
