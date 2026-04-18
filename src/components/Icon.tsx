import { ICONS, type IconName } from "../icons";

export type { IconName };

interface IconProps {
  name: IconName;
  /** px. Defaults to 14 (matches body-adjacent icon size). */
  size?: number;
  className?: string;
  /** Accessible label. Omit for decorative icons (aria-hidden by default). */
  "aria-label"?: string;
  /** Optional title tooltip (shown on hover). */
  title?: string;
  /**
   * Accepted for drop-in compatibility with the previous Lucide API,
   * but has no effect — Nerd Font icons are font glyphs, not SVG, so
   * stroke width is baked into the glyph. Kept to avoid noisy diffs
   * at call sites.
   */
  strokeWidth?: number;
}

/**
 * Renders a Nerd Font glyph as inline text in the monospace UI body.
 * Color inherits from the surrounding element; size is set via
 * font-size. Decorative by default — pass an `aria-label` for icons
 * that carry semantic meaning on their own.
 */
export function Icon({
  name,
  size = 14,
  className = "",
  strokeWidth: _strokeWidth,
  title,
  "aria-label": ariaLabel,
}: IconProps) {
  const decorative = ariaLabel === undefined;
  return (
    <span
      className={`icon${className ? ` ${className}` : ""}`}
      style={{ fontSize: size, lineHeight: 1 }}
      aria-hidden={decorative || undefined}
      aria-label={ariaLabel}
      role={ariaLabel ? "img" : undefined}
      title={title}
    >
      {ICONS[name]}
    </span>
  );
}
