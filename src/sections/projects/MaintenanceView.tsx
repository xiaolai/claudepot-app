import { useCallback, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { CleanResult } from "../../types";
import { useAppState } from "../../providers/AppStateProvider";
import { AbandonedCleanupCard } from "./AbandonedCleanupCard";
import { GcCard } from "./GcCard";
import { RepairView } from "./RepairView";
import { CleanOrphansModal } from "./CleanOrphansModal";
import { Button } from "../../components/primitives/Button";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";

/**
 * Merged Clean + Repair view (P2.2). Always visible from the
 * Projects segmented control — no more hidden-behind-banner discovery.
 */
export function MaintenanceView({
  onOpTerminated,
}: {
  onOpTerminated?: () => void;
}) {
  const { t } = useTranslation("projects");
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
          t("maint.removedProjects", { count: result.orphans_removed }),
        );
      if (result.orphans_skipped_live > 0)
        parts.push(
          t("maint.skippedLive", { n: result.orphans_skipped_live }),
        );
      if (result.snapshot_paths.length > 0)
        parts.push(
          t("maint.snapshotsSaved", { n: result.snapshot_paths.length }),
        );
      if (parts.length > 0) pushToast("info", parts.join(" — "));
    },
    [pushToast, t],
  );

  return (
    <main className="content maintenance-view">
      {/* Clean section */}
      <section className="maintenance-section">
        <div className="maintenance-section-header">
          <Glyph g={NF.trash} style={{ fontSize: 14 }} />
          <h2>{t("maint.cleanHeading")}</h2>
        </div>
        <p className="muted maintenance-desc">
          {t("maint.cleanDesc")}
        </p>
        <Button
          variant="solid"
          onClick={() => setCleanOpen(true)}
          title={t("maint.previewTitle")}
        >
          {t("maint.previewCleanup")}
        </Button>
      </section>

      {/* Recovery-artifact cleanup — hidden when there's nothing
          abandoned. Sits between Clean and Repair because its
          artifacts are products of the Repair flow (Abandon writes
          the sidecar; this card sweeps it up). */}
      <AbandonedCleanupCard
        onCleaned={() => {
          latestPushToast.current("info", t("maint.abandonedRemovedToast"));
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
          <Glyph g={NF.tools} style={{ fontSize: 14 }} />
          <h2>{t("maint.repairHeading")}</h2>
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
