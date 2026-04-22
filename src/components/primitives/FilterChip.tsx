import type { CSSProperties, KeyboardEvent, ReactNode } from "react";
import { useState } from "react";
import type { NfIcon } from "../../icons";
import { Glyph } from "./Glyph";

interface FilterChipProps {
  /** Optional leading glyph. */
  glyph?: NfIcon;
  /** Current pressed state. Chips are controlled — parent owns state. */
  active: boolean;
  /** Click/Enter/Space fires this. */
  onToggle: () => void;
  /** Chip label. */
  children: ReactNode;
  /** Optional right-side count badge. Only rendered if > 0 (render-if-nonzero). */
  count?: number;
  /** Native title for hover tooltips. Keep short; prefer explicit affordances. */
  title?: string;
  disabled?: boolean;
  /** Optional style overrides (e.g. `flex: 0 0 auto`). */
  style?: CSSProperties;
  /** `aria-label` when the children aren't a readable phrase. */
  "aria-label"?: string;
}

/**
 * Interactive filter chip for paper-mono lists. Binary-state button:
 *   · inactive → hairline outline, muted ink
 *   · active   → accent background + border, solid ink
 *
 * Chips are stateless (`active` + `onToggle`). The parent combines them
 * into single-select or multi-select groups — FilterChip itself does
 * not care. See design rules: color never carries meaning alone,
 * focus-visible ring is always shown.
 */
export function FilterChip({
  glyph,
  active,
  onToggle,
  children,
  count,
  title,
  disabled = false,
  style,
  ...aria
}: FilterChipProps) {
  const [hover, setHover] = useState(false);
  const [focused, setFocused] = useState(false);

  const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (disabled) return;
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onToggle();
    }
  };

  const bg = active
    ? "var(--accent-soft)"
    : hover
      ? "var(--bg-raised)"
      : "var(--bg-sunken)";
  const fg = active ? "var(--accent-ink)" : "var(--fg-muted)";
  const bd = active
    ? "var(--accent-border)"
    : focused
      ? "var(--accent-border)"
      : "var(--line)";

  return (
    <button
      type="button"
      role="switch"
      aria-pressed={active}
      aria-checked={active}
      aria-label={aria["aria-label"]}
      title={title}
      disabled={disabled}
      onClick={() => !disabled && onToggle()}
      onKeyDown={onKeyDown}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-6)",
        height: "var(--sp-24)",
        padding: "0 var(--sp-10)",
        fontSize: "var(--fs-xs)",
        fontWeight: 500,
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
        color: disabled ? "var(--fg-faint)" : fg,
        background: bg,
        border: `var(--bw-hair) solid ${bd}`,
        borderRadius: "var(--r-1)",
        cursor: disabled ? "not-allowed" : "pointer",
        opacity: disabled ? 0.6 : 1,
        transition:
          "background-color var(--dur-fast) var(--ease-linear), border-color var(--dur-fast) var(--ease-linear)",
        whiteSpace: "nowrap",
        outlineOffset: 2,
        ...style,
      }}
    >
      {glyph && (
        <Glyph
          g={glyph}
          style={{ fontSize: "var(--fs-2xs)", color: "currentColor" }}
        />
      )}
      <span>{children}</span>
      {count !== undefined && count > 0 && (
        <span
          aria-hidden="true"
          style={{
            fontVariantNumeric: "tabular-nums",
            fontSize: "var(--fs-2xs)",
            color: "currentColor",
            opacity: 0.75,
          }}
        >
          {count}
        </span>
      )}
    </button>
  );
}
