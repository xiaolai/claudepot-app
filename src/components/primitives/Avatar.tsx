import { Glyph } from "./Glyph";

export type AvatarSize = "2xs" | "xs" | "sm" | "md" | "lg" | "xl";

interface AvatarProps {
  /** Display name — initial is derived from the first character. */
  name?: string | null;
  /** Background color. When omitted, renders a neutral outline. */
  color?: string | null;
  /**
   * Semantic size variant mapping to the `--avatar-*` tokens.
   * Numeric is a legacy escape hatch; new code should use variants.
   */
  size?: AvatarSize | number;
  /** NF codepoint to render instead of the initial. */
  glyph?: string;
}

const SIZE_TOKEN: Record<
  AvatarSize,
  { dim: string; initial: string }
> = {
  "2xs": { dim: "var(--avatar-2xs)", initial: "var(--avatar-initial-2xs)" },
  xs:    { dim: "var(--avatar-xs)",  initial: "var(--avatar-initial-xs)"  },
  sm:    { dim: "var(--avatar-sm)",  initial: "var(--avatar-initial-sm)"  },
  md:    { dim: "var(--avatar-md)",  initial: "var(--avatar-initial-md)"  },
  lg:    { dim: "var(--avatar-lg)",  initial: "var(--avatar-initial-lg)"  },
  xl:    { dim: "var(--avatar-xl)",  initial: "var(--avatar-initial-xl)"  },
};

/** Numeric prop fallback — matches the scale defined in tokens.css. */
const NUMERIC_TO_VARIANT: Record<number, AvatarSize> = {
  14: "2xs",
  18: "xs",
  20: "sm",
  22: "md",
  28: "lg",
  36: "xl",
};

function resolveAvatar(
  size: AvatarProps["size"],
): { dim: string; initial: string } {
  if (typeof size === "string") return SIZE_TOKEN[size];
  if (typeof size === "number") {
    const variant = NUMERIC_TO_VARIANT[size];
    if (variant) return SIZE_TOKEN[variant];
    // Off-scale numeric — escape hatch. Emits raw px; tokens.css
    // should be extended so the variant name exists instead.
    return { dim: `${size}px`, initial: `${size * 0.5}px` };
  }
  return SIZE_TOKEN.md;
}

/**
 * Colored rounded square with the name's first initial. Used
 * everywhere an account appears: cards, list rows, swap-target
 * switchers, command palette.
 *
 * `size` is a semantic variant that maps to the `--avatar-*` tokens
 * for both the square dimension and the derived initial font size.
 */
export function Avatar({
  name,
  color,
  size = "md",
  glyph,
}: AvatarProps) {
  const initial = name?.[0]?.toUpperCase() || "?";
  const { dim, initial: fs } = resolveAvatar(size);
  // CSS grid + place-items + line-height:1 gives us pixel-exact
  // box centering. The inner wrapper then applies
  // --avatar-optical-nudge to shift the ink DOWN — JetBrains Mono
  // NF caps sit visibly high in the em box, so geometric centre is
  // not optical centre. Nudging only the inner content keeps the
  // border + background perfectly aligned with surrounding layout.
  return (
    <span
      style={{
        width: dim,
        height: dim,
        display: "grid",
        placeItems: "center",
        borderRadius: "var(--r-2)",
        background: color || "var(--bg-sunken)",
        color: color ? "var(--on-color)" : "var(--fg-muted)",
        fontSize: fs,
        lineHeight: "var(--lh-flat)",
        fontWeight: 600,
        border: color ? "none" : "var(--bw-hair) solid var(--line)",
        flexShrink: 0,
        letterSpacing: 0,
        overflow: "hidden",
      }}
    >
      <span
        style={{
          display: "inline-block",
          transform: "translateY(var(--avatar-optical-nudge))",
        }}
      >
        {glyph ? <Glyph g={glyph} /> : initial}
      </span>
    </span>
  );
}

/**
 * Deterministic accent color derived from a string (e.g., an email).
 * Used when the account record doesn't carry an explicit color. The
 * derivation lightness and chroma come from `--avatar-derived-l/c`
 * tokens so the palette stays cohesive across many accounts.
 */
export function avatarColorFor(seed: string): string {
  let h = 0;
  for (let i = 0; i < seed.length; i++) {
    h = (h * 31 + seed.charCodeAt(i)) >>> 0;
  }
  const hue = h % 360;
  return `oklch(var(--avatar-derived-l) var(--avatar-derived-c) ${hue})`;
}
