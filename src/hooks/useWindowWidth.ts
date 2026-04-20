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

/**
 * Threshold below which the Projects section collapses from a
 * master/detail split (table + fixed 420 px aside) to a single-pane
 * stack — i.e. the selected project's detail replaces the table,
 * with a Back button to return.
 *
 * Budget:
 *   sidebar         260
 *   chrome padding   64 (2 × 32 column insets)
 *   detail aside    420 (`--project-detail-width`)
 *   table min      ~420 (name + sessions + size + status + last-touched
 *                        with comfortable column widths)
 *   ─────────────────────
 *   total         ≈ 1164
 *
 * Below 1180 px the table squeezes to the point where the path ellipsis
 * eats the basename. Above, the split is usable. The breakpoint is
 * slightly above the exact budget so the transition doesn't flicker
 * as the user drags the edge.
 */
export const SPLIT_VIEW_BREAKPOINT = 1180;

/**
 * Convenience: true when the Projects section should keep its side-by-side
 * table + detail layout. False → master-or-detail single pane.
 */
export function useSplitView(): boolean {
  return useWindowWidth() >= SPLIT_VIEW_BREAKPOINT;
}
