import type { CSSProperties, ReactNode } from "react";
import { useDevMode } from "../../hooks/useDevMode";

/**
 * Mono-caps tag shown only when Developer mode is on. Use for
 * surfacing backend command names, internal enum values, or UUIDs
 * next to their human-facing label. Never rendered by default — this
 * is a diagnostic overlay, not a production surface.
 */
export function DevBadge({
  children,
  style,
}: {
  children: ReactNode;
  style?: CSSProperties;
}) {
  const [on] = useDevMode();
  if (!on) return null;
  return (
    <code
      style={{
        fontSize: "var(--fs-2xs)",
        color: "var(--fg-ghost)",
        background: "var(--bg)",
        border: "var(--bw-hair) solid var(--line)",
        padding: "var(--sp-px) var(--sp-5)",
        borderRadius: "var(--r-1)",
        fontFamily: "var(--font)",
        ...style,
      }}
    >
      {children}
    </code>
  );
}
