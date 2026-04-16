import { useCallback, useEffect, useMemo, useState } from "react";
import { FolderSimple } from "@phosphor-icons/react";
import { api } from "../api";
import { useOperations } from "../hooks/useOperations";
import type { CleanResult, MoveArgs, ProjectInfo } from "../types";
import { ProjectsList, type ProjectFilter } from "./projects/ProjectsList";
import { ProjectDetail } from "./projects/ProjectDetail";
import { RenameProjectModal } from "./projects/RenameProjectModal";
import { CleanOrphansModal } from "./projects/CleanOrphansModal";
import { RepairView } from "./projects/RepairView";
import { classifyProject } from "./projects/projectStatus";

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
  const [renameTarget, setRenameTarget] = useState<string | null>(null);
  const [cleanOpen, setCleanOpen] = useState(false);
  const [filter, setFilter] = useState<ProjectFilter>("all");
  const [toast, setToast] = useState<string | null>(null);
  const { open: openOpModal } = useOperations();

  // Count of cleanable projects — orphan or empty. Computed once so
  // the sidebar button and the preview modal agree on the number.
  const cleanCount = useMemo(
    () =>
      projects.filter((p) => {
        const s = classifyProject(p);
        return s === "orphan" || s === "empty";
      }).length,
    [projects],
  );

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
        filter={filter}
        onFilterChange={setFilter}
        onClean={() => setCleanOpen(true)}
        cleanCount={cleanCount}
      />
      {selectedPath ? (
        <ProjectDetail
          key={selectedPath}
          path={selectedPath}
          onRename={(path) => setRenameTarget(path)}
        />
      ) : (
        <main className="content" />
      )}


      {cleanOpen && (
        <CleanOrphansModal
          onClose={() => setCleanOpen(false)}
          onDone={(result: CleanResult) => {
            refresh();
            const parts: string[] = [];
            if (result.orphans_removed > 0) {
              parts.push(
                `Removed ${result.orphans_removed} project${
                  result.orphans_removed === 1 ? "" : "s"
                }`,
              );
            }
            if (result.orphans_skipped_live > 0) {
              parts.push(
                `skipped ${result.orphans_skipped_live} with live session${
                  result.orphans_skipped_live === 1 ? "" : "s"
                }`,
              );
            }
            if (result.snapshot_paths.length > 0) {
              parts.push(
                `${result.snapshot_paths.length} recovery snapshot${
                  result.snapshot_paths.length === 1 ? "" : "s"
                } saved`,
              );
            }
            if (parts.length > 0) setToast(parts.join(" — "));
          }}
        />
      )}

      {renameTarget && (
        <RenameProjectModal
          oldPath={renameTarget}
          onClose={() => setRenameTarget(null)}
          onSubmit={async (args: MoveArgs) => {
            setRenameTarget(null);
            try {
              const opId = await api.projectMoveStart(args);
              const base = (p: string) =>
                p.split("/").filter(Boolean).pop() ?? p;
              openOpModal({
                opId,
                title: `Renaming ${base(args.oldPath)} → ${base(args.newPath)}`,
                onComplete: () => {
                  setToast("Rename complete.");
                  refresh();
                },
                onError: (detail) => {
                  setToast(`Rename failed: ${detail ?? "unknown"}`);
                  refresh();
                },
              });
            } catch (e) {
              setToast(`Couldn't start rename: ${e}`);
            }
          }}
        />
      )}

      {toast && (
        <div className="inline-toast" role="status" onClick={() => setToast(null)}>
          {toast}
        </div>
      )}
    </>
  );
}

ProjectsSection.label = "Projects";
