import { useEffect, useRef, type RefObject } from "react";

/**
 * Dismiss a popover on outside click or Escape.
 *
 * Extracted from the byte-identical effects in SidebarTargetSwitcher,
 * RunningOpsChip, and PreviewHeader (audit 2026-07 F14). While `open`
 * is true, a `mousedown` outside `rootRef`'s subtree or an Escape
 * keydown calls `onDismiss`.
 *
 * The 0ms timeout defers wiring the mousedown listener past the click
 * that opened the popover so it doesn't re-close on the same event
 * tick. `onDismiss` is read through a ref so callers can pass an
 * inline arrow without re-subscribing the listeners every render.
 */
export function usePopoverDismiss(
  rootRef: RefObject<HTMLElement | null>,
  open: boolean,
  onDismiss: () => void,
): void {
  const dismissRef = useRef(onDismiss);
  dismissRef.current = onDismiss;

  useEffect(() => {
    if (!open) return;
    const onDocClick = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) {
        dismissRef.current();
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") dismissRef.current();
    };
    const t = window.setTimeout(() => {
      document.addEventListener("mousedown", onDocClick);
    }, 0);
    window.addEventListener("keydown", onKey);
    return () => {
      window.clearTimeout(t);
      document.removeEventListener("mousedown", onDocClick);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, rootRef]);
}
