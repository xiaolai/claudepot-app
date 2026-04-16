import { IconContext } from "@phosphor-icons/react";
import { SectionRail } from "./components/SectionRail";
import { PendingJournalsBanner } from "./components/PendingJournalsBanner";
import { RunningOpStrip } from "./components/RunningOpStrip";
import { sections, sectionIds } from "./sections/registry";
import { AccountsSection } from "./sections/AccountsSection";
import { ProjectsSection } from "./sections/ProjectsSection";
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
  const { count: pendingCount, refresh: refreshPendingBanner } =
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

  // Hide the banner whenever the user is already looking at Repair —
  // no point nagging from the page they'd navigate to.
  const onRepairSubview = section === "projects" && subRoute === "repair";
  const showBanner =
    pendingCount !== null && pendingCount > 0 && !onRepairSubview;

  return (
    <IconContext.Provider value={{ size: 16, weight: "light" }}>
      <div className="app-layout">
        <div className="titlebar-drag" data-tauri-drag-region />
        <SectionRail sections={sections} active={section} onSelect={setSection} />
        {section === "accounts" && <AccountsSection />}
        {section === "projects" && (
          <ProjectsSection
            subRoute={subRoute}
            onSubRouteChange={setSubRoute}
          />
        )}
        {showBanner && (
          <div className="global-banner-slot">
            <PendingJournalsBanner
              count={pendingCount}
              onOpen={() => setSection("projects", "repair")}
            />
          </div>
        )}
        <RunningOpStrip
          ops={runningOps}
          onReopen={(opId) => {
            const op = runningOps.find((o) => o.op_id === opId);
            if (!op) return;
            openOp({
              opId,
              title: labelFor(op),
              onComplete: () => refreshPendingBanner(),
              onError: () => refreshPendingBanner(),
            });
          }}
        />

        {activeOp && (
          <OperationProgressModal
            key={activeOp.opId}
            opId={activeOp.opId}
            title={activeOp.title}
            onClose={closeOp}
            onComplete={activeOp.onComplete}
            onError={activeOp.onError}
            onOpenRepair={() => {
              closeOp();
              setSection("projects", "repair");
            }}
          />
        )}
      </div>
    </IconContext.Provider>
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
