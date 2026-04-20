import type { CSSProperties } from "react";
import type { NfIcon } from "../../icons";

interface GlyphProps {
  /** Lucide icon component (e.g. `NF.user`). */
  g: NfIcon;
  /**
   * Render size. Accepts a CSS length string (`"var(--sp-32)"`) or a
   * raw px number. Defaults to `1em` so the icon picks up the
   * surrounding font-size.
   */
  size?: number | string;
  /** Override color. Defaults to `currentColor`. */
  color?: CSSProperties["color"];
  /** Override stroke width. Lucide's default is 2; we use 1.75 to
   * match paper-mono's lighter register. */
  strokeWidth?: number;
  className?: string;
  style?: CSSProperties;
  /** Accessible label. Omit for decorative icons (aria-hidden). */
  "aria-label"?: string;
  title?: string;
}

/**
 * Thin wrapper around a Lucide icon component. Enforces paper-mono's
 * neutral stroke weight and inherits the surrounding font color by
 * default. The inline-block wrapper keeps vertical alignment
 * consistent with adjacent text (Lucide's SVG is centered on the
 * text baseline only when the parent sets `line-height: 1`).
 */
export function Glyph({
  g: Icon,
  size,
  color,
  strokeWidth = 1.75,
  className,
  style,
  "aria-label": ariaLabel,
  title,
}: GlyphProps) {
  const decorative = ariaLabel === undefined;
  const sizeStr =
    size == null
      ? "1em"
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
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        width: sizeStr,
        height: sizeStr,
        color,
        flexShrink: 0,
        verticalAlign: "var(--glyph-baseline-shift, -0.1em)",
        ...style,
      }}
    >
      <Icon
        size="100%"
        strokeWidth={strokeWidth}
        absoluteStrokeWidth={false}
        aria-hidden="true"
      />
    </span>
  );
}
