import { useMemo, type ReactNode } from "react";
import { Icon } from "../../components/Icon";
import type { ProjectInfo } from "../../types";
import { classifyProject, type ProjectStatus } from "./projectStatus";

import { formatSize } from "./format";

function formatRelative(ms: number | null): string | null {
  if (ms === null) return null;
  const diff = Date.now() - ms;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

// Zero-state metadata filter (design-principles.md §8 / design-patterns.md).
// Dropping zero sessions stops the `0 sessions · 33.8 MB · 21d ago` noise.
function formatRowMeta(p: ProjectInfo): string | null {
  const parts = [
    p.total_size_bytes > 0 ? formatSize(p.total_size_bytes) : null,
    formatRelative(p.last_modified_ms),
    p.session_count > 0
      ? `${p.session_count} session${p.session_count === 1 ? "" : "s"}`
      : null,
  ].filter((v): v is string => v !== null);
  return parts.length === 0 ? null : parts.join(" · ");
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
  onContextMenu,
  filter,
  onFilterChange,
  segmentedControl,
}: {
  projects: ProjectInfo[];
  selectedPath: string | null;
  onSelect: (path: string) => void;
  onContextMenu?: (e: React.MouseEvent, p: ProjectInfo) => void;
  filter: ProjectFilter;
  onFilterChange: (next: ProjectFilter) => void;
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
          <Icon name="folder" size={28} />
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
        role="toolbar"
        aria-label="Filter projects by status"
      >
        <FilterChip
          current={filter}
          value="all"
          icon={<Icon name="list" size={14} />}
          label="All"
          tooltip="All projects"
          count={projects.length}
          onClick={onFilterChange}
        />
        <FilterChip
          current={filter}
          value="orphan"
          icon={<Icon name="unlink" size={14} />}
          label="Orphan"
          tooltip="Orphan — source dir missing"
          count={counts.orphan}
          onClick={onFilterChange}
        />
        <FilterChip
          current={filter}
          value="unreachable"
          icon={<Icon name="wifi-off" size={14} />}
          label="Offline"
          tooltip="Unreachable — volume unmounted or permission-denied"
          count={counts.unreachable}
          onClick={onFilterChange}
        />
        <FilterChip
          current={filter}
          value="empty"
          icon={<Icon name="circle-dashed" size={14} />}
          label="Empty"
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
                onContextMenu={
                  onContextMenu ? (e) => onContextMenu(e, p) : undefined
                }
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onSelect(p.original_path);
                  }
                }}
              >
                <div className="sidebar-item-row">
                  <div className="sidebar-item-text">
                    <span className="sidebar-item-name" title={p.original_path}>
                      {p.original_path.split("/").filter(Boolean).pop() ??
                        p.sanitized_name}
                    </span>
                    {(() => {
                      const meta = formatRowMeta(p);
                      return meta ? (
                        <div className="sidebar-item-meta muted">{meta}</div>
                      ) : null;
                    })()}
                  </div>
                  <StatusBadge status={status} />
                </div>
              </li>
            );
          })}
        </ul>
      )}

    </aside>
  );
}

function FilterChip({
  current,
  value,
  icon,
  label,
  tooltip,
  count,
  onClick,
}: {
  current: ProjectFilter;
  value: ProjectFilter;
  icon: ReactNode;
  label: string;
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
      aria-pressed={selected}
      aria-label={tooltip}
      title={tooltip}
      className={`project-filter-chip${selected ? " selected" : ""}`}
      onClick={() => onClick(value)}
    >
      {icon}
      <span className="project-filter-chip-text">
        <span className="project-filter-label">{label}</span>
        {showCount && <span className="project-filter-count">{count}</span>}
      </span>
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
