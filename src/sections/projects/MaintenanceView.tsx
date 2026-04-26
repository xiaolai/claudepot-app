import { useCallback, useRef, useState } from "react";
import { Icon } from "../../components/Icon";
import type { CleanResult } from "../../types";
import { useAppState } from "../../providers/AppStateProvider";
import { AbandonedCleanupCard } from "./AbandonedCleanupCard";
import { GcCard } from "./GcCard";
import { RepairView } from "./RepairView";
import { CleanOrphansModal } from "./CleanOrphansModal";

/**
 * Merged Clean + Repair view (P2.2). Always visible from the
 * Projects segmented control — no more hidden-behind-banner discovery.
 */
export function MaintenanceView({
  onOpTerminated,
}: {
  onOpTerminated?: () => void;
}) {
  const [cleanOpen, setCleanOpen] = useState(false);
  // Bump on successful abandoned-cleanup to force the embedded
  // RepairView to re-fetch (its `entries` list shares state with
  // the journals dir that just shrank).
  const [repairRefreshKey, setRepairRefreshKey] = useState(0);
  const { pushToast } = useAppState();
  const latestPushToast = useRef(pushToast);
  latestPushToast.current = pushToast;

  const handleCleanDone = useCallback(
    (result: CleanResult) => {
      const parts: string[] = [];
      if (result.orphans_removed > 0)
        parts.push(
          `Removed ${result.orphans_removed} project${result.orphans_removed === 1 ? "" : "s"}`,
        );
      if (result.orphans_skipped_live > 0)
        parts.push(
          `skipped ${result.orphans_skipped_live} with live sessions`,
        );
      if (result.snapshot_paths.length > 0)
        parts.push(
          `${result.snapshot_paths.length} recovery snapshots saved`,
        );
      if (parts.length > 0) pushToast("info", parts.join(" — "));
    },
    [pushToast],
  );

  return (
    <main className="content maintenance-view">
      {/* Clean section */}
      <section className="maintenance-section">
        <div className="maintenance-section-header">
          <Icon name="trash-2" size={14} />
          <h2>Clean Orphan Projects</h2>
        </div>
        <p className="muted maintenance-desc">
          Remove CC project directories whose source folder no longer exists.
          Unreachable projects (unmounted volumes) are never auto-cleaned.
        </p>
        <button className="btn primary" onClick={() => setCleanOpen(true)}
          title="Preview which orphan projects would be removed">
          Preview cleanup…
        </button>
      </section>

      {/* Recovery-artifact cleanup — hidden when there's nothing
          abandoned. Sits between Clean and Repair because its
          artifacts are products of the Repair flow (Abandon writes
          the sidecar; this card sweeps it up). */}
      <AbandonedCleanupCard
        onCleaned={() => {
          latestPushToast.current("info", "Abandoned recovery files removed.");
          // Refresh the Repair list too — list_actionable excludes
          // abandoned entries, but a stale cached view could still
          // reference the journal paths we just deleted.
          setRepairRefreshKey((n) => n + 1);
          onOpTerminated?.();
        }}
      />

      <GcCard pushToast={pushToast} />

      {/* Repair section — reuse existing RepairView without the back button */}
      <section className="maintenance-section">
        <div className="maintenance-section-header">
          <Icon name="wrench" size={14} />
          <h2>Repair Queue</h2>
        </div>
        <RepairView
          key={repairRefreshKey}
          onBack={() => {}}
          embedded
          onOpTerminated={onOpTerminated}
        />
      </section>

      {cleanOpen && (
        <CleanOrphansModal
          onClose={() => setCleanOpen(false)}
          onDone={(result) => { handleCleanDone(result); }}
        />
      )}
    </main>
  );
}
