import type { ReactNode } from "react";

/**
 * Keyboard-shortcut hint. Use inside buttons, palette hints, and
 * menu labels. Never interactive.
 */
export function Kbd({ children }: { children: ReactNode }) {
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        minWidth: "var(--kbd-min-w)",
        height: "var(--kbd-h)",
        padding: "0 var(--sp-4)",
        fontSize: "var(--fs-2xs)",
        fontWeight: 500,
        color: "var(--fg-muted)",
        background: "var(--bg-sunken)",
        border: "var(--bw-hair) solid var(--line)",
        borderBottom: "var(--bw-kbd) solid var(--line-strong)",
        borderRadius: "var(--r-1)",
        fontFamily: "var(--font)",
      }}
    >
      {children}
    </span>
  );
}
