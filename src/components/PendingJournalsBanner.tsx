import { Wrench } from "@phosphor-icons/react";

/**
 * Global banner shown when actionable rename journals exist. Clicking
 * navigates to Projects → repair subview so the user can resolve them
 * without hunting for the entry point.
 *
 * Kept intentionally minimal: one row, neutral background (per plan
 * §7.5: status-aware variants come in Step 4 once we can distinguish
 * pending vs stale by polling `repair_list` instead of just the count).
 */
export function PendingJournalsBanner({
  count,
  onOpen,
}: {
  count: number;
  onOpen: () => void;
}) {
  if (count <= 0) return null;
  const label =
    count === 1
      ? "1 pending rename journal"
      : `${count} pending rename journals`;
  return (
    <button
      type="button"
      className="pending-journals-banner"
      aria-label={`${label}. Open Repair.`}
      onClick={onOpen}
    >
      <Wrench />
      <span>
        <strong>{label}.</strong> Click to resolve.
      </span>
    </button>
  );
}
