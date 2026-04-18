import { Icon } from "./Icon";
import type { PendingJournalsSummary } from "../types";

/**
 * Global banner shown when actionable rename journals exist.
 * Status-aware (plan §7.5):
 * - `stale` (≥24h) → warning tone, "resolve" copy
 * - `pending` (<24h, dead lock) → neutral tone, informational copy
 * - `running` entries are excluded — they're already visible in
 *   the RunningOpStrip, no point nagging about them.
 */
export function PendingJournalsBanner({
  summary,
  onOpen,
}: {
  summary: PendingJournalsSummary;
  onOpen: () => void;
}) {
  const total = summary.pending + summary.stale;
  if (total <= 0) return null;

  const hasStale = summary.stale > 0;
  const label =
    total === 1
      ? "1 pending rename journal"
      : `${total} pending rename journals`;

  return (
    <button
      type="button"
      className={`pending-journals-banner${hasStale ? " stale" : ""}`}
      aria-label={`${label}. Open Repair.`}
      onClick={onOpen}
    >
      {hasStale ? <Icon name="alert-triangle" size={14} /> : <Icon name="wrench" size={14} />}
      <span>
        <strong>{label}.</strong>{" "}
        {hasStale
          ? summary.pending === 0
            ? "All are ≥24h old — resolve them via Repair."
            : `${summary.stale} stale ≥24h. Resolve via Repair.`
          : "Click to resolve."}
      </span>
    </button>
  );
}
