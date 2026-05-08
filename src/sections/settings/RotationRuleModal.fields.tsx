import type { CSSProperties, ReactNode } from "react";

/* Field + form-input primitives shared by the RotationRuleModal.
 * Extracted to keep the modal under the loc-guardian production-LOC
 * limit; kept nearby so callers don't go fishing for them. */

export function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint: string;
  children: ReactNode;
}) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-4)" }}>
      <span
        style={{
          fontSize: "var(--fs-xs)",
          fontWeight: 600,
          color: "var(--fg)",
          letterSpacing: "var(--ls-tight)",
        }}
      >
        {label}
      </span>
      {children}
      {hint && (
        <small style={{ color: "var(--fg-muted)", fontSize: "var(--fs-xs)" }}>
          {hint}
        </small>
      )}
    </div>
  );
}

export function CandidateChecklist({
  options,
  selected,
  onToggle,
}: {
  options: string[];
  selected: string[];
  onToggle: (email: string) => void;
}) {
  if (options.length === 0) {
    return (
      <small style={{ color: "var(--fg-faint)", fontSize: "var(--fs-xs)" }}>
        No accounts registered yet.
      </small>
    );
  }
  return (
    <ul
      style={{
        listStyle: "none",
        margin: 0,
        padding: "var(--sp-8)",
        background: "var(--bg-raised)",
        borderRadius: "var(--rad-sm)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-4)",
        maxHeight: "var(--candidate-list-max-h, var(--sp-160))",
        overflow: "auto",
      }}
    >
      {options.map((email) => (
        <li key={email}>
          <label
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-8)",
              fontSize: "var(--fs-sm)",
              cursor: "pointer",
            }}
          >
            <input
              type="checkbox"
              checked={selected.includes(email)}
              onChange={() => onToggle(email)}
            />
            {email}
          </label>
        </li>
      ))}
    </ul>
  );
}

export function ModeRadio({
  value,
  current,
  label,
  onChange,
}: {
  value: "confirm" | "auto";
  current: "confirm" | "auto";
  label: string;
  onChange: (v: "confirm" | "auto") => void;
}) {
  return (
    <label
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--sp-6)",
        fontSize: "var(--fs-sm)",
        cursor: "pointer",
      }}
    >
      <input
        type="radio"
        name="rotation-mode"
        value={value}
        checked={current === value}
        onChange={() => onChange(value)}
      />
      {label}
    </label>
  );
}

export function inputStyle(): CSSProperties {
  return {
    padding: "var(--sp-6) var(--sp-8)",
    border: "var(--bw-hair) solid var(--line)",
    borderRadius: "var(--rad-sm)",
    background: "var(--bg-raised)",
    color: "var(--fg)",
    fontFamily: "inherit",
    fontSize: "var(--fs-sm)",
  };
}

export function selectStyle(): CSSProperties {
  return inputStyle();
}

export function suggestId(existing: string[]): string {
  const base = "5h-near-cap";
  if (!existing.includes(base)) return base;
  let n = 2;
  while (existing.includes(`${base}-${n}`)) n += 1;
  return `${base}-${n}`;
}

export function clampPct(n: number): number {
  if (Number.isNaN(n)) return 1;
  return Math.max(1, Math.min(100, Math.round(n)));
}
