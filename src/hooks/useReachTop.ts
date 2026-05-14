import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Fire `onReach` whenever the observed element enters the viewport.
 * Used by the session-detail viewer to page in older events when the
 * user scrolls to the top of the list — it's pagination, not
 * animation, so an IntersectionObserver is the right primitive
 * (cheap, no scroll listeners on the hot path).
 *
 * Returns a **callback ref** to attach to the sentinel element
 * (`<div ref={useReachTop(...)} />`). A callback ref is what makes
 * this correct: React invokes it with the node on mount and `null`
 * on unmount, so the observer attaches exactly when the sentinel
 * enters/leaves the DOM. Reading a plain `ref.current` at render
 * time would miss the mount — `.current` is populated *after*
 * render and doesn't trigger one, so the observer could attach late
 * or never.
 *
 * `enabled` lets the caller suspend observation while a fetch is
 * in-flight or the cursor is at the head of the data, so the
 * observer doesn't fire repeatedly into a no-op.
 */
export function useReachTop(
  enabled: boolean,
  onReach: () => void,
): (node: HTMLElement | null) => void {
  const [el, setEl] = useState<HTMLElement | null>(null);

  // `onReach` is typically an inline lambda — keep it in a ref so
  // the observer is created once per (el, enabled) change, not torn
  // down and rebuilt on every consumer render.
  const onReachRef = useRef(onReach);
  useEffect(() => {
    onReachRef.current = onReach;
  });

  useEffect(() => {
    if (!el || !enabled) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) onReachRef.current();
      },
      { threshold: 0 },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [el, enabled]);

  return useCallback((node: HTMLElement | null) => setEl(node), []);
}
