import { useEffect } from "react";
import { SectionRail } from "./components/SectionRail";
import { PendingJournalsBanner } from "./components/PendingJournalsBanner";
import { RunningOpStrip } from "./components/RunningOpStrip";
import { sections, sectionIds } from "./sections/registry";
import { AccountsSection } from "./sections/AccountsSection";
import { ProjectsSection } from "./sections/ProjectsSection";
import { SettingsSection } from "./sections/SettingsSection";
import { OperationProgressModal } from "./sections/projects/OperationProgressModal";
import { useSection } from "./hooks/useSection";
import { usePendingJournals } from "./hooks/usePendingJournals";
import { useRunningOps } from "./hooks/useRunningOps";
import { OperationsProvider, useOperations } from "./hooks/useOperations";
import type { RunningOpInfo } from "./types";

function AppShell() {
  const { section, subRoute, setSection, setSubRoute } = useSection(
    sectionIds[0],
    sectionIds,
  );
  const { summary: pendingSummary, refresh: refreshPendingBanner } =
    usePendingJournals();
  const { ops: runningOps } = useRunningOps();
  const { active: activeOp, open: openOp, close: closeOp } = useOperations();

  const labelFor = (op: RunningOpInfo): string => {
    const verb =
      op.kind === "repair_resume"
        ? "Resuming"
        : op.kind === "repair_rollback"
          ? "Rolling back"
          : "Renaming";
    const base = (p: string) => p.split("/").filter(Boolean).pop() ?? p;
    return `${verb} ${base(op.old_path)} → ${base(op.new_path)}`;
  };

  // Cmd+, opens Settings (standard macOS shortcut)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "," && !e.shiftKey && !e.altKey) {
        e.preventDefault();
        setSection("settings");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setSection]);

  // Hide the banner whenever the user is already looking at Repair —
  // no point nagging from the page they'd navigate to.
  const onRepairSubview = section === "projects" &&
    (subRoute === "repair" || subRoute === "maintenance");
  const actionableTotal =
    pendingSummary === null
      ? 0
      : pendingSummary.pending + pendingSummary.stale;
  const showBanner =
    pendingSummary !== null && actionableTotal > 0 && !onRepairSubview;

  return (
    <div className="app-layout">
      <SectionRail sections={sections} active={section} onSelect={setSection} />
      {section === "accounts" && <AccountsSection onNavigate={setSection} />}
      {section === "projects" && (
        <ProjectsSection
          subRoute={subRoute}
          onSubRouteChange={setSubRoute}
        />
      )}
      {section === "settings" && <SettingsSection />}
      {showBanner && pendingSummary && (
        <div className="global-banner-slot">
          <PendingJournalsBanner
            summary={pendingSummary}
            onOpen={() => setSection("projects", "repair")}
          />
        </div>
      )}
      <RunningOpStrip
        ops={runningOps}
        onReopen={(opId) => {
          const op = runningOps.find((o) => o.op_id === opId);
          if (!op) return;
          // Audit Low: don't call refreshPendingBanner here. The
          // modal's own onComplete/onError below already does it;
          // duplicating it caused two invalidations per terminal
          // event when reopening a running op.
          openOp({
            opId,
            title: labelFor(op),
          });
        }}
      />

      {activeOp && (
        <OperationProgressModal
          key={activeOp.opId}
          opId={activeOp.opId}
          title={activeOp.title}
          onClose={closeOp}
          // Every terminal event — whether repair or rename —
          // invalidates the pending-journals banner (plan §7.5).
          onComplete={() => {
            activeOp.onComplete?.();
            refreshPendingBanner();
          }}
          onError={(detail) => {
            activeOp.onError?.(detail);
            refreshPendingBanner();
          }}
          onOpenRepair={() => {
            closeOp();
            setSection("projects", "repair");
          }}
        />
      )}
    </div>
  );
}

function App() {
  return (
    <OperationsProvider>
      <AppShell />
    </OperationsProvider>
  );
}

export default App;
