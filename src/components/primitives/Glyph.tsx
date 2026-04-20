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
 * Mono-rendered Nerd Font icon. By default the glyph is rendered at
 * `1.2em` — NF Mono's visible ink sits in the upper half of the em
 * cell, so matching surrounding font-size verbatim makes icons read
 * visibly smaller than the text cap-height next to them. The 20%
 * bump brings glyph ink to roughly the same visual weight as body
 * caps. Pass `size` to override (explicit CSS length or raw px).
 * Color inherits.
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
  // fontSize controls the glyph's actual rendered cell height.
  // When no size is passed we scale up to 1.2em so the icon ink
  // matches the visual weight of surrounding text; explicit sizes
  // (string or number) win.
  const fontSize =
    size == null
      ? "1.2em"
      : typeof size === "number"
        ? `${size}px`
        : size;
  // Width is 1em of the element's own (now-scaled) font-size. This
  // keeps a fixed aspect-ratio monospace cell (so list icon columns
  // still line up) without re-widening by an extra 20%.
  return (
    <span
      aria-hidden={decorative || undefined}
      aria-label={ariaLabel}
      role={ariaLabel ? "img" : undefined}
      title={title}
      className={className}
      style={{
        fontFamily: "var(--font)",
        fontSize,
        color,
        display: "inline-block",
        width: "1em",
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
