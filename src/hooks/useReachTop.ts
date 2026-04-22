import { useEffect } from "react";

/**
 * Fire `onReach` whenever `el` enters the viewport. Used by the
 * session-detail viewer to page in older events when the user scrolls
 * to the top of the list — it's pagination, not animation, so an
 * IntersectionObserver is the right primitive (cheap, no scroll
 * listeners on the hot path).
 *
 * `enabled` lets the caller suspend observation while a fetch is
 * in-flight or the cursor is at the head of the data, so the
 * observer doesn't fire repeatedly into a no-op.
 */
export function useReachTop(
  el: HTMLElement | null,
  enabled: boolean,
  onReach: () => void,
) {
  useEffect(() => {
    if (!el || !enabled) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) onReach();
      },
      { threshold: 0 },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [el, enabled, onReach]);
}
