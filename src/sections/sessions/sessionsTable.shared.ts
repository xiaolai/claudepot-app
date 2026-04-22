import type { SessionRow } from "../../types";

/**
 * Shared types, layout constants, and pure helpers for the
 * `SessionsTable` cluster of components. Lifted out of the main
 * `SessionsTable.tsx` so each rendered component lives in its own
 * file (project rule: one component per file in `components/`).
 */

export type SessionFilter = "all" | "errors" | "sidechain";

export type SortKey = "last_active" | "project" | "turns" | "tokens" | "size";
export type SortDir = "asc" | "desc";

/**
 * Column template:
 *   glyph | session preview | project | turns | tokens | last-active | chevron
 *
 * Shared between the table header and every row so the columns line
 * up. Changing this template is the single point of edit.
 */
export const COLS = "var(--sp-20) 2fr 1.1fr 0.6fr 0.7fr 0.9fr var(--sp-24)";

/**
 * Above this count, switch to row-level virtualization.
 *
 * Math: at ~72px per row the non-virtualized list still renders fine
 * up to a couple of laptop viewports' worth of work; past that, every
 * row is off-screen DOM the browser paints for nothing, which is the
 * condition virtualization wins on. 80 rows × 72 px ≈ 5760 px — above
 * any realistic viewport, so the crossover matches the paint budget.
 *
 * Secondary factor: jsdom returns 0 for layout metrics, so tests that
 * mount fewer than this many rows stay on the plain path and assert
 * real DOM. A dedicated virtualization test mocks layout to exercise
 * the virtualized path explicitly.
 */
export const VIRTUALIZE_THRESHOLD = 80;

/**
 * Initial row-height estimate used by the virtualizer before each row's
 * real height is measured. Biased above the common "metadata line
 * only" row (~58px) so that rows that also show a deep-search snippet
 * (~85px) don't jolt the scrollbar thumb on first paint. The real
 * height is measured post-paint via `measureElement`, so this only
 * controls the first frame.
 */
export const ESTIMATED_ROW_PX = 72;

/**
 * Tally of the `errors` and `sidechain` flags across a session list.
 * `all` is the raw total; the two other counts feed the FilterChip
 * badges in `SessionsSection`.
 */
export function countSessionStatus(
  sessions: SessionRow[],
): Record<SessionFilter, number> {
  const counts: Record<SessionFilter, number> = {
    all: sessions.length,
    errors: 0,
    sidechain: 0,
  };
  for (const s of sessions) {
    if (s.has_error) counts.errors += 1;
    if (s.is_sidechain) counts.sidechain += 1;
  }
  return counts;
}
