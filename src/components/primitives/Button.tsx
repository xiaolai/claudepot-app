import { type CSSProperties, type ReactNode, useState } from "react";
import { Glyph } from "./Glyph";

export type ButtonVariant =
  | "solid"
  | "ghost"
  | "subtle"
  | "outline"
  | "accent";

export type ButtonSize = "sm" | "md" | "lg";

interface ButtonProps {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** NF codepoint (decorative leading icon). */
  glyph?: string;
  children?: ReactNode;
  onClick?: () => void;
  active?: boolean;
  disabled?: boolean;
  danger?: boolean;
  title?: string;
  type?: "button" | "submit";
  style?: CSSProperties;
  "aria-label"?: string;
  "aria-pressed"?: boolean;
  "aria-haspopup"?: boolean | "menu" | "listbox" | "tree" | "grid" | "dialog";
  "aria-expanded"?: boolean;
  autoFocus?: boolean;
}

const SIZES: Record<
  ButtonSize,
  { h: string; px: string; fs: string }
> = {
  sm: { h: "var(--btn-h-sm)", px: "var(--sp-8)",  fs: "var(--fs-xs)" },
  md: { h: "var(--btn-h-md)", px: "var(--sp-12)", fs: "var(--fs-sm)" },
  lg: { h: "var(--btn-h-lg)", px: "var(--sp-16)", fs: "var(--fs-base)" },
};

/**
 * Canonical button. One `solid` per view — that's the primary action.
 * Everything else is ghost / subtle / outline / accent. Apply `danger`
 * to shift color to --danger for destructive actions.
 *
 * For icon-only square buttons, use `IconButton` instead.
 */
export function Button({
  variant = "ghost",
  size = "md",
  glyph,
  children,
  onClick,
  active,
  disabled,
  danger,
  title,
  type = "button",
  style,
  autoFocus,
  ...aria
}: ButtonProps) {
  const [hover, setHover] = useState(false);
  const [press, setPress] = useState(false);

  const s = SIZES[size];
  const paint = variantPaint(variant, { hover, active, danger });

  return (
    <button
      type={type}
      title={title}
      disabled={disabled}
      autoFocus={autoFocus}
      onClick={disabled ? undefined : onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => {
        setHover(false);
        setPress(false);
      }}
      onMouseDown={() => setPress(true)}
      onMouseUp={() => setPress(false)}
      {...aria}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-8)",
        height: s.h,
        padding: `0 ${s.px}`,
        fontSize: s.fs,
        fontWeight: 500,
        borderRadius: "var(--r-2)",
        background: paint.bg,
        color: paint.color,
        border: paint.border,
        opacity: disabled ? "var(--opacity-disabled)" : 1,
        cursor: disabled ? "not-allowed" : "pointer",
        transform: press
          ? "translateY(var(--press-shift))"
          : "none",
        transition:
          "background var(--dur-fast) var(--ease-linear), color var(--dur-fast) var(--ease-linear)",
        whiteSpace: "nowrap",
        ...style,
      }}
    >
      {glyph && <Glyph g={glyph} />}
      {children}
    </button>
  );
}

function variantPaint(
  variant: ButtonVariant,
  flags: { hover: boolean; active?: boolean; danger?: boolean },
): { bg: string; color: string; border: string } {
  const { hover, active, danger } = flags;
  const fg = danger ? "var(--danger)" : undefined;

  switch (variant) {
    case "solid":
      return {
        bg: danger ? "var(--danger)" : "var(--accent)",
        color: "var(--on-color)",
        border: `var(--bw-hair) solid ${danger ? "var(--danger)" : "var(--accent)"}`,
      };
    case "subtle":
      return {
        bg: hover ? "var(--bg-hover)" : "var(--bg-sunken)",
        color: fg ?? "var(--fg)",
        border: "var(--bw-hair) solid var(--line)",
      };
    case "outline":
      return {
        bg: hover ? "var(--bg-hover)" : "transparent",
        color: fg ?? "var(--fg)",
        border: `var(--bw-hair) solid ${danger ? "var(--danger)" : "var(--line-strong)"}`,
      };
    case "accent":
      return {
        bg: hover ? "var(--accent-soft)" : "transparent",
        color: "var(--accent-ink)",
        border: "var(--bw-hair) solid var(--accent-border)",
      };
    case "ghost":
    default:
      return {
        bg: active
          ? "var(--bg-active)"
          : hover
            ? "var(--bg-hover)"
            : "transparent",
        color: fg ?? "var(--fg)",
        border: "var(--bw-hair) solid transparent",
      };
  }
}
