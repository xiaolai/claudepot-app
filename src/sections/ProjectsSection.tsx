import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import { useOperations } from "../hooks/useOperations";
import { useGlobalShortcuts } from "../hooks/useGlobalShortcuts";
import { useCompactHeader } from "../hooks/useWindowWidth";
import type { MoveArgs, OrphanedProject, ProjectInfo } from "../types";
import { ContextMenu, type ContextMenuItem } from "../components/ContextMenu";
import { classifyProject } from "./projects/projectStatus";
import type { ProjectStatus } from "./projects/projectStatus";
import { Button } from "../components/primitives/Button";
import { IconButton } from "../components/primitives/IconButton";
import { Input } from "../components/primitives/Input";
import { useAppState } from "../providers/AppStateProvider";
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
import { basename as projectBasename } from "./projects/format";
import { MemoryPane } from "./projects/MemoryPane";
import { SectionTab } from "./sessions/components/SectionTab";
import { RenameProjectModal } from "./projects/RenameProjectModal";
import { RemoveProjectModal } from "./projects/RemoveProjectModal";
import { ExportProjectModal } from "./projects/ExportProjectModal";
import { ImportBundleModal } from "./projects/ImportBundleModal";
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
  pendingProjectPath,
  pendingSessionPath,
  onPendingConsumed,
}: {
  subRoute: string | null;
  onSubRouteChange: (next: string | null) => void;
  /** Project cwd to open on next mount/refresh, e.g. from a card
   *  click in the Activity surface. Cleared via `onPendingConsumed`
   *  the first paint that successfully selects the project. */
  pendingProjectPath?: string | null;
  /** Session jsonl path to open inside ProjectDetail once the
   *  project resolves. Consumed alongside `pendingProjectPath`. */
  pendingSessionPath?: string | null;
  /** Fired when the pending pair has been consumed so the parent
   *  can clear state and avoid re-applying on every prop change. */
  onPendingConsumed?: () => void;
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
  const [projectTab, setProjectTab] = useState<
    "sessions" | "config" | "memory"
  >("sessions");
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

  // Cross-section navigation hand-off. Two callers feed in here:
  //   1. Activity surface card click — supplies `{projectPath, sessionPath}`
  //      (cwd carried on the card itself; clean lookup).
  //   2. cp-goto-session, openLiveSession, palette goto — supply
  //      only `sessionPath`. We derive the project by extracting the
  //      slug (the second-to-last path segment of the .jsonl) and
  //      matching it against `project.sanitized_name`. The slug
  //      encodes the cwd via `sanitize_path`; matching by that name
  //      is what the GUI uses everywhere else.
  // We key the "did I already consume this?" gate by the pending
  // payload itself (project + session) so a SECOND deep-link while
  // ProjectsSection stays mounted re-fires the resolver — earlier
  // versions used a once-per-mount boolean and silently dropped
  // every navigation after the first. The gate is also released
  // once the consumer would resolve to nothing (e.g. project list
  // hasn't arrived yet, or the project doesn't exist anymore) so
  // the parent doesn't have to clear-and-reset state on a no-op.
  const lastConsumedRef = useRef<string | null>(null);
  useEffect(() => {
    if (!pendingProjectPath && !pendingSessionPath) return;
    if (loading) return;
    const payloadKey = `${pendingProjectPath ?? ""}::${pendingSessionPath ?? ""}`;
    if (lastConsumedRef.current === payloadKey) return;
    let resolvedProject: ProjectInfo | null = null;
    if (pendingProjectPath) {
      const wanted = pendingProjectPath.toLowerCase();
      resolvedProject =
        projects.find((p) => p.original_path.toLowerCase() === wanted) ?? null;
    }
    if (!resolvedProject && pendingSessionPath) {
      // Derive slug: the parent directory of the .jsonl file under
      // `<cc_config_dir>/projects/<slug>/<sid>.jsonl`.
      const parts = pendingSessionPath.split(/[\\/]/);
      const slug = parts[parts.length - 2] ?? null;
      if (slug) {
        resolvedProject =
          projects.find((p) => p.sanitized_name === slug) ?? null;
      }
    }
    // No matching project (orphan, deleted, or project list still
    // syncing): do NOT mark consumed yet. The parent keeps the
    // pending state and we retry once `projects` updates. If the
    // project genuinely no longer exists the user lands on the
    // empty-pane state on next render — which is the correct
    // "graceful degradation" outcome rather than silently dropping
    // the request.
    if (!resolvedProject) return;
    setSelectedPath(resolvedProject.original_path);
    if (pendingSessionPath) setOpenedSessionPath(pendingSessionPath);
    lastConsumedRef.current = payloadKey;
    onPendingConsumed?.();
  }, [
    loading,
    pendingProjectPath,
    pendingSessionPath,
    projects,
    onPendingConsumed,
  ]);
  const [renameTarget, setRenameTarget] = useState<string | null>(null);
  const [removeTarget, setRemoveTarget] = useState<string | null>(null);
  // Project migrate UI state (spec §12.2). Export is row-keyed (we
  // know which project to bundle); import is global (the user picks
  // a bundle file from anywhere on disk).
  const [exportTarget, setExportTarget] = useState<string | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [filter, setFilter] = useState<ProjectFilter>("all");
  const [nameFilter, setNameFilter] = useState("");
  const { pushToast } = useAppState();
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
  // Audit T2 H#1: chip-state `filter` was set but never consumed before
  // rendering `ProjectsList`. Apply status filter on top of the
  // name-filtered set so orphan/offline/empty chips actually narrow
  // the rail.
  const shownProjects = useMemo(() => {
    if (filter === "all") return filteredByName;
    return filteredByName.filter(
      (p) => classifyProject(p) === (filter as ProjectStatus),
    );
  }, [filteredByName, filter]);

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
    const narrowed =
      (nameFilter.trim() !== "" || filter !== "all") &&
      shownProjects.length !== total;
    if (narrowed) {
      return `${shownProjects.length} of ${total} project${total === 1 ? "" : "s"} shown`;
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
                glyph={NF.download}
                onClick={() => setImportOpen(true)}
                title="Import bundle"
                aria-label="Import bundle"
              />
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
                glyph={NF.download}
                glyphColor="var(--fg-muted)"
                onClick={() => setImportOpen(true)}
                title="Import a *.claudepot.tar.zst bundle"
              >
                Import bundle
              </Button>
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
              projects={shownProjects}
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
                    minWidth: 0,
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
                        onError={(msg) => pushToast("error", msg)}
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
                        onError={(msg) => pushToast("error", msg)}
                        onOpenMaintenance={() => onSubRouteChange("maintenance")}
                        onOpenInConfig={() => setProjectTab("config")}
                        onOpenSession={(p) => setOpenedSessionPath(p)}
                      />
                    )
                  ) : projectTab === "config" ? (
                    <ConfigSection
                      key={`config:${selectedPath}`}
                      subRoute={configSubRoute}
                      onSubRouteChange={setConfigSubRoute}
                      forcedAnchor={{ kind: "folder", path: selectedPath }}
                    />
                  ) : (
                    <MemoryPane
                      key={`memory:${selectedPath}`}
                      projectRoot={selectedPath}
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
              // Cross-platform basename — Windows paths contain `\`
              // separators (audit 2026-05 #12).
              const base = (p: string) => projectBasename(p) || p;
              openOpModal({
                opId,
                title: `Renaming ${base(args.oldPath)} → ${base(args.newPath)}`,
                onComplete: () => {
                  pushToast("info", "Rename complete.");
                  refresh();
                },
                onError: (detail) => {
                  pushToast("error", `Rename failed: ${detail ?? "unknown"}`);
                  refresh();
                },
              });
            } catch (e) {
              pushToast("error", `Couldn't start rename: ${e}`);
            }
          }}
        />
      )}

      {removeTarget && (
        <RemoveProjectModal
          target={removeTarget}
          onClose={() => setRemoveTarget(null)}
          onCompleted={(result) => {
            setRemoveTarget(null);
            // Drop the selection if we just trashed the open project,
            // so the detail pane doesn't render a stale ghost.
            setSelectedPath((prev) =>
              prev && result.original_path === prev ? null : prev,
            );
            pushToast(
              "info",
              `Removed ${result.slug}. Restore via project trash if needed.`,
            );
            refresh();
          }}
          onError={(msg) => {
            setRemoveTarget(null);
            pushToast("error", `Couldn't remove: ${msg}`);
          }}
        />
      )}

      {exportTarget && (
        <ExportProjectModal
          cwd={exportTarget}
          onClose={() => setExportTarget(null)}
          onCompleted={(receipt) => {
            setExportTarget(null);
            pushToast(
              "info",
              `Exported ${receipt.projectCount} project(s) to ${receipt.bundlePath}`,
            );
          }}
          onError={(msg) => {
            setExportTarget(null);
            pushToast("error", `Export failed: ${msg}`);
          }}
        />
      )}

      {importOpen && (
        <ImportBundleModal
          onClose={() => setImportOpen(false)}
          onCompleted={(receipt) => {
            setImportOpen(false);
            const verb = receipt.dryRun ? "Plan" : "Imported";
            pushToast(
              "info",
              `${verb}: ${receipt.projectsImported.length} project(s)${
                receipt.projectsRefused.length
                  ? ` (${receipt.projectsRefused.length} refused)`
                  : ""
              }`,
            );
            // Re-fetch the list so newly-imported slugs show up.
            if (!receipt.dryRun) refresh();
          }}
          onError={(msg) => {
            setImportOpen(false);
            pushToast("error", `Import failed: ${msg}`);
          }}
        />
      )}

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
                  pushToast("error", `Couldn't reveal: ${e}`);
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
              label: "Export bundle…",
              onClick: () => setExportTarget(p.original_path),
            },
            {
              label: "Remove project…",
              onClick: () =>
                setRemoveTarget(p.original_path || p.sanitized_name),
            },
            {
              label: "Clean orphans…",
              onClick: () => onSubRouteChange("maintenance"),
            },
            { label: "", separator: true, onClick: () => {} },
            {
              label: "Copy path",
              onClick: () => {
                navigator.clipboard
                  .writeText(p.original_path)
                  .then(() => pushToast("info", "Copied path."))
                  .catch((e) =>
                    pushToast("error", `Couldn't copy path: ${e}`),
                  );
              },
            },
            {
              label: "Copy key",
              onClick: () => {
                navigator.clipboard
                  .writeText(p.sanitized_name)
                  .then(() => pushToast("info", "Copied key."))
                  .catch((e) =>
                    pushToast("error", `Couldn't copy key: ${e}`),
                  );
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

type ProjectTab = "sessions" | "config" | "memory";

function ProjectTabBar({
  active,
  onChange,
}: {
  active: ProjectTab;
  onChange: (next: ProjectTab) => void;
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
      <SectionTab
        id="project-tab-memory"
        panelId="project-panel-memory"
        label="Memory"
        active={active === "memory"}
        onSelect={() => onChange("memory")}
      />
    </div>
  );
}
