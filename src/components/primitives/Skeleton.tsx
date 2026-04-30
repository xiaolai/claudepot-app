// Loading-state placeholder. Wraps the existing `.skeleton`,
// `.skeleton-card`, `.skeleton-header` CSS classes (defined in
// `src/styles/components/modals.css`) so consumers don't sprinkle
// className strings throughout sections.
//
// Use this in place of bare "Loading…" text on list/grid surfaces
// where the user is waiting for structure they expect to scan
// (cards, table rows, panes). Inline button labels like "Checking…"
// or single-line modal hints stay as text — a shimmer block where
// a word is about to appear is worse than a word.
//
// Accessibility: shimmer blocks are `aria-hidden`; the container
// carries `role="status"` + `aria-live="polite"` and a visually
// hidden text label so screen readers hear "Loading…" without the
// sighted UI carrying the literal word. `.claude/rules/design.md`
// requires non-color signal for state — the SR label is that signal.

import type { CSSProperties } from "react";

const srOnly: CSSProperties = {
  position: "absolute",
  width: 1,
  height: 1,
  padding: 0,
  margin: -1,
  overflow: "hidden",
  clip: "rect(0,0,0,0)",
  whiteSpace: "nowrap",
  border: 0,
};

export function Skeleton({
  variant = "card",
  style,
}: {
  variant?: "card" | "header" | "card-short";
  style?: CSSProperties;
}) {
  const cls =
    variant === "header"
      ? "skeleton skeleton-header"
      : variant === "card-short"
        ? "skeleton skeleton-card short"
        : "skeleton skeleton-card";
  return <div className={cls} style={style} aria-hidden="true" />;
}

/** A typical list-pane placeholder: one header bar + N card rows.
 *  Wraps in `.skeleton-container` so the inherited padding / gap
 *  rules from `modals.css` apply uniformly. The container is the
 *  live region — individual blocks stay decorative. */
export function SkeletonList({
  rows = 3,
  showHeader = true,
  label = "Loading…",
  style,
}: {
  rows?: number;
  showHeader?: boolean;
  /** Screen-reader text. Override only when "Loading…" misleads —
   *  e.g. a refresh that's expected to be near-instant might say
   *  "Refreshing accounts…". */
  label?: string;
  style?: CSSProperties;
}) {
  return (
    <div
      className="skeleton-container"
      style={style}
      role="status"
      aria-live="polite"
      aria-busy="true"
    >
      <span style={srOnly}>{label}</span>
      {showHeader && <Skeleton variant="header" />}
      {Array.from({ length: rows }).map((_, i) => (
        <Skeleton key={i} variant="card" />
      ))}
    </div>
  );
}

/** Compact rows-only variant for surfaces that already have their
 *  own header (e.g. tables under a SectionLabel). */
export function SkeletonRows({
  rows = 3,
  label,
  style,
}: {
  rows?: number;
  label?: string;
  style?: CSSProperties;
}) {
  return (
    <SkeletonList rows={rows} showHeader={false} label={label} style={style} />
  );
}
