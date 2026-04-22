import type { ReactNode } from "react";

/**
 * Shared state cells for the session-detail viewer. Both the loading
 * pane and the empty state center their child message; LoadingPane
 * additionally fills available height so the spinner doesn't wedge
 * against the header.
 */

export function LoadingPane({ children }: { children: ReactNode }) {
  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: "var(--sp-8)",
        padding: "var(--sp-48)",
        color: "var(--fg-muted)",
        fontSize: "var(--fs-sm)",
      }}
    >
      {children}
    </div>
  );
}

export function EmptyState({ children }: { children: ReactNode }) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: "var(--sp-8)",
        padding: "var(--sp-48)",
        color: "var(--fg-muted)",
        fontSize: "var(--fs-sm)",
      }}
    >
      {children}
    </div>
  );
}
