import { IconContext } from "@phosphor-icons/react";
import { SectionRail } from "./components/SectionRail";
import { PendingJournalsBanner } from "./components/PendingJournalsBanner";
import { RunningOpStrip } from "./components/RunningOpStrip";
import { sections, sectionIds } from "./sections/registry";
import { AccountsSection } from "./sections/AccountsSection";
import { ProjectsSection } from "./sections/ProjectsSection";
import { useSection } from "./hooks/useSection";
import { usePendingJournals } from "./hooks/usePendingJournals";
import { useRunningOps } from "./hooks/useRunningOps";

function App() {
  const { section, subRoute, setSection, setSubRoute } = useSection(
    sectionIds[0],
    sectionIds,
  );
  const { count: pendingCount } = usePendingJournals();
  const { ops: runningOps } = useRunningOps();

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
            // Re-opening into an active modal comes in Step 6 when
            // RenameProjectModal/OperationProgressModal wire by op-id
            // through a shared context. For now just deep-link to
            // Repair so the user sees the list.
            console.info("[stub] reopen op", opId);
            setSection("projects", "repair");
          }}
        />
      </div>
    </IconContext.Provider>
  );
}

export default App;
