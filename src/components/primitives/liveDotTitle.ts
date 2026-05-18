import type { LiveSessionSummary } from "../../types/activity";

/**
 * Tooltip text for a `LiveStatusDot` paired with a `LiveSessionSummary`.
 *
 * Lives next to the primitive so every call site stays in sync —
 * adding a new overlay (e.g. `stuck`) means one edit, not two. The
 * verb mapping mirrors `STATUS_TONE` in
 * `src/sections/sessions/components/liveStatusBits.tsx`.
 *
 * Accepts a structural subset of `LiveSessionSummary` so callers
 * with narrower types can use it without a cast.
 */
export function liveDotTitle(live: {
  status: LiveSessionSummary["status"];
  errored: boolean;
  waiting_for: string | null;
}): string {
  if (live.errored) return "Errored";
  if (live.status === "waiting") {
    return live.waiting_for ? `Waiting · ${live.waiting_for}` : "Waiting";
  }
  return live.status === "busy" ? "Busy" : "Idle";
}
