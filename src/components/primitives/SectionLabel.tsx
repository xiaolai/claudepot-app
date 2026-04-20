import type { CSSProperties, ReactNode } from "react";

interface SectionLabelProps {
  children: ReactNode;
  /** Optional right-aligned content (a button, badge, etc). */
  right?: ReactNode;
  style?: CSSProperties;
}

/** Uppercase section divider label — "ACCOUNTS", "~/.claude", etc. */
export function SectionLabel({ children, right, style }: SectionLabelProps) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding: "var(--sp-12) var(--sp-14) var(--sp-6)",
        ...style,
      }}
    >
      <span className="mono-cap" style={{ color: "var(--fg-faint)" }}>
        {children}
      </span>
      {right}
    </div>
  );
}
