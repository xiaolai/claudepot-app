import { useMemo, type ReactNode } from "react";
import { Folder, Trash2, List, Unlink, WifiOff, CircleDashed as CircleDashedIcon } from "lucide-react";
import type { ProjectInfo } from "../../types";
import { classifyProject, type ProjectStatus } from "./projectStatus";

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function formatRelative(ms: number | null): string {
  if (ms === null) return "—";
  const diff = Date.now() - ms;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

export type ProjectFilter = "all" | "orphan" | "unreachable" | "empty";

/**
 * Left-pane project list. Renders a filter chip row, the project
 * list itself (each row carries its status badge), and a "Clean
 * orphans…" action at the bottom.
 *
 * Status badges are the only surface that distinguishes:
 *   * orphan       — amber, will be cleaned by the backend.
 *   * unreachable  — neutral/grey, NOT cleaned (mount may come back).
 *   * empty        — muted, stub dir with nothing to lose.
 * "alive" projects render no badge at all.
 */
export function ProjectsList({
  projects,
  selectedPath,
  onSelect,
  filter,
  onFilterChange,
  onClean,
  cleanCount,
  segmentedControl,
}: {
  projects: ProjectInfo[];
  selectedPath: string | null;
  onSelect: (path: string) => void;
  filter: ProjectFilter;
  onFilterChange: (next: ProjectFilter) => void;
  onClean: () => void;
  cleanCount: number;
  /** Optional segmented control rendered below the sidebar header. */
  segmentedControl?: ReactNode;
}) {
  const { counts, filtered } = useMemo(() => {
    const counts: Record<ProjectStatus, number> = {
      alive: 0,
      orphan: 0,
      unreachable: 0,
      empty: 0,
    };
    for (const p of projects) counts[classifyProject(p)] += 1;

    const filtered =
      filter === "all"
        ? projects
        : projects.filter((p) => classifyProject(p) === filter);
    return { counts, filtered };
  }, [projects, filter]);

  if (projects.length === 0) {
    return (
      <aside className="sidebar projects-sidebar">
        <div className="sidebar-header">
          <div className="sidebar-title">Projects</div>
        </div>
        <div className="empty small muted">
          <Folder size={28} strokeWidth={1} />
          <p>No CC projects yet.</p>
        </div>
      </aside>
    );
  }
  return (
    <aside className="sidebar projects-sidebar">
      <div className="sidebar-header">
        <div className="sidebar-title">Projects</div>
        <div className="sidebar-item-meta muted">{projects.length}</div>
      </div>

      {segmentedControl && (
        <div className="sidebar-segmented-slot">
          {segmentedControl}
        </div>
      )}

      <div
        className="project-filter-row"
        role="tablist"
        aria-label="Filter projects by status"
      >
        <FilterChip
          current={filter}
          value="all"
          icon={<List size={14} />}
          tooltip="All projects"
          count={projects.length}
          onClick={onFilterChange}
        />
        <FilterChip
          current={filter}
          value="orphan"
          icon={<Unlink size={14} />}
          tooltip="Orphan — source dir missing"
          count={counts.orphan}
          onClick={onFilterChange}
        />
        <FilterChip
          current={filter}
          value="unreachable"
          icon={<WifiOff size={14} />}
          tooltip="Unreachable — volume unmounted"
          count={counts.unreachable}
          onClick={onFilterChange}
        />
        <FilterChip
          current={filter}
          value="empty"
          icon={<CircleDashedIcon size={14} />}
          tooltip="Empty — no sessions or memory"
          count={counts.empty}
          onClick={onFilterChange}
        />
      </div>

      {filtered.length === 0 ? (
        <div className="empty small muted">
          <p>No projects in this filter.</p>
        </div>
      ) : (
        <ul className="sidebar-list" role="listbox" aria-label="Projects">
          {filtered.map((p) => {
            const isActive = p.original_path === selectedPath;
            const status = classifyProject(p);
            return (
              <li
                key={p.sanitized_name}
                role="option"
                aria-selected={isActive}
                className={`sidebar-item${isActive ? " active" : ""}`}
                tabIndex={0}
                onClick={() => onSelect(p.original_path)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onSelect(p.original_path);
                  }
                }}
              >
                <div className="sidebar-item-row">
                  <div className="sidebar-item-text">
                    <strong title={p.original_path}>
                      {p.original_path.split("/").filter(Boolean).pop() ??
                        p.sanitized_name}
                    </strong>
                    <div className="sidebar-item-meta muted">
                      {p.session_count} session{p.session_count === 1 ? "" : "s"} ·{" "}
                      {formatSize(p.total_size_bytes)} ·{" "}
                      {formatRelative(p.last_modified_ms)}
                    </div>
                  </div>
                  <StatusBadge status={status} />
                </div>
              </li>
            );
          })}
        </ul>
      )}

      <div className="project-sidebar-actions">
        <button
          type="button"
          className="sidebar-action"
          onClick={onClean}
          disabled={cleanCount === 0}
          title={
            cleanCount === 0
              ? "No orphans or empty projects to clean"
              : `Review and remove ${cleanCount} project${cleanCount === 1 ? "" : "s"}`
          }
        >
          <Trash2 size={14} />
          <span>
            Clean{cleanCount > 0 ? ` (${cleanCount})` : ""}…
          </span>
        </button>
      </div>
    </aside>
  );
}

function FilterChip({
  current,
  value,
  icon,
  tooltip,
  count,
  onClick,
}: {
  current: ProjectFilter;
  value: ProjectFilter;
  icon: ReactNode;
  tooltip: string;
  count: number;
  onClick: (next: ProjectFilter) => void;
}) {
  const selected = current === value;
  // Zero counts on non-selected chips are noise — hide them. `All`
  // always shows its total (that's its whole point).
  const showCount = value === "all" || selected || count > 0;
  return (
    <button
      type="button"
      role="tab"
      aria-selected={selected}
      aria-label={tooltip}
      title={tooltip}
      className={`project-filter-chip${selected ? " selected" : ""}`}
      onClick={() => onClick(value)}
    >
      {icon}
      {showCount && <span className="project-filter-count">{count}</span>}
    </button>
  );
}

const STATUS_LABEL: Record<Exclude<ProjectStatus, "alive">, string> = {
  orphan: "orphan — source dir missing",
  unreachable: "unreachable — mount the source volume to re-check",
  empty: "empty — CC project dir has no content",
};

function StatusBadge({ status }: { status: ProjectStatus }) {
  if (status === "alive") return null;
  return (
    <span
      className={`project-status-dot ${status}`}
      aria-label={STATUS_LABEL[status]}
      title={STATUS_LABEL[status]}
    />
  );
}
