import type { ReactNode } from "react";

/**
 * Centered empty-state cell used both for "no sessions on disk" and
 * "no sessions match this filter". Pure layout — callers compose the
 * actual message from primitives.
 */
export function EmptyRow({ children }: { children: ReactNode }) {
  return (
    <div
      style={{
        padding: "var(--sp-60)",
        textAlign: "center",
        color: "var(--fg-faint)",
        fontSize: "var(--fs-sm)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-6)",
        alignItems: "center",
      }}
    >
      {children}
    </div>
  );
}
