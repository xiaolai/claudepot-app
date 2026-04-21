import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api";
import { useOperations } from "../hooks/useOperations";
import { useGlobalShortcuts } from "../hooks/useGlobalShortcuts";
import { useCompactHeader, useSplitView } from "../hooks/useWindowWidth";
import type { MoveArgs, OrphanedProject, ProjectInfo } from "../types";
import { ContextMenu, type ContextMenuItem } from "../components/ContextMenu";
import { Button } from "../components/primitives/Button";
import { Glyph } from "../components/primitives/Glyph";
import { IconButton } from "../components/primitives/IconButton";
import { Input } from "../components/primitives/Input";
import { Toast } from "../components/primitives/Toast";
import { NF } from "../icons";
import { ScreenHeader } from "../shell/ScreenHeader";
import {
  ProjectsTable,
  countByStatus,
  type ProjectFilter,
} from "./projects/ProjectsTable";
import { ProjectDetail } from "./projects/ProjectDetail";
import { RenameProjectModal } from "./projects/RenameProjectModal";
import { MaintenanceView } from "./projects/MaintenanceView";
import { OrphanBanner } from "./projects/OrphanBanner";
import { AdoptOrphansModal } from "./projects/AdoptOrphansModal";

// "orphan" here = ProjectInfo.is_orphan — source directory no longer
// exists on a reachable filesystem. Distinct from the OrphanBanner's
// transcript-level orphans (slugs whose internal cwd is gone), which
// is why the chip reads "Source gone" rather than "Orphan" — avoid
// one word for two concepts.
const SEG_OPTIONS: { id: "all" | "orphan" | "unreachable" | "empty"; label: string }[] = [
  { id: "all", label: "All" },
  { id: "orphan", label: "Source gone" },
  { id: "unreachable", label: "Offline" },
  { id: "empty", label: "Empty" },
];

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

  const compact = useCompactHeader();
  // When the window is too narrow to give both the table and the 420px
  // detail aside enough room, collapse to a single-pane master/detail
  // flow. Selecting a project replaces the table with the detail view;
  // the detail's Back button restores the table.
  const splitView = useSplitView();
  const showDetail = selectedPath !== null;
  const showTable = splitView || selectedPath === null;

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
                title="Refresh (⌘R)"
              >
                Refresh
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

      {/* Filter bar — only when the table is showing. In single-pane
          detail view we hide it so the detail has the full column. */}
      {showTable && (
      <div
        style={{
          padding: "var(--sp-14) var(--sp-32)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          display: "flex",
          flexWrap: "wrap",
          gap: "var(--sp-12)",
          alignItems: "center",
          background: "var(--bg)",
          flexShrink: 0,
        }}
      >
        <Input
          glyph={NF.search}
          placeholder="Filter by name or path"
          value={nameFilter}
          onChange={(e) => setNameFilter(e.target.value)}
          style={{
            // Grow into available space; shrink to a sane minimum when
            // the segmented chips and "Refreshing…" share the row.
            flex: "1 1 var(--filter-input-width)",
            minWidth: "var(--filter-input-min)",
            maxWidth: "var(--filter-input-width)",
          }}
          aria-label="Filter projects by name or path"
        />

        <div
          role="tablist"
          style={{
            display: "flex",
            gap: "var(--sp-2)",
            padding: "var(--sp-2)",
            background: "var(--bg-sunken)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
          }}
        >
          {SEG_OPTIONS.map((opt) => {
            const current = filter === opt.id;
            const count =
              opt.id === "all"
                ? projects.length
                : counts[opt.id as keyof typeof counts];
            return (
              <button
                key={opt.id}
                type="button"
                role="tab"
                aria-selected={current}
                onClick={() => setFilter(opt.id)}
                style={{
                  padding: "var(--sp-4) var(--sp-10)",
                  fontSize: "var(--fs-xs)",
                  fontWeight: 500,
                  color: current ? "var(--fg)" : "var(--fg-muted)",
                  background: current
                    ? "var(--bg-raised)"
                    : "transparent",
                  border: current
                    ? "var(--bw-hair) solid var(--line)"
                    : "var(--bw-hair) solid transparent",
                  borderRadius: "var(--r-1)",
                  letterSpacing: "var(--ls-wide)",
                  textTransform: "uppercase",
                  cursor: "pointer",
                }}
              >
                {opt.label} · {count}
              </button>
            );
          })}
        </div>

        <div style={{ flex: 1 }} />
        {loading && projects.length > 0 && (
          <span
            style={{
              fontSize: "var(--fs-xs)",
              color: "var(--fg-faint)",
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-6)",
            }}
          >
            <Glyph g={NF.refresh} />
            Refreshing…
          </span>
        )}
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
        <div style={{ display: "flex", minHeight: 0, flex: 1 }}>
          {showTable && (
            <div
              style={{
                flex: 1,
                minWidth: 0,
                overflow: "auto",
                display: "flex",
                flexDirection: "column",
              }}
            >
              <ProjectsTable
                projects={filteredByName}
                filter={filter}
                selectedPath={selectedPath}
                onSelect={setSelectedPath}
                onContextMenu={handleContextMenu}
              />
            </div>
          )}

          {showDetail && selectedPath && (
            <aside
              style={{
                // Split mode: fixed 420 px so the table keeps a stable
                // width. Single-pane (narrow window): detail replaces
                // the table and takes the whole content column.
                width: splitView ? "var(--project-detail-width)" : "100%",
                flex: splitView ? "0 0 auto" : "1 1 auto",
                flexShrink: splitView ? 0 : 1,
                borderLeft: splitView
                  ? "var(--bw-hair) solid var(--line)"
                  : "none",
                background: splitView ? "var(--bg-sunken)" : "var(--bg)",
                overflow: "auto",
                minWidth: 0,
              }}
            >
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
                onBack={splitView ? undefined : () => setSelectedPath(null)}
              />
            </aside>
          )}
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
