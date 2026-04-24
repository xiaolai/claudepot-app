import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import { useOperations } from "../hooks/useOperations";
import { useGlobalShortcuts } from "../hooks/useGlobalShortcuts";
import { useCompactHeader } from "../hooks/useWindowWidth";
import type { MoveArgs, OrphanedProject, ProjectInfo } from "../types";
import { ContextMenu, type ContextMenuItem } from "../components/ContextMenu";
import { classifyProject } from "./projects/projectStatus";
import { Button } from "../components/primitives/Button";
import { IconButton } from "../components/primitives/IconButton";
import { Input } from "../components/primitives/Input";
import { Toast } from "../components/primitives/Toast";
import { NF } from "../icons";
import { ScreenHeader } from "../shell/ScreenHeader";
import {
  countByStatus,
  type ProjectFilter,
} from "./projects/ProjectsTable";
import { ProjectsList } from "./projects/ProjectsList";
import { ProjectDetail } from "./projects/ProjectDetail";
import { SessionDetail } from "./sessions/SessionDetail";
import { ConfigSection } from "./ConfigSection";
import { SectionTab } from "./sessions/components/SectionTab";
import { RenameProjectModal } from "./projects/RenameProjectModal";
import { MaintenanceView } from "./projects/MaintenanceView";
import { OrphanBanner } from "./projects/OrphanBanner";
import { AdoptOrphansModal } from "./projects/AdoptOrphansModal";


/**
 * Projects section. Three sub-routes:
 *  - null / "list": ScreenHeader + filter/segmented + ProjectsTable
 *    + right-pane ProjectDetail (when a row is selected).
 *  - "maintenance" / "repair": MaintenanceView (unchanged).
 *
 * Rename, adopt-orphans, and context-menu flows work as before; they
 * just render into the new shell instead of the old sidebar chrome.
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
  // Per-project tab + embedded Config sub-route. Reset on project
  // switch so each project starts on Sessions. Config sub-route is
  // local to this section — not persisted across app restarts, which
  // matches the "follow the user" model (no sticky drilldowns).
  const [projectTab, setProjectTab] = useState<"sessions" | "config">("sessions");
  const [configSubRoute, setConfigSubRoute] = useState<string | null>(null);
  // Master-detail state for the Sessions tab: which transcript file
  // the user opened. `null` = show the session list. Reset on project
  // or tab switch so we never carry a stale transcript from one
  // project into another.
  const [openedSessionPath, setOpenedSessionPath] = useState<string | null>(
    null,
  );
  useEffect(() => {
    setProjectTab("sessions");
    setConfigSubRoute(null);
    setOpenedSessionPath(null);
  }, [selectedPath]);
  useEffect(() => {
    if (projectTab !== "sessions") setOpenedSessionPath(null);
  }, [projectTab]);
  const [renameTarget, setRenameTarget] = useState<string | null>(null);
  const [filter, setFilter] = useState<ProjectFilter>("all");
  const [nameFilter, setNameFilter] = useState("");
  const [toast, setToast] = useState<string | null>(null);
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
          return null;
        });
      })
      .catch((e) => {
        if (!mountedRef.current || myToken !== refreshTokenRef.current) return;
        setError(String(e));
        setLoading(false);
      });
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

  useGlobalShortcuts({ onRefresh: refresh });

  const counts = useMemo(() => countByStatus(projects), [projects]);
  const filteredByName = useMemo(() => {
    if (!nameFilter.trim()) return projects;
    const q = nameFilter.toLowerCase();
    return projects.filter(
      (p) =>
        p.original_path.toLowerCase().includes(q) ||
        p.sanitized_name.toLowerCase().includes(q),
    );
  }, [projects, nameFilter]);

  const compact = useCompactHeader();

  const activeTab: "list" | "maintenance" =
    subRoute === "repair" || subRoute === "maintenance"
      ? "maintenance"
      : "list";

  if (activeTab === "maintenance") {
    return (
      <>
        <ScreenHeader
          crumbs={["claudepot", "projects", "maintenance"]}
          title="Maintenance"
          subtitle="Clean orphaned projects and resume pending rename journals."
          actions={
            <Button
              variant="ghost"
              glyph={NF.arrowR}
              onClick={() => onSubRouteChange(null)}
              title="Back to project list"
            >
              Back to list
            </Button>
          }
        />
        <MaintenanceView onOpTerminated={() => refresh()} />
      </>
    );
  }
  const subtitle = (() => {
    const total = projects.length;
    if (total === 0) {
      return "No CC projects yet — run `claude` in any directory to create one.";
    }
    const narrowed = nameFilter.trim() && filteredByName.length !== total;
    if (narrowed) {
      return `${filteredByName.length} of ${total} project${total === 1 ? "" : "s"} shown`;
    }
    const actionable = counts.orphan + counts.unreachable + counts.empty;
    if (actionable === 0) {
      return `${total} project${total === 1 ? "" : "s"} · all healthy`;
    }
    const pieces: string[] = [];
    if (counts.orphan) pieces.push(`${counts.orphan} orphan`);
    if (counts.unreachable) pieces.push(`${counts.unreachable} offline`);
    if (counts.empty) pieces.push(`${counts.empty} empty`);
    return `${total} project${total === 1 ? "" : "s"} · ${pieces.join(" · ")}`;
  })();

  return (
    <>
      <ScreenHeader
        title="Projects"
        subtitle={subtitle}
        actions={
          compact ? (
            <>
              <IconButton
                glyph={NF.wrench}
                onClick={() => onSubRouteChange("maintenance")}
                title="Maintenance — clean + repair"
                aria-label="Maintenance"
              />
              <IconButton
                glyph={NF.refresh}
                onClick={refresh}
                title="Refresh (⌘R)"
                aria-label="Refresh projects"
              />
            </>
          ) : (
            <>
              <Button
                variant="ghost"
                glyph={NF.wrench}
                glyphColor="var(--fg-muted)"
                onClick={() => onSubRouteChange("maintenance")}
                title="Maintenance: clean + repair"
              >
                Maintenance
              </Button>
              <Button
                variant="ghost"
                glyph={NF.refresh}
                glyphColor="var(--fg-muted)"
                onClick={refresh}
                title="Refresh project list (⌘R)"
              >
                Refresh projects
              </Button>
            </>
          )
        }
      />

      {orphans.length > 0 && (
        <div style={{ padding: "var(--sp-12) var(--sp-32) 0" }}>
          <OrphanBanner
            orphans={orphans}
            onAdopt={() => setAdoptOpen(true)}
          />
        </div>
      )}

      {error && projects.length === 0 ? (
        <div
          style={{
            flex: 1,
            minHeight: 0,
            overflow: "auto",
            padding: "var(--sp-48)",
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            gap: "var(--sp-12)",
          }}
        >
          <h2 style={{ fontSize: "var(--fs-lg)", margin: 0 }}>
            Couldn't load projects
          </h2>
          <p
            style={{
              color: "var(--fg-muted)",
              fontSize: "var(--fs-xs)",
              margin: 0,
            }}
          >
            {error}
          </p>
          <Button variant="solid" onClick={refresh}>
            Retry
          </Button>
        </div>
      ) : (
        <div
          style={{
            flex: 1,
            display: "grid",
            gridTemplateColumns: "var(--config-tree-width) minmax(0, 1fr)",
            minHeight: 0,
          }}
        >
          {/* Left rail — filter chips + compact project list. Mirrors
              Global section's column proportions so the two sections
              read as siblings. */}
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              minHeight: 0,
              minWidth: 0,
              background: "var(--bg-sunken)",
              borderRight: "var(--bw-hair) solid var(--line)",
            }}
          >
            <ProjectsListFilterBar
              nameFilter={nameFilter}
              onNameFilter={setNameFilter}
              filter={filter}
              onFilter={setFilter}
              counts={{ all: projects.length, ...counts }}
              loading={loading && projects.length > 0}
            />
            <ProjectsList
              projects={filteredByName}
              selectedPath={selectedPath}
              onSelect={setSelectedPath}
              onContextMenu={handleContextMenu}
            />
          </div>

          {/* Right column — project detail with tabs, or placeholder
              when nothing is selected. Always mounted so the layout
              reads symmetrically with Global. */}
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              minHeight: 0,
              minWidth: 0,
            }}
          >
            {selectedPath ? (
              <>
                <ProjectTabBar active={projectTab} onChange={setProjectTab} />
                <div
                  style={{
                    flex: 1,
                    minHeight: 0,
                    display: "flex",
                    flexDirection: "column",
                  }}
                >
                  {projectTab === "sessions" ? (
                    openedSessionPath ? (
                      // Transcript viewer. `key` on the path so
                      // switching transcripts cleanly remounts rather
                      // than reusing the search/pagination state.
                      <SessionDetail
                        key={openedSessionPath}
                        filePath={openedSessionPath}
                        projects={projects}
                        refreshSignal={detailRefreshSignal}
                        onMoved={() => {
                          setDetailRefreshSignal((n) => n + 1);
                          refresh();
                        }}
                        onError={(msg) => setToast(msg)}
                        onBack={() => setOpenedSessionPath(null)}
                      />
                    ) : (
                      <ProjectDetail
                        key={selectedPath}
                        path={selectedPath}
                        projects={projects}
                        refreshSignal={detailRefreshSignal}
                        onRename={(path) => setRenameTarget(path)}
                        onMoved={() => {
                          // One signal per surface (design §Non-negotiables).
                          // The MoveSessionModal's own done-state carries the
                          // detailed report; firing a toast here would be a
                          // second signal for the same event.
                          setDetailRefreshSignal((n) => n + 1);
                          refresh();
                        }}
                        onError={(msg) => setToast(msg)}
                        onOpenMaintenance={() => onSubRouteChange("maintenance")}
                        onOpenInConfig={() => setProjectTab("config")}
                        onOpenSession={(p) => setOpenedSessionPath(p)}
                      />
                    )
                  ) : (
                    <ConfigSection
                      key={`config:${selectedPath}`}
                      subRoute={configSubRoute}
                      onSubRouteChange={setConfigSubRoute}
                      forcedAnchor={{ kind: "folder", path: selectedPath }}
                    />
                  )}
                </div>
              </>
            ) : (
              <div
                style={{
                  flex: 1,
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  color: "var(--fg-faint)",
                  fontSize: "var(--fs-sm)",
                  textAlign: "center",
                  padding: "var(--sp-32)",
                }}
              >
                Select a project from the list to see its sessions and
                config.
              </div>
            )}
          </div>
        </div>
      )}

      {adoptOpen && (
        <AdoptOrphansModal
          orphans={orphans}
          onClose={() => setAdoptOpen(false)}
          onCompleted={() => {
            // Per-row status lives inside the modal; don't double-signal.
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

      <Toast message={toast} onDismiss={() => setToast(null)} />

      {ctxMenu &&
        (() => {
          const p = ctxMenu.project;
          const status = classifyProject(p);
          const canOpenInConfig =
            status !== "orphan" && status !== "unreachable";
          const items: ContextMenuItem[] = [
            {
              label: "Open in Finder",
              onClick: () => {
                api.revealInFinder(p.original_path).catch((e) => {
                  setToast(`Couldn't reveal: ${e}`);
                });
              },
            },
            ...(canOpenInConfig
              ? [
                  {
                    label: "Open in Config",
                    onClick: () => {
                      // Right-click on a row → select it + flip the
                      // shell's tab to Config. Everything happens in
                      // this section — no cross-section hop needed.
                      setSelectedPath(p.original_path);
                      setProjectTab("config");
                    },
                  } satisfies ContextMenuItem,
                ]
              : []),
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

function ProjectsListFilterBar({
  nameFilter,
  onNameFilter,
  filter,
  onFilter,
  counts,
  loading,
}: {
  nameFilter: string;
  onNameFilter: (v: string) => void;
  filter: ProjectFilter;
  onFilter: (next: ProjectFilter) => void;
  counts: { all: number; orphan: number; unreachable: number; empty: number };
  loading: boolean;
}) {
  const chips: { id: ProjectFilter; label: string; n: number }[] = [
    { id: "all", label: "All", n: counts.all },
    { id: "orphan", label: "Missing", n: counts.orphan },
    { id: "unreachable", label: "Offline", n: counts.unreachable },
    { id: "empty", label: "Empty", n: counts.empty },
  ];
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-6)",
        padding: "var(--sp-10) var(--sp-12)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        flexShrink: 0,
      }}
    >
      <Input
        glyph={NF.search}
        placeholder="Filter by name or path"
        value={nameFilter}
        onChange={(e) => onNameFilter(e.target.value)}
        aria-label="Filter projects by name or path"
      />
      <div
        role="tablist"
        aria-label="Project status filter"
        style={{
          display: "flex",
          flexWrap: "wrap",
          gap: "var(--sp-4)",
        }}
      >
        {chips.map((c) => {
          const active = filter === c.id;
          // Hide status chips that have a 0 count and aren't the
          // currently-selected filter — cuts visual clutter on the
          // common "everything reachable" case.
          if (!active && c.id !== "all" && c.n === 0) return null;
          return (
            <button
              key={c.id}
              type="button"
              role="tab"
              aria-selected={active}
              onClick={() => onFilter(c.id)}
              className="pm-focus"
              style={{
                padding: "var(--sp-2) var(--sp-8)",
                fontSize: "var(--fs-2xs)",
                fontWeight: 500,
                letterSpacing: "var(--ls-wide)",
                textTransform: "uppercase",
                color: active ? "var(--fg)" : "var(--fg-muted)",
                background: active ? "var(--bg-raised)" : "transparent",
                border: `var(--bw-hair) solid ${active ? "var(--line-strong)" : "var(--line)"}`,
                borderRadius: "var(--r-1)",
                cursor: "pointer",
              }}
            >
              {c.label} · {c.n}
            </button>
          );
        })}
        {loading && (
          <span
            style={{
              marginLeft: "auto",
              fontSize: "var(--fs-2xs)",
              color: "var(--fg-faint)",
            }}
          >
            …
          </span>
        )}
      </div>
    </div>
  );
}

function ProjectTabBar({
  active,
  onChange,
}: {
  active: "sessions" | "config";
  onChange: (next: "sessions" | "config") => void;
}) {
  return (
    <div
      role="tablist"
      aria-label="Project view"
      style={{
        display: "flex",
        gap: "var(--sp-6)",
        padding: "var(--sp-8) var(--sp-12)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-sunken)",
        flexShrink: 0,
      }}
    >
      <SectionTab
        id="project-tab-sessions"
        panelId="project-panel-sessions"
        label="Sessions"
        active={active === "sessions"}
        onSelect={() => onChange("sessions")}
      />
      <SectionTab
        id="project-tab-config"
        panelId="project-panel-config"
        label="Config"
        active={active === "config"}
        onSelect={() => onChange("config")}
      />
    </div>
  );
}
