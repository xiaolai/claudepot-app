import { useEffect, useRef, type CSSProperties } from "react";

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
 * Auto-dismiss: by default the toast calls `onDismiss` after
 * `durationMs` (10 s). Pass `durationMs={Infinity}` or `null` for a
 * sticky toast that only dismisses on click / ESC. The timer is
 * keyed on `message`, so pushing a new message resets the clock —
 * consecutive toasts each get a full duration.
 *
 * This primitive replaces three ad-hoc inline-style copies that had
 * drifted apart AND fought a leftover `.inline-toast` class rule,
 * producing a viewport-tall rectangle instead of a small toast.
 * Keep new callers on this primitive.
 */
export interface ToastProps {
  /** Message to show. `null`/`undefined`/empty string → nothing renders. */
  message: string | null | undefined;
  /** Fires on click AND when the auto-dismiss timer elapses.
   *  Callers typically setToast(null). */
  onDismiss: () => void;
  /** `"status"` (default) → polite. `"error"` → assertive alert. */
  tone?: "status" | "error";
  /** Auto-dismiss delay in ms. Default 10 000. Pass `Infinity` or
   *  `null` to disable auto-dismiss (sticky until click). */
  durationMs?: number | null;
  /** Override-hook for edge cases (e.g., placing inside a modal). */
  style?: CSSProperties;
}

export function Toast({
  message,
  onDismiss,
  tone = "status",
  durationMs = 10_000,
  style,
}: ToastProps) {
  // Auto-dismiss. Re-keyed on `message` so a fresh toast gets a fresh
  // clock; a parent rendering the same content twice in a row (unlikely
  // but legal) keeps the full window each time.
  //
  // The `onDismiss` callback is typically written as an inline arrow
  // (`() => setToast(null)`) which changes identity on every parent
  // render — including parents that re-render for unrelated reasons
  // while a toast is active. If we depended on `onDismiss` directly
  // the timer would reset on every re-render and never fire. Latch it
  // behind a ref so the effect only re-runs when message/duration
  // actually change.
  const onDismissRef = useRef(onDismiss);
  onDismissRef.current = onDismiss;
  useEffect(() => {
    if (!message) return;
    if (durationMs == null || !Number.isFinite(durationMs)) return;
    const t = setTimeout(() => {
      onDismissRef.current();
    }, durationMs);
    return () => clearTimeout(t);
  }, [message, durationMs]);

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
