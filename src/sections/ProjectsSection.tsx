import { useCallback, useEffect, useRef, useState } from "react";
import { Icon } from "../components/Icon";
import { api } from "../api";
import { useOperations } from "../hooks/useOperations";
import { useGlobalShortcuts } from "../hooks/useGlobalShortcuts";
import type { MoveArgs, OrphanedProject, ProjectInfo } from "../types";
import { SegmentedControl } from "../components/SegmentedControl";
import { ContextMenu, type ContextMenuItem } from "../components/ContextMenu";
import { ProjectsList, type ProjectFilter } from "./projects/ProjectsList";
import { ProjectDetail } from "./projects/ProjectDetail";
import { RenameProjectModal } from "./projects/RenameProjectModal";
import { MaintenanceView } from "./projects/MaintenanceView";
import { OrphanBanner } from "./projects/OrphanBanner";
import { AdoptOrphansModal } from "./projects/AdoptOrphansModal";

type ProjectsTab = "list" | "maintenance";
const TABS = [
  { id: "list" as const, label: "List" },
  { id: "maintenance" as const, label: "Maintenance" },
];

/**
 * Projects section. Segmented control switches between:
 * - List tab: project list + detail split
 * - Maintenance tab: Clean + Repair merged view (P2.2)
 *
 * `subRoute === "repair"` or `subRoute === "maintenance"` activates
 * the maintenance tab; the shell uses this for deep-links from the
 * PendingJournalsBanner.
 */
export function ProjectsSection({
  subRoute,
  onSubRouteChange,
}: {
  subRoute: string | null;
  onSubRouteChange: (next: string | null) => void;
}) {
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [orphans, setOrphans] = useState<OrphanedProject[]>([]);
  const [adoptOpen, setAdoptOpen] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [renameTarget, setRenameTarget] = useState<string | null>(null);
  const [filter, setFilter] = useState<ProjectFilter>("all");
  const [toast, setToast] = useState<string | null>(null);
  /** Bumped on every completed session move so ProjectDetail's
   * useEffect refetches even though `path` didn't change. */
  const [detailRefreshSignal, setDetailRefreshSignal] = useState(0);
  const [ctxMenu, setCtxMenu] = useState<{
    x: number;
    y: number;
    project: ProjectInfo;
  } | null>(null);
  const { open: openOpModal } = useOperations();

  const handleContextMenu = useCallback(
    (e: React.MouseEvent, p: ProjectInfo) => {
      e.preventDefault();
      setCtxMenu({ x: e.clientX, y: e.clientY, project: p });
    },
    [],
  );
  const closeCtxMenu = useCallback(() => setCtxMenu(null), []);

  // Audit M15: token-sequenced refresh. Mount, ⌘R, maintenance
  // callbacks, and rename completion can all call refresh() — without
  // a token, an older slower response can resolve AFTER a newer
  // response and overwrite fresher state. Each call increments the
  // token; a response is applied only if its token is still the
  // latest on resolution. Also provides an unmount guard: on unmount
  // we bump `mountedRef` to false so any in-flight response is
  // discarded.
  const refreshTokenRef = useRef(0);
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const refresh = useCallback(() => {
    const myToken = ++refreshTokenRef.current;
    setLoading(true);
    setError(null);
    api
      .projectList()
      .then((ps) => {
        if (!mountedRef.current || myToken !== refreshTokenRef.current) return;
        setProjects(ps);
        setLoading(false);
        setSelectedPath((prev) => {
          if (prev && ps.some((p) => p.original_path === prev)) return prev;
          return ps[0]?.original_path ?? null;
        });
      })
      .catch((e) => {
        if (!mountedRef.current || myToken !== refreshTokenRef.current) return;
        setError(String(e));
        setLoading(false);
      });
    // Orphan scan is independent of projectList — fire in parallel and
    // let each render when ready. An orphan-scan failure shouldn't
    // block the main list; log and leave the banner empty.
    api
      .sessionListOrphans()
      .then((os) => {
        if (!mountedRef.current || myToken !== refreshTokenRef.current) return;
        setOrphans(os);
      })
      .catch(() => {
        if (!mountedRef.current || myToken !== refreshTokenRef.current) return;
        setOrphans([]);
      });
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // ⌘R refreshes the project list from anywhere in the Projects section
  // (maintenance tab included). Matches the macOS "Reload" idiom and
  // fixes the P2 audit bug where shortcuts were Accounts-scoped only.
  useGlobalShortcuts({ onRefresh: refresh });

  const activeTab: ProjectsTab =
    subRoute === "repair" || subRoute === "maintenance" ? "maintenance" : "list";

  if (activeTab === "maintenance") {
    return (
      <>
        <aside className="sidebar projects-sidebar">
          <div className="sidebar-header">
            <span className="sidebar-title">Projects</span>
          </div>
          <div className="sidebar-segmented-slot">
            <SegmentedControl
              options={TABS}
              value={activeTab}
              onChange={(t) => onSubRouteChange(t === "list" ? null : "maintenance")}
            />
          </div>
        </aside>
        <MaintenanceView onOpTerminated={() => refresh()} />
      </>
    );
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
          <Icon name="folder" size={32} />
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
        onContextMenu={handleContextMenu}
        filter={filter}
        onFilterChange={setFilter}
        segmentedControl={
          <SegmentedControl
            options={TABS}
            value={activeTab}
            onChange={(t) => onSubRouteChange(t === "list" ? null : "maintenance")}
          />
        }
      />
      {selectedPath ? (
        <div className="content-with-banner">
          {orphans.length > 0 && (
            <div className="content-banner-slot">
              <OrphanBanner
                orphans={orphans}
                onAdopt={() => setAdoptOpen(true)}
              />
            </div>
          )}
          <ProjectDetail
            key={selectedPath}
            path={selectedPath}
            projects={projects}
            refreshSignal={detailRefreshSignal}
            onRename={(path) => setRenameTarget(path)}
            onMoved={() => {
              setToast("Session moved.");
              setDetailRefreshSignal((n) => n + 1);
              refresh();
            }}
            onError={(msg) => setToast(msg)}
          />
        </div>
      ) : (
        <main className="content">
          {orphans.length > 0 && (
            <OrphanBanner
              orphans={orphans}
              onAdopt={() => setAdoptOpen(true)}
            />
          )}
        </main>
      )}

      {adoptOpen && (
        <AdoptOrphansModal
          orphans={orphans}
          onClose={() => setAdoptOpen(false)}
          onCompleted={() => {
            setToast("Adoption done.");
            refresh();
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

      {ctxMenu && (() => {
        const p = ctxMenu.project;
        const items: ContextMenuItem[] = [
          {
            label: "Open in Finder",
            onClick: () => {
              api.revealInFinder(p.original_path).catch((e) => {
                setToast(`Couldn't reveal: ${e}`);
              });
            },
          },
          { label: "", separator: true, onClick: () => {} },
          {
            label: "Rename…",
            onClick: () => setRenameTarget(p.original_path),
          },
          {
            label: "Clean orphans…",
            onClick: () => onSubRouteChange("maintenance"),
          },
          { label: "", separator: true, onClick: () => {} },
          {
            label: "Copy path",
            onClick: () => {
              navigator.clipboard.writeText(p.original_path);
              setToast("Copied path.");
            },
          },
          {
            label: "Copy key",
            onClick: () => {
              navigator.clipboard.writeText(p.sanitized_name);
              setToast("Copied key.");
            },
          },
        ];
        return (
          <ContextMenu
            x={ctxMenu.x}
            y={ctxMenu.y}
            items={items}
            onClose={closeCtxMenu}
          />
        );
      })()}
    </>
  );
}

