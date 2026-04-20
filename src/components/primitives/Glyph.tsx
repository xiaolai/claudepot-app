import type { CSSProperties } from "react";

interface GlyphProps {
  /** NF codepoint (string). Use `import { NF } from '@/icons'`. */
  g: string;
  /**
   * Optional override size. Accepts a CSS length string
   * (`"var(--sp-32)"` — preferred) or a raw px number (escape
   * hatch). When omitted, the glyph inherits the surrounding
   * element's font-size.
   */
  size?: number | string;
  /** Override color. Defaults to `currentColor`. */
  color?: CSSProperties["color"];
  className?: string;
  style?: CSSProperties;
  /** Accessible label. Omit for decorative glyphs (aria-hidden). */
  "aria-label"?: string;
  title?: string;
}

/**
 * Mono-rendered Nerd Font icon. Size follows font-size by default;
 * pass `size` to force a specific px height. Color inherits. The NF
 * Mono build forces every icon glyph into the monospace cell, so the
 * explicit `width` here keeps lists and columns aligned.
 */
export function Glyph({
  g,
  size,
  color,
  className,
  style,
  "aria-label": ariaLabel,
  title,
}: GlyphProps) {
  const decorative = ariaLabel === undefined;
  const dim =
    size == null
      ? "1.2em"
      : typeof size === "number"
        ? `${size}px`
        : size;
  return (
    <span
      aria-hidden={decorative || undefined}
      aria-label={ariaLabel}
      role={ariaLabel ? "img" : undefined}
      title={title}
      className={className}
      style={{
        fontFamily: "var(--font)",
        fontSize: size,
        color,
        display: "inline-block",
        width: dim,
        textAlign: "center",
        lineHeight: "var(--lh-flat)",
        fontFeatureSettings: "normal",
        verticalAlign: "var(--glyph-baseline-shift)",
        flexShrink: 0,
        ...style,
      }}
    >
      {g}
    </span>
  );
}
