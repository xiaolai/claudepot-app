import type { CSSProperties } from "react";
import { Glyph } from "./Glyph";
import { NF } from "../../icons";

interface BackAffordanceProps {
  /** Where this returns to, rendered as the link text. Use a noun the
   *  user recognizes from the section's chrome (e.g. "Artifacts" in
   *  Config, "Sessions" in Projects). */
  label: string;
  onClick: () => void;
  /** Override the hover/title text when the rendered label is too
   *  generic on its own (e.g. "Artifacts" → "Back to artifact list"). */
  title?: string;
  style?: CSSProperties;
}

/**
 * Single-tab-stop back link rendered as a chevron-left + uppercase
 * label. One focusable control for one action, matching the design's
 * accessibility floor (every interactive element keyboard-reachable;
 * one focus ring per signal).
 *
 * Use above a detail-pane title when the parent surface needs to
 * collapse back to its home/empty state (Config FilePreview /
 * EffectiveShell back to ConfigHomePane). For breadcrumb-style
 * navigation where individual segments map to distinct destinations
 * (e.g. SessionDetailHeader's "PROJECT › sessId"), make the segment
 * itself a button instead — that's not what this primitive is for.
 */
export function BackAffordance({
  label,
  onClick,
  title,
  style,
}: BackAffordanceProps) {
  const accessibleTitle = title ?? `Back to ${label.toLowerCase()}`;
  return (
    <button
      type="button"
      onClick={onClick}
      title={accessibleTitle}
      aria-label={accessibleTitle}
      className="pm-focus"
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-4)",
        background: "transparent",
        border: "none",
        padding: "var(--sp-2) var(--sp-4)",
        margin: `0 0 0 calc(-1 * var(--sp-4))`,
        borderRadius: "var(--r-1)",
        fontSize: "var(--fs-2xs)",
        fontFamily: "inherit",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
        color: "var(--fg-muted)",
        cursor: "pointer",
        ...style,
      }}
    >
      <Glyph g={NF.chevronL} style={{ fontSize: "var(--fs-xs)" }} />
      <span>{label}</span>
    </button>
  );
}
