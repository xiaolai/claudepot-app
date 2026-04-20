import { type MouseEvent, useMemo, useState } from "react";
import { Glyph } from "../../components/primitives/Glyph";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import type { ProjectInfo } from "../../types";
import { formatSize } from "./format";
import { classifyProject, type ProjectStatus } from "./projectStatus";

export type ProjectFilter = "all" | "orphan" | "unreachable" | "empty";

const COLS = "var(--sp-20) 1.6fr 0.9fr 0.9fr 0.8fr 1fr var(--sp-24)";

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
  const shown = useMemo(() => {
    if (filter === "all") return projects;
    return projects.filter((p) => classifyProject(p) === filter);
  }, [projects, filter]);

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
        <span>Project</span>
        <span>Sessions</span>
        <span>Size</span>
        <span>Status</span>
        <span>Last touched</span>
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
  const name =
    p.original_path.split("/").filter(Boolean).pop() ?? p.sanitized_name;

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
        {formatRelative(p.last_modified_ms) ?? "—"}
      </span>

      <span>
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

function formatRelative(ms: number | null): string | null {
  if (ms === null) return null;
  const diff = Date.now() - ms;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
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
