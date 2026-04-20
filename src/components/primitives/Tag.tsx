import type { CSSProperties, ReactNode } from "react";
import { Glyph } from "./Glyph";

export type TagTone =
  | "neutral"
  | "accent"
  | "ok"
  | "warn"
  | "danger"
  | "ghost";

interface TagProps {
  tone?: TagTone;
  glyph?: string;
  children: ReactNode;
  style?: CSSProperties;
  title?: string;
}

const TONES: Record<TagTone, { bg: string; fg: string; bd: string }> = {
  neutral: {
    bg: "var(--bg-sunken)",
    fg: "var(--fg-muted)",
    bd: "var(--line)",
  },
  accent: {
    bg: "var(--accent-soft)",
    fg: "var(--accent-ink)",
    bd: "var(--accent-border)",
  },
  ok: {
    bg: "transparent",
    fg: "var(--ok)",
    bd: "color-mix(in oklch, var(--ok) 40%, transparent)",
  },
  warn: {
    bg: "transparent",
    fg: "var(--warn)",
    bd: "color-mix(in oklch, var(--warn) 40%, transparent)",
  },
  danger: {
    bg: "transparent",
    fg: "var(--danger)",
    bd: "color-mix(in oklch, var(--danger) 40%, transparent)",
  },
  ghost: {
    bg: "transparent",
    fg: "var(--fg-faint)",
    bd: "transparent",
  },
};

/**
 * Small uppercase pill for metadata. Paired with text, never color
 * alone. See `.claude/rules/accessibility.md` — color is never the
 * only signal.
 */
export function Tag({
  tone = "neutral",
  glyph,
  children,
  style,
  title,
}: TagProps) {
  const t = TONES[tone];
  return (
    <span
      title={title}
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-4)",
        height: "var(--sp-18)",
        padding: "0 var(--sp-6)",
        fontSize: "var(--fs-xs)",
        fontWeight: 500,
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
        background: t.bg,
        color: t.fg,
        border: `var(--bw-hair) solid ${t.bd}`,
        borderRadius: "var(--r-1)",
        whiteSpace: "nowrap",
        ...style,
      }}
    >
      {glyph && (
        <Glyph g={glyph} style={{ fontSize: "var(--fs-2xs)" }} />
      )}
      {children}
    </span>
  );
}
