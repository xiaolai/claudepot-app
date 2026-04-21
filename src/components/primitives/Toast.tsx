import type { CSSProperties } from "react";

/**
 * Transient, bottom-centered, click-to-dismiss notification. Paper-
 * mono register — hairline border, raised background, soft shadow.
 *
 * Parent owns the message (state). Passing `null`/`undefined` renders
 * nothing, so callers just write `<Toast message={err} onDismiss={...}
 * />` — no conditional wrapper needed.
 *
 * `role` defaults to `"status"` (polite live region, for confirmations
 * and non-critical feedback). Pass `tone="error"` to upgrade to
 * `"alert"` (assertive live region) — screen readers will interrupt
 * to announce the message immediately.
 *
 * This primitive replaces three ad-hoc inline-style copies that had
 * drifted apart AND fought a leftover `.inline-toast` class rule,
 * producing a viewport-tall rectangle instead of a small toast.
 * Keep new callers on this primitive.
 */
export interface ToastProps {
  /** Message to show. `null`/`undefined`/empty string → nothing renders. */
  message: string | null | undefined;
  /** Fires on click. Callers typically setToast(null). */
  onDismiss: () => void;
  /** `"status"` (default) → polite. `"error"` → assertive alert. */
  tone?: "status" | "error";
  /** Override-hook for edge cases (e.g., placing inside a modal). */
  style?: CSSProperties;
}

export function Toast({
  message,
  onDismiss,
  tone = "status",
  style,
}: ToastProps) {
  if (!message) return null;
  return (
    <div
      role={tone === "error" ? "alert" : "status"}
      onClick={onDismiss}
      style={{
        position: "fixed",
        bottom: "var(--sp-40)",
        left: "50%",
        transform: "translateX(-50%)",
        padding: "var(--sp-10) var(--sp-16)",
        background: "var(--bg-raised)",
        border: "var(--bw-hair) solid var(--line-strong)",
        borderRadius: "var(--r-2)",
        fontSize: "var(--fs-sm)",
        color: "var(--fg)",
        boxShadow: "var(--shadow-md)",
        cursor: "pointer",
        maxWidth: "var(--toast-max-width)",
        zIndex: "var(--z-toast)" as unknown as number,
        ...style,
      }}
    >
      {message}
    </div>
  );
}
