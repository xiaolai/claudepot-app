import { useId, type ReactNode } from "react";

/**
 * One label + hint + switch row in a Settings pane.
 *
 * Extracted while adding the fast-mode and per-session-opt-in
 * switches, which would otherwise have been the third and fourth
 * hand-copied instance of this markup. The two older copies
 * (`ExtendedThinkingToggle`, `CompanionArtifactToggle`) still carry
 * their own — migrating them is a separate, purely mechanical change.
 *
 * Design contract, per `rules/design.md`:
 *   - `role="switch"` + `aria-checked` + `aria-describedby` on the hint.
 *   - A locked switch states its reason **inline** (the hint turns
 *     warn-coloured), never in a tooltip.
 *   - The hint carries the meaning; colour never carries it alone.
 */
export function SettingToggleRow({
  label,
  hint,
  hintTone = "muted",
  checked,
  disabled,
  onChange,
  indent = false,
  children,
}: {
  label: string;
  hint: ReactNode;
  /** `warn` for "this switch is locked / inert, and here's why". */
  hintTone?: "muted" | "warn";
  checked: boolean;
  disabled?: boolean;
  onChange: (next: boolean) => void;
  /** Indent a switch that only makes sense under the row above it. */
  indent?: boolean;
  /** Extra content under the hint (e.g. a cost note). */
  children?: ReactNode;
}) {
  const hintId = useId();
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "var(--settings-label-col) 1fr",
        gap: "var(--sp-16)",
        alignItems: "start",
        padding: "var(--sp-8) 0",
        paddingLeft: indent ? "var(--sp-16)" : undefined,
        borderBottom: "var(--bw-hair) solid var(--line)",
      }}
    >
      <div>
        <div style={{ fontSize: "var(--fs-sm)", color: "var(--fg)" }}>
          {label}
        </div>
        <div
          id={hintId}
          style={{
            fontSize: "var(--fs-xs)",
            color: hintTone === "warn" ? "var(--warn)" : "var(--fg-faint)",
            marginTop: "var(--sp-3)",
            lineHeight: "var(--lh-body)",
          }}
        >
          {hint}
        </div>
        {children}
      </div>
      <div style={{ display: "flex", alignItems: "center" }}>
        <button
          type="button"
          role="switch"
          aria-checked={checked}
          aria-label={label}
          aria-describedby={hintId}
          aria-disabled={disabled || undefined}
          disabled={disabled}
          onClick={disabled ? undefined : () => onChange(!checked)}
          className="pm-focus"
          style={{
            width: "var(--toggle-track-w)",
            height: "var(--toggle-track-h)",
            borderRadius: "var(--r-pill)",
            background: checked ? "var(--accent)" : "var(--bg-active)",
            border: `var(--bw-hair) solid ${
              checked ? "var(--accent)" : "var(--line-strong)"
            }`,
            position: "relative",
            opacity: disabled ? "var(--opacity-disabled)" : 1,
            transition: "background var(--dur-base) var(--ease-linear)",
          }}
        >
          <span
            aria-hidden
            style={{
              position: "absolute",
              top: "var(--toggle-thumb-off)",
              left: checked
                ? "var(--toggle-thumb-on)"
                : "var(--toggle-thumb-off)",
              width: "var(--toggle-thumb-d)",
              height: "var(--toggle-thumb-d)",
              borderRadius: "var(--r-pill)",
              background: "var(--bg-raised)",
              boxShadow: "var(--shadow-thumb)",
              transition: "left var(--dur-base) var(--ease-linear)",
            }}
          />
        </button>
      </div>
    </div>
  );
}
