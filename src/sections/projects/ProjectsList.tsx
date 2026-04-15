import { Folder, Warning } from "@phosphor-icons/react";
import type { ProjectInfo } from "../../types";

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

/**
 * Left-pane project list. Orphans (no source dir on disk) get a
 * warning icon and muted styling so users can triage stale entries.
 * Click selects; the right pane renders detail.
 */
export function ProjectsList({
  projects,
  selectedPath,
  onSelect,
}: {
  projects: ProjectInfo[];
  selectedPath: string | null;
  onSelect: (path: string) => void;
}) {
  if (projects.length === 0) {
    return (
      <aside className="sidebar projects-sidebar">
        <div className="sidebar-header">
          <div className="sidebar-title">Projects</div>
        </div>
        <div className="empty small muted">
          <Folder size={28} weight="thin" />
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
      <ul className="sidebar-list" role="listbox" aria-label="Projects">
        {projects.map((p) => {
          const isActive = p.original_path === selectedPath;
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
                {p.is_orphan && (
                  <Warning
                    className="warn"
                    aria-label="orphan — source dir missing"
                  />
                )}
              </div>
            </li>
          );
        })}
      </ul>
    </aside>
  );
}
