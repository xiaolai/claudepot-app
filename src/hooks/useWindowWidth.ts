import { useEffect, useState } from "react";

/**
 * Live window inner width. Subscribes to `resize` so consumers can
 * flip between compact / spacious layouts without a full observer
 * dance. The sidebar is fixed at `--sidebar-width`, so this value
 * is a reliable proxy for the content column's available width.
 */
export function useWindowWidth(): number {
  const [w, setW] = useState<number>(() =>
    typeof window === "undefined" ? 1200 : window.innerWidth,
  );
  useEffect(() => {
    const onResize = () => setW(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  return w;
}

/**
 * Threshold below which the Accounts / Projects header collapses
 * ghost actions into icon-only buttons so the row stays inside the
 * content pane.
 *
 * Derived from the Accounts worst case: 260 sidebar + 64 padding +
 * 16 gap + 198 HealthChips + ~232 compact action strip ≈ 770 min,
 * with headroom. 1000 leaves a clean jump from labeled → iconic.
 */
export const COMPACT_HEADER_BREAKPOINT = 1000;

export function useCompactHeader(): boolean {
  return useWindowWidth() < COMPACT_HEADER_BREAKPOINT;
}
