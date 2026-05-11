import type { MouseEvent } from "react";
import type { ProjectInfo } from "../../types";
import { classifyProject } from "./projectStatus";
import { formatRelativeTime, formatSize } from "./format";
import { formatUsd } from "../../costs";

/**
 * Mirror of `ProjectsSection::normalizePath` — the cost-map producer
 * normalizes its keys, so the consumer has to too. Both copies must
 * stay byte-for-byte equivalent. Kept in-file rather than promoted to
 * a util module because these are the only two call sites and a util
 * adds an import-graph node for a four-line function.
 */
function normalizePath(p: string): string {
  let s = p;
  while (
    s.length > 1 &&
    (s.endsWith("/") || s.endsWith("\\")) &&
    !/^[A-Za-z]:[/\\]$/.test(s)
  ) {
    s = s.slice(0, -1);
  }
  return s;
}

/**
 * Compact project list for the Projects section's left rail. Mirrors
 * the Global section's tree-column layout: narrow, scrollable,
 * selection-driven. Each row shows the project name on top and a
 * single metadata subtitle so the rail stays the same width as
 * Global's `--config-tree-width`.
 *
 * The bulky column layout (sessions / size / status / last-touched)
 * that lived in the previous ProjectsTable is not reproduced here —
 * in a narrow list you can only scan one axis, and the primary
 * workflow is "pick a project, then drill in," not "compare 50
 * projects at a glance." Users that genuinely want tabular comparison
 * can still use the Maintenance view.
 */
export function ProjectsList({
  projects,
  selectedPath,
  onSelect,
  onContextMenu,
  costByPath,
}: {
  projects: ProjectInfo[];
  selectedPath: string | null;
  onSelect: (path: string) => void;
  onContextMenu?: (e: MouseEvent, project: ProjectInfo) => void;
  /**
   * Hypothetical API-rate cost per project, keyed by `original_path`.
   * `undefined` (key missing) = not yet computed → render no cost.
   * `null` = project has sessions but every model was unpriced →
   * render nothing rather than $0.00 to avoid misleading the eye.
   * Number = sum across all sessions in this project.
   */
  costByPath?: Map<string, number | null>;
}) {
  if (projects.length === 0) {
    return (
      <div
        style={{
          padding: "var(--sp-20)",
          color: "var(--fg-faint)",
          fontSize: "var(--fs-sm)",
          textAlign: "center",
        }}
      >
        No projects match.
      </div>
    );
  }
  return (
    <ul
      role="listbox"
      aria-label="Projects"
      style={{
        listStyle: "none",
        margin: 0,
        padding: "var(--sp-4) 0",
        overflowY: "auto",
        flex: 1,
        minHeight: 0,
      }}
    >
      {projects.map((p) => (
        <ProjectRow
          key={p.sanitized_name}
          project={p}
          selected={p.original_path === selectedPath}
          onSelect={onSelect}
          onContextMenu={onContextMenu}
          cost={costByPath?.get(normalizePath(p.original_path))}
        />
      ))}
    </ul>
  );
}

function ProjectRow({
  project,
  selected,
  onSelect,
  onContextMenu,
  cost,
}: {
  project: ProjectInfo;
  selected: boolean;
  onSelect: (path: string) => void;
  onContextMenu?: (e: MouseEvent, project: ProjectInfo) => void;
  cost: number | null | undefined;
}) {
  const status = classifyProject(project);
  const name =
    project.original_path.split(/[/\\]/).filter(Boolean).pop() ??
    project.sanitized_name;
  const lastTouched =
    project.last_modified_ms != null
      ? formatRelativeTime(project.last_modified_ms)
      : null;
  // Secondary subtitle assembles only the non-null facts. `render-if-
  // nonzero` per design guide — a 0-session, 0-byte row would look
  // like "· · ·".
  const parts: string[] = [];
  if (project.session_count > 0) {
    parts.push(
      `${project.session_count} session${project.session_count === 1 ? "" : "s"}`,
    );
  }
  if (project.total_size_bytes > 0) {
    parts.push(formatSize(project.total_size_bytes));
  }
  if (lastTouched) parts.push(lastTouched);
  if (typeof cost === "number" && cost > 0) parts.push(formatUsd(cost));
  const subtitle = parts.join(" · ");

  return (
    <li
      role="option"
      aria-selected={selected}
      tabIndex={selected ? 0 : -1}
      onClick={() => onSelect(project.original_path)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect(project.original_path);
        }
      }}
      onContextMenu={(e) => onContextMenu?.(e, project)}
      title={project.original_path}
      className="pm-focus"
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-2)",
        padding: "var(--sp-8) var(--sp-12)",
        cursor: "pointer",
        background: selected ? "var(--bg-active)" : "transparent",
        color: selected ? "var(--accent-ink)" : "var(--fg)",
        borderLeft: selected
          ? "2px solid var(--accent-border)"
          : "2px solid transparent",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-6)",
          fontSize: "var(--fs-sm)",
          fontWeight: 500,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
          minWidth: 0,
        }}
      >
        <span
          style={{
            flex: 1,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {name}
        </span>
        {status === "orphan" && (
          <span
            className="project-tag orphan"
            title="source directory does not exist"
          >
            orphan
          </span>
        )}
        {status === "unreachable" && (
          <span
            className="project-tag unreachable"
            title="source on an unmounted volume or permission-denied path"
          >
            offline
          </span>
        )}
        {status === "empty" && (
          <span
            className="project-tag empty"
            title="no sessions, no memory files"
          >
            empty
          </span>
        )}
      </div>
      {subtitle && (
        <div
          style={{
            fontSize: "var(--fs-2xs)",
            color: selected ? "var(--accent-ink)" : "var(--fg-faint)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {subtitle}
        </div>
      )}
    </li>
  );
}
