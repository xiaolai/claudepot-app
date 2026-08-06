import { i18n } from "../../lib/i18n";
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
 *
 * Plain function, not a component — reads the global i18n instance.
 * Call sites render inside components that subscribe via
 * `useTranslation`, so a language switch re-invokes this.
 */
export function liveDotTitle(live: {
  status: LiveSessionSummary["status"];
  errored: boolean;
  waiting_for: string | null;
}): string {
  const ns = { ns: "components" } as const;
  if (live.errored) return i18n.t("liveDot.errored", ns);
  if (live.status === "waiting") {
    return live.waiting_for
      ? i18n.t("liveDot.waitingOn", { ...ns, reason: live.waiting_for })
      : i18n.t("liveDot.waiting", ns);
  }
  return live.status === "busy"
    ? i18n.t("liveDot.busy", ns)
    : i18n.t("liveDot.idle", ns);
}
