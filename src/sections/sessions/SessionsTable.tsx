import { type MouseEvent, useMemo, useState } from "react";
import { Glyph } from "../../components/primitives/Glyph";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import type { SessionRow } from "../../types";
import { formatRelativeTime, formatSize } from "../projects/format";
import {
  bestTimestampMs,
  formatTokens,
  modelBadge,
  projectBasename,
  shortSessionId,
} from "./format";

export type SessionFilter = "all" | "errors" | "sidechain";

export type SortKey =
  | "last_active"
  | "project"
  | "turns"
  | "tokens"
  | "size";
export type SortDir = "asc" | "desc";

/**
 * Column template:
 *   glyph | session preview | project | turns | tokens | last-active | chevron
 */
const COLS = "var(--sp-20) 2fr 1.1fr 0.6fr 0.7fr 0.9fr var(--sp-24)";

export function SessionsTable({
  sessions,
  filter,
  selectedId,
  onSelect,
  onContextMenu,
}: {
  sessions: SessionRow[];
  filter: SessionFilter;
  /** Selected row — keyed by `file_path`, not `session_id`, because CC
   * can end up with two files that share a session_id (interrupted
   * rescue / adopt, manual copy). file_path is always unique. */
  selectedId: string | null;
  /** Called with `file_path` (unique per row on disk). */
  onSelect: (filePath: string) => void;
  onContextMenu?: (e: MouseEvent, s: SessionRow) => void;
}) {
  const [sort, setSort] = useState<{ key: SortKey; dir: SortDir }>({
    key: "last_active",
    dir: "desc",
  });

  const toggleSort = (key: SortKey) => {
    setSort((prev) => {
      if (prev.key !== key) return { key, dir: "asc" };
      if (prev.dir === "asc") return { key, dir: "desc" };
      return { key: "last_active", dir: "desc" };
    });
  };

  const shown = useMemo(() => {
    const filtered = sessions.filter((s) => {
      if (filter === "errors") return s.has_error;
      if (filter === "sidechain") return s.is_sidechain;
      return true;
    });
    const cmp = (a: SessionRow, b: SessionRow): number => {
      switch (sort.key) {
        case "last_active": {
          const av = bestTimestampMs(a.last_ts, a.last_modified_ms) ?? 0;
          const bv = bestTimestampMs(b.last_ts, b.last_modified_ms) ?? 0;
          return av - bv;
        }
        case "project":
          return projectBasename(a.project_path)
            .toLowerCase()
            .localeCompare(projectBasename(b.project_path).toLowerCase());
        case "turns":
          return a.message_count - b.message_count;
        case "tokens":
          return a.tokens.total - b.tokens.total;
        case "size":
          return a.file_size_bytes - b.file_size_bytes;
      }
    };
    const sorted = [...filtered].sort(cmp);
    if (sort.dir === "desc") sorted.reverse();
    return sorted;
  }, [sessions, filter, sort]);

  if (sessions.length === 0) {
    return (
      <EmptyRow>
        <Glyph g={NF.chatAlt} size="var(--sp-24)" color="var(--fg-ghost)" />
        <div>No CC sessions on disk.</div>
        <div
          style={{
            marginTop: "var(--sp-4)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-faint)",
          }}
        >
          Run <code style={{ fontFamily: "var(--font)" }}>claude</code> in
          a project to start one.
        </div>
      </EmptyRow>
    );
  }

  return (
    <>
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
        <span>Session</span>
        <SortHeader
          label="Project"
          col="project"
          currentKey={sort.key}
          currentDir={sort.dir}
          onToggle={toggleSort}
        />
        <SortHeader
          label="Turns"
          col="turns"
          currentKey={sort.key}
          currentDir={sort.dir}
          onToggle={toggleSort}
        />
        <SortHeader
          label="Tokens"
          col="tokens"
          currentKey={sort.key}
          currentDir={sort.dir}
          onToggle={toggleSort}
        />
        <SortHeader
          label="Last active"
          col="last_active"
          currentKey={sort.key}
          currentDir={sort.dir}
          onToggle={toggleSort}
        />
        <span />
      </div>

      {shown.length === 0 ? (
        <EmptyRow>No sessions match this filter.</EmptyRow>
      ) : (
        <ul
          role="listbox"
          aria-label="Sessions"
          style={{ listStyle: "none", margin: 0, padding: 0 }}
        >
          {shown.map((s) => (
            <SessionRowView
              key={s.file_path}
              session={s}
              active={s.file_path === selectedId}
              onSelect={onSelect}
              onContextMenu={onContextMenu}
            />
          ))}
        </ul>
      )}
    </>
  );
}

function SessionRowView({
  session: s,
  active,
  onSelect,
  onContextMenu,
}: {
  session: SessionRow;
  active: boolean;
  onSelect: (filePath: string) => void;
  onContextMenu?: (e: MouseEvent, s: SessionRow) => void;
}) {
  const [hover, setHover] = useState(false);
  const lastTs = bestTimestampMs(s.last_ts, s.last_modified_ms);
  const project = projectBasename(s.project_path) || s.slug;
  const headline =
    s.first_user_prompt?.trim() ||
    (s.is_sidechain ? "Agent subsession" : shortSessionId(s.session_id));
  const model = modelBadge(s.models);
  const tokens = formatTokens(s.tokens.total);

  return (
    <li
      role="option"
      aria-selected={active}
      tabIndex={0}
      onClick={() => onSelect(s.file_path)}
      onContextMenu={onContextMenu ? (e) => onContextMenu(e, s) : undefined}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect(s.file_path);
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
          g={s.has_error ? NF.warn : NF.chatAlt}
          color={s.has_error ? "var(--warn)" : "var(--fg-muted)"}
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
            title={s.first_user_prompt ?? s.session_id}
            style={{
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
          >
            {headline}
          </span>
          {s.is_sidechain && (
            <Tag tone="ghost" title="Agent subsession">
              agent
            </Tag>
          )}
          {s.has_error && (
            <Tag tone="warn" glyph={NF.warn} title="This session had an error">
              error
            </Tag>
          )}
        </div>
        <div
          style={{
            marginTop: "var(--sp-2)",
            color: "var(--fg-faint)",
            fontSize: "var(--fs-xs)",
            display: "flex",
            gap: "var(--sp-8)",
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
          <span className="mono">{shortSessionId(s.session_id)}</span>
          {model && (
            <>
              <span>·</span>
              <span>{model}</span>
            </>
          )}
          {s.git_branch && (
            <>
              <span>·</span>
              <span style={{ display: "inline-flex", gap: "var(--sp-4)" }}>
                <Glyph
                  g={NF.branch}
                  style={{ fontSize: "var(--fs-2xs)" }}
                />
                {s.git_branch}
              </span>
            </>
          )}
          {s.file_size_bytes > 0 && (
            <>
              <span>·</span>
              <span>{formatSize(s.file_size_bytes)}</span>
            </>
          )}
        </div>
      </div>

      <div style={{ minWidth: 0, overflow: "hidden" }}>
        <div
          title={s.project_path}
          style={{
            color: "var(--fg-muted)",
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
          {project}
        </div>
        {!s.project_from_transcript && (
          <div
            style={{
              marginTop: "var(--sp-2)",
              color: "var(--fg-ghost)",
              fontSize: "var(--fs-xs)",
            }}
            title="Decoded from the on-disk slug — the transcript didn't carry a cwd field"
          >
            decoded from slug
          </div>
        )}
      </div>

      <span
        style={{
          color: s.message_count > 0 ? "var(--fg-muted)" : "var(--fg-ghost)",
          fontVariantNumeric: "tabular-nums",
        }}
        title={`${s.user_message_count} user · ${s.assistant_message_count} assistant`}
      >
        {s.message_count > 0 ? s.message_count : "—"}
      </span>

      <span
        style={{
          color: s.tokens.total > 0 ? "var(--fg-muted)" : "var(--fg-ghost)",
          fontVariantNumeric: "tabular-nums",
        }}
        title={
          s.tokens.total > 0
            ? `input ${s.tokens.input} · output ${s.tokens.output} · cache r/w ${s.tokens.cache_read}/${s.tokens.cache_creation}`
            : undefined
        }
      >
        {tokens || "—"}
      </span>

      <span
        style={{
          color: "var(--fg-faint)",
          whiteSpace: "nowrap",
          overflow: "hidden",
          textOverflow: "ellipsis",
        }}
      >
        {lastTs != null ? formatRelativeTime(lastTs) : "—"}
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

export function countSessionStatus(
  sessions: SessionRow[],
): Record<SessionFilter, number> {
  const counts: Record<SessionFilter, number> = {
    all: sessions.length,
    errors: 0,
    sidechain: 0,
  };
  for (const s of sessions) {
    if (s.has_error) counts.errors += 1;
    if (s.is_sidechain) counts.sidechain += 1;
  }
  return counts;
}
