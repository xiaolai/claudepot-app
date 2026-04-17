import { useCallback, useState } from "react";
import { Trash2, Wrench } from "lucide-react";
import type { CleanResult } from "../../types";
import { useToasts } from "../../hooks/useToasts";
import { ToastContainer } from "../../components/ToastContainer";
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
  const { toasts, pushToast, dismissToast } = useToasts();

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
          <Trash2 size={16} />
          <h2>Clean Orphan Projects</h2>
        </div>
        <p className="muted maintenance-desc">
          Remove CC project directories whose source folder no longer exists.
          Unreachable projects (unmounted volumes) are never auto-cleaned.
        </p>
        <button className="primary" onClick={() => setCleanOpen(true)}
          title="Preview which orphan projects would be removed">
          Preview cleanup…
        </button>
      </section>

      {/* Repair section — reuse existing RepairView without the back button */}
      <section className="maintenance-section">
        <div className="maintenance-section-header">
          <Wrench size={16} />
          <h2>Repair Queue</h2>
        </div>
        <RepairView onBack={() => {}} embedded onOpTerminated={onOpTerminated} />
      </section>

      {cleanOpen && (
        <CleanOrphansModal
          onClose={() => setCleanOpen(false)}
          onDone={(result) => { handleCleanDone(result); }}
        />
      )}

      <ToastContainer toasts={toasts} onDismiss={dismissToast} />
    </main>
  );
}
