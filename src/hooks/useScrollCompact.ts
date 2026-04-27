import { useEffect, useState } from "react";

/**
 * Hysteresis thresholds (px) for the scroll-driven compact toggle.
 * Read once at mount via `getComputedStyle` so they stay aligned with
 * the rest of the design scale. Falls back to numeric defaults if the
 * tokens are missing — keeps the behaviour intact during a token
 * rename.
 */
const TOKEN_ENGAGE = "--scroll-compact-engage";
const TOKEN_RELEASE = "--scroll-compact-release";
const FALLBACK_ENGAGE = 16;
const FALLBACK_RELEASE = 4;

/**
 * Watch a scroll container and return a `compact` flag that engages
 * once the user has scrolled past the engage threshold and releases
 * when they scroll back above the release threshold.
 *
 * The two thresholds give a small hysteresis so the boundary doesn't
 * flicker when momentum scroll lands the user near the engage line.
 *
 * Pass `null` (or wait until the ref is populated) to disable the
 * effect — the hook re-attaches when the element identity changes.
 *
 * Extracted out of `SessionDetail` so the threshold parsing and the
 * scroll-listener lifecycle have their own home and are unit-testable
 * without rendering the full session viewer.
 */
export function useScrollCompact(scrollEl: HTMLElement | null): boolean {
  const [compact, setCompact] = useState(false);

  useEffect(() => {
    if (!scrollEl) return;
    const cs = getComputedStyle(scrollEl);
    const engage =
      Number.parseFloat(cs.getPropertyValue(TOKEN_ENGAGE)) || FALLBACK_ENGAGE;
    const release =
      Number.parseFloat(cs.getPropertyValue(TOKEN_RELEASE)) || FALLBACK_RELEASE;

    const onScroll = () => {
      const top = scrollEl.scrollTop;
      setCompact((c) => {
        if (c && top < release) return false;
        if (!c && top > engage) return true;
        return c;
      });
    };

    scrollEl.addEventListener("scroll", onScroll, { passive: true });
    onScroll();
    return () => scrollEl.removeEventListener("scroll", onScroll);
  }, [scrollEl]);

  return compact;
}
