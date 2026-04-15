import { useCallback, useEffect, useState } from "react";
import { FolderSimple } from "@phosphor-icons/react";
import { api } from "../api";
import type { ProjectInfo } from "../types";
import { ProjectsList } from "./projects/ProjectsList";
import { ProjectDetail } from "./projects/ProjectDetail";
import { RepairView } from "./projects/RepairView";

/**
 * Projects section. Routes between:
 * - default list-view (ProjectsList + ProjectDetail split)
 * - repair subview (RepairView)
 *
 * `subRoute === "repair"` activates the subview; the shell is
 * responsible for setting this (e.g. via the global PendingJournalsBanner
 * deep-link). The section also exposes a "Back to Projects" affordance
 * via the subview so the user can return without digging into the rail.
 */
export function ProjectsSection({
  subRoute,
  onSubRouteChange,
}: {
  subRoute: string | null;
  onSubRouteChange: (next: string | null) => void;
}) {
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);

  const refresh = useCallback(() => {
    setLoading(true);
    setError(null);
    api
      .projectList()
      .then((ps) => {
        setProjects(ps);
        setLoading(false);
        setSelectedPath((prev) => {
          if (prev && ps.some((p) => p.original_path === prev)) return prev;
          return ps[0]?.original_path ?? null;
        });
      })
      .catch((e) => {
        setError(String(e));
        setLoading(false);
      });
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  if (subRoute === "repair") {
    return <RepairView onBack={() => onSubRouteChange(null)} />;
  }

  if (loading && projects.length === 0) {
    return (
      <main className="content">
        <div className="skeleton-container">
          <div className="skeleton skeleton-header" />
          <div className="skeleton skeleton-card" />
          <div className="skeleton skeleton-card" />
        </div>
      </main>
    );
  }

  if (error && projects.length === 0) {
    return (
      <main className="content">
        <div className="empty">
          <h2>Couldn't load projects</h2>
          <p className="muted mono">{error}</p>
          <button className="primary" onClick={refresh}>Retry</button>
        </div>
      </main>
    );
  }

  if (projects.length === 0) {
    return (
      <main className="content">
        <div className="empty">
          <FolderSimple size={32} weight="thin" />
          <h2>No CC projects</h2>
          <p className="muted">
            Run Claude Code in any directory to create a project.
          </p>
        </div>
      </main>
    );
  }

  return (
    <>
      <ProjectsList
        projects={projects}
        selectedPath={selectedPath}
        onSelect={setSelectedPath}
      />
      {selectedPath ? (
        <ProjectDetail
          key={selectedPath}
          path={selectedPath}
          onRename={(path) => {
            // Stub: Step 5 replaces this with the rename modal.
            // Surfaced for now so the flow is reviewable end-to-end.
            console.info("[stub] rename requested for", path);
          }}
        />
      ) : (
        <main className="content" />
      )}
    </>
  );
}

ProjectsSection.label = "Projects";
