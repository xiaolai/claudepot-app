import type { CSSProperties } from "react";
import type { LiveStatus } from "../../types/activity";

/**
 * 8-px paper-mono dot mapping a live session's status (and optional
 * `errored` overlay) to a single color. Decorative by default — the
 * surrounding row carries the semantic label; this dot is a glance
 * signal. Pass `aria-label` when the dot stands alone.
 *
 * Color mapping matches `STATUS_TONE` in
 * `src/sections/sessions/components/liveStatusBits.tsx` so the dot
 * and the detail-header chip read as the same vocabulary:
 *   busy    → --accent  (terracotta, attention)
 *   waiting → --warn    (yellow, blocked)
 *   idle    → --fg-muted (neutral, fine)
 *   errored → --danger  (red, overrides base status)
 */

type Tone = "accent" | "warn" | "muted" | "danger";

const STATUS_DOT_TONE: Record<LiveStatus, Tone> = {
  busy: "accent",
  waiting: "warn",
  idle: "muted",
};

const TONE_COLOR: Record<Tone, string> = {
  accent: "var(--accent)",
  warn: "var(--warn)",
  muted: "var(--fg-muted)",
  danger: "var(--danger)",
};

interface Props {
  status: LiveStatus;
  /** When true, overrides `status` with a danger-toned dot. */
  errored?: boolean;
  /** Hover tooltip — usually the verb form ("Busy", "Waiting on
   *  Bash approval"). Mandatory in practice; primitives can't
   *  enforce this without warning at runtime, but downstream
   *  reviewers should reject a dot with no title. */
  title?: string;
  /** Accessible label. Omit when the surrounding row carries the
   *  label (the default). */
  "aria-label"?: string;
  /** Override size. Defaults to 8 px, matching paper-mono's
   *  inline-indicator scale. */
  size?: number;
  style?: CSSProperties;
}

export function LiveStatusDot({
  status,
  errored = false,
  title,
  "aria-label": ariaLabel,
  size = 8,
  style,
}: Props) {
  const tone: Tone = errored ? "danger" : STATUS_DOT_TONE[status];
  return (
    <span
      role={ariaLabel ? "img" : undefined}
      aria-label={ariaLabel}
      aria-hidden={ariaLabel ? undefined : true}
      title={title}
      style={{
        display: "inline-block",
        width: `${size}px`,
        height: `${size}px`,
        borderRadius: "50%",
        background: TONE_COLOR[tone],
        flexShrink: 0,
        ...style,
      }}
    />
  );
}
