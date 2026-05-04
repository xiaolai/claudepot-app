import { type MouseEvent, useMemo, useState } from "react";
import { Glyph } from "../../components/primitives/Glyph";
import { IconButton } from "../../components/primitives/IconButton";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import type { ProjectInfo } from "../../types";
import { basename, formatRelativeTime, formatSize } from "./format";
import { classifyProject, type ProjectStatus } from "./projectStatus";

export type ProjectFilter = "all" | "orphan" | "unreachable" | "empty";

/**
 * Columns a user can sort by. `path` sorts by the project basename
 * (case-insensitive) — what the user reads, not the slug. Status
 * and the leading icon aren't sortable.
 */
export type SortKey = "path" | "session_count" | "size" | "last_touched";
export type SortDir = "asc" | "desc";

// `minmax(0, *fr)` rather than bare `*fr` tracks so each column can
// shrink below intrinsic min-content. The Project name span already
// has `overflow: hidden`, but the bare tracks would otherwise let the
// row overflow when a project carries a single very long unbreakable
// path segment.
const COLS =
  "var(--sp-20) minmax(0,1.6fr) minmax(0,0.9fr) minmax(0,0.9fr) minmax(0,0.8fr) minmax(0,1fr) var(--sp-24)";

function projectBasename(p: ProjectInfo): string {
  // Cross-platform basename — pre-fix this hardcoded `/` and rendered
  // the whole Windows path as the basename (audit 2026-05 #11).
  const tail = basename(p.original_path);
  return (tail || p.sanitized_name).toLowerCase();
}

function compareProjects(a: ProjectInfo, b: ProjectInfo, key: SortKey): number {
  switch (key) {
    case "path":
      return projectBasename(a).localeCompare(projectBasename(b));
    case "session_count":
      return a.session_count - b.session_count;
    case "size":
      return Number(a.total_size_bytes) - Number(b.total_size_bytes);
    case "last_touched":
      // null last_modified sorts to the bottom ascending.
      return (a.last_modified_ms ?? 0) - (b.last_modified_ms ?? 0);
  }
}

/**
 * Full-width project table. Replaces the previous sidebar-style
 * ProjectsList. Rows expand on hover (`--bg-hover`) and reveal a
 * chevron at the right; clicking selects the project and pins it in
 * the adjacent detail pane.
 *
 * The filter value filters rows by `classifyProject` status. Zero-
 * meta fields drop from the meta line — no "0 sessions · …" noise.
 */
export function ProjectsTable({
  projects,
  filter,
  selectedPath,
  onSelect,
  onContextMenu,
}: {
  projects: ProjectInfo[];
  filter: ProjectFilter;
  selectedPath: string | null;
  onSelect: (path: string) => void;
  onContextMenu?: (e: MouseEvent, p: ProjectInfo) => void;
}) {
  // Default: last_touched desc — freshest work on top. Clicking a
  // sortable column cycles that column through asc → desc → default.
  const [sort, setSort] = useState<{ key: SortKey; dir: SortDir }>({
    key: "last_touched",
    dir: "desc",
  });

  const toggleSort = (key: SortKey) => {
    setSort((prev) => {
      if (prev.key !== key) return { key, dir: "asc" };
      if (prev.dir === "asc") return { key, dir: "desc" };
      return { key: "last_touched", dir: "desc" };
    });
  };

  const shown = useMemo(() => {
    const filtered =
      filter === "all"
        ? projects
        : projects.filter((p) => classifyProject(p) === filter);
    const sorted = [...filtered].sort((a, b) =>
      compareProjects(a, b, sort.key),
    );
    if (sort.dir === "desc") sorted.reverse();
    return sorted;
  }, [projects, filter, sort]);

  if (projects.length === 0) {
    return (
      <EmptyRow>
        <Glyph g={NF.folder} size="var(--sp-24)" color="var(--fg-ghost)" />
        <div>No CC projects yet.</div>
        <div
          style={{
            marginTop: "var(--sp-4)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-faint)",
          }}
        >
          Run <code style={{ fontFamily: "var(--font)" }}>claude</code>{" "}
          in any directory to create one.
        </div>
      </EmptyRow>
    );
  }

  return (
    <>
      {/* table header */}
      <div
        role="row"
        style={{
          display: "grid",
          gridTemplateColumns: COLS,
          padding: "var(--sp-8) var(--sp-32)",
          fontSize: "var(--fs-xs)",
          color: "var(--fg-faint)",
          letterSpacing: "var(--ls-wide)",
          textTransform: "uppercase",
          gap: "var(--sp-16)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          background: "var(--bg-sunken)",
          alignItems: "center",
          position: "sticky",
          top: 0,
          zIndex: "var(--z-sticky)" as unknown as number,
        }}
      >
        <span />
        <SortHeader
          label="Project"
          col="path"
          currentKey={sort.key}
          currentDir={sort.dir}
          onToggle={toggleSort}
        />
        <SortHeader
          label="Sessions"
          col="session_count"
          currentKey={sort.key}
          currentDir={sort.dir}
          onToggle={toggleSort}
        />
        <SortHeader
          label="Size"
          col="size"
          currentKey={sort.key}
          currentDir={sort.dir}
          onToggle={toggleSort}
        />
        <span>Status</span>
        <SortHeader
          label="Last touched"
          col="last_touched"
          currentKey={sort.key}
          currentDir={sort.dir}
          onToggle={toggleSort}
        />
        <span />
      </div>

      {shown.length === 0 ? (
        <EmptyRow>No projects in this filter.</EmptyRow>
      ) : (
        <ul
          role="listbox"
          aria-label="Projects"
          style={{ listStyle: "none", margin: 0, padding: 0 }}
        >
          {shown.map((p) => (
            <ProjectRow
              key={p.sanitized_name}
              project={p}
              active={p.original_path === selectedPath}
              onSelect={onSelect}
              onContextMenu={onContextMenu}
            />
          ))}
        </ul>
      )}
    </>
  );
}

function ProjectRow({
  project: p,
  active,
  onSelect,
  onContextMenu,
}: {
  project: ProjectInfo;
  active: boolean;
  onSelect: (path: string) => void;
  onContextMenu?: (e: MouseEvent, p: ProjectInfo) => void;
}) {
  const [hover, setHover] = useState(false);
  const status = classifyProject(p);
  // Cross-platform basename — see projectBasename above (audit #11).
  const name = basename(p.original_path) || p.sanitized_name;

  return (
    <li
      role="option"
      aria-selected={active}
      tabIndex={0}
      onClick={() => onSelect(p.original_path)}
      onContextMenu={onContextMenu ? (e) => onContextMenu(e, p) : undefined}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect(p.original_path);
        }
      }}
      style={{
        display: "grid",
        gridTemplateColumns: COLS,
        padding: "var(--sp-12) var(--sp-32)",
        gap: "var(--sp-16)",
        alignItems: "center",
        borderBottom: "var(--bw-hair) solid var(--line)",
        background: active
          ? "var(--bg-active)"
          : hover
            ? "var(--bg-hover)"
            : "transparent",
        borderLeft: active
          ? "var(--bw-strong) solid var(--accent)"
          : "var(--bw-strong) solid transparent",
        cursor: "pointer",
        fontSize: "var(--fs-sm)",
        outline: "none",
      }}
    >
      <span aria-hidden>
        <Glyph
          g={NF.folder}
          color="var(--fg-muted)"
          style={{ fontSize: "var(--fs-md)" }}
        />
      </span>

      <div style={{ minWidth: 0, overflow: "hidden" }}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--sp-8)",
            fontSize: "var(--fs-base)",
            color: "var(--fg)",
            fontWeight: active ? 600 : 500,
            minWidth: 0,
          }}
        >
          <span
            title={p.original_path}
            style={{
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
          >
            {name}
          </span>
        </div>
        <div
          // Truncated path subtext — full string disclosed via tooltip
          // here; the canonical copy site is `ProjectDetail` (header
          // path with `<CopyButton>`), which the row click opens.
          // Per .claude/rules/path-display.md state B: no inline copy.
          title={p.original_path}
          style={{
            marginTop: "var(--sp-2)",
            color: "var(--fg-faint)",
            fontSize: "var(--fs-xs)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {p.original_path}
        </div>
      </div>

      <span
        style={{
          color: p.session_count > 0 ? "var(--fg-muted)" : "var(--fg-ghost)",
          fontVariantNumeric: "tabular-nums",
        }}
      >
        {p.session_count > 0 ? p.session_count : "—"}
      </span>

      <span
        style={{
          color:
            p.total_size_bytes > 0 ? "var(--fg-muted)" : "var(--fg-ghost)",
          fontVariantNumeric: "tabular-nums",
        }}
      >
        {p.total_size_bytes > 0 ? formatSize(p.total_size_bytes) : "—"}
      </span>

      <span>
        <StatusTag status={status} />
      </span>

      <span
        style={{
          color: "var(--fg-faint)",
          whiteSpace: "nowrap",
          overflow: "hidden",
          textOverflow: "ellipsis",
        }}
      >
        {p.last_modified_ms != null ? formatRelativeTime(p.last_modified_ms) : "—"}
      </span>

      <span
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: "var(--sp-2)",
          justifyContent: "flex-end",
        }}
      >
        {(hover || active) && onContextMenu && (
          <span
            // Stop row-select click from firing when the kebab is used.
            onClick={(e) => e.stopPropagation()}
            onMouseDown={(e) => e.stopPropagation()}
            style={{ display: "inline-flex" }}
          >
            <IconButton
              glyph={NF.ellipsis}
              size="sm"
              onClick={() => {
                const el = document.activeElement as HTMLElement | null;
                const rect = el?.getBoundingClientRect();
                onContextMenu(
                  {
                    preventDefault: () => {},
                    stopPropagation: () => {},
                    clientX: rect ? rect.right : 0,
                    clientY: rect ? rect.bottom : 0,
                  } as unknown as MouseEvent,
                  p,
                );
              }}
              title="More actions"
              aria-label={`More actions for ${name}`}
              aria-haspopup="menu"
            />
          </span>
        )}
        {(hover || active) && (
          <Glyph
            g={NF.chevronR}
            color={active ? "var(--accent)" : "var(--fg-faint)"}
            style={{ fontSize: "var(--fs-xs)" }}
          />
        )}
      </span>
    </li>
  );
}

/**
 * Clickable column header. Shows a direction arrow when this column
 * is the active sort; otherwise the column reads as plain label but
 * stays discoverable via the button semantics.
 */
function SortHeader({
  label,
  col,
  currentKey,
  currentDir,
  onToggle,
}: {
  label: string;
  col: SortKey;
  currentKey: SortKey;
  currentDir: SortDir;
  onToggle: (key: SortKey) => void;
}) {
  const active = currentKey === col;
  const aria: "ascending" | "descending" | "none" = active
    ? currentDir === "asc"
      ? "ascending"
      : "descending"
    : "none";
  return (
    <button
      type="button"
      role="columnheader"
      aria-sort={aria}
      onClick={() => onToggle(col)}
      title={`Sort by ${label.toLowerCase()}`}
      style={{
        background: "transparent",
        border: 0,
        padding: 0,
        font: "inherit",
        color: active ? "var(--fg)" : "var(--fg-faint)",
        letterSpacing: "var(--ls-wide)",
        textTransform: "uppercase",
        textAlign: "left",
        cursor: "pointer",
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-4)",
      }}
    >
      <span>{label}</span>
      {active && (
        <Glyph
          g={currentDir === "asc" ? NF.chevronU : NF.chevronD}
          color="var(--fg-muted)"
          style={{ fontSize: "var(--fs-2xs)" }}
        />
      )}
    </button>
  );
}

function StatusTag({ status }: { status: ProjectStatus }) {
  switch (status) {
    case "orphan":
      return (
        <Tag tone="warn" glyph={NF.warn} title="Source directory is missing">
          orphan
        </Tag>
      );
    case "unreachable":
      return (
        <Tag tone="neutral" title="Volume unmounted or permission denied">
          offline
        </Tag>
      );
    case "empty":
      return (
        <Tag tone="ghost" title="No sessions or memory files">
          empty
        </Tag>
      );
    case "alive":
    default:
      return null;
  }
}

function EmptyRow({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        padding: "var(--sp-60)",
        textAlign: "center",
        color: "var(--fg-faint)",
        fontSize: "var(--fs-sm)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-6)",
        alignItems: "center",
      }}
    >
      {children}
    </div>
  );
}

/**
 * Status counts for the filter bar. Re-exported so the section
 * header can display "14 orphaned · 3 offline · 2 empty" without
 * recomputing.
 */
export function countByStatus(
  projects: ProjectInfo[],
): Record<ProjectStatus, number> {
  const counts: Record<ProjectStatus, number> = {
    alive: 0,
    orphan: 0,
    unreachable: 0,
    empty: 0,
  };
  for (const p of projects) counts[classifyProject(p)] += 1;
  return counts;
}
