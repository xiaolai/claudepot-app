import { type MouseEvent, useMemo, useState } from "react";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import type { SessionRow } from "../../types";
import { EmptyRow } from "./components/EmptyRow";
import { PlainList, VirtualList } from "./components/SessionsList";
import { SessionsTableHeader } from "./components/SessionsTableHeader";
import { VirtualFallbackBoundary } from "./components/VirtualFallbackBoundary";
import {
  bestTimestampMs,
  projectBasename,
} from "./format";
import {
  VIRTUALIZE_THRESHOLD,
  type SessionFilter,
  type SortDir,
  type SortKey,
} from "./sessionsTable.shared";

// Re-exports for callers that import from the table entry point. Keeps
// the SessionsSection import path stable across this extraction.
export {
  countSessionStatus,
  type SessionFilter,
  type SortKey,
  type SortDir,
  VIRTUALIZE_THRESHOLD,
} from "./sessionsTable.shared";

/**
 * Sessions tab table — orchestrates header, sort state, scroll
 * container, and the choice between plain / virtualized list. The
 * heavy lifting (row rendering, virtualization) lives in
 * `./components/`; this file is intentionally thin so the
 * filter→sort→render pipeline is readable end to end.
 */
export function SessionsTable({
  sessions,
  filter,
  selectedId,
  onSelect,
  onContextMenu,
  searchSnippets,
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
  /**
   * Optional map from `file_path` → snippet (already redacted by the
   * backend). Rows whose path appears here show the snippet beneath
   * the metadata line; rows that aren't in the map render unchanged.
   */
  searchSnippets?: Map<string, string>;
}) {
  const [sort, setSort] = useState<{ key: SortKey; dir: SortDir }>({
    key: "last_active",
    dir: "desc",
  });

  /**
   * Scroll-parent reference passed to `VirtualList` via callback ref +
   * state. We can't use a plain `useRef` here: the parent's ref is set
   * during commit, but a child component's `useLayoutEffect` (where
   * `useVirtualizer` calls `getScrollElement`) may already have run
   * with `ref.current === null` in React 19. Holding the element in
   * state guarantees the child re-renders once the element exists, so
   * `useVirtualizer` sees a non-null scroll parent on its second pass.
   */
  const [scrollEl, setScrollEl] = useState<HTMLDivElement | null>(null);

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
    <div
      ref={setScrollEl}
      data-testid="sessions-table-scroll"
      style={{
        flex: 1,
        minHeight: 0,
        overflow: "auto",
        display: "flex",
        flexDirection: "column",
      }}
    >
      <SessionsTableHeader sort={sort} onToggle={toggleSort} />
      {shown.length === 0 ? (
        <EmptyRow>No sessions match this filter.</EmptyRow>
      ) : shown.length > VIRTUALIZE_THRESHOLD ? (
        // Degrade to PlainList if the virtualizer throws on layout —
        // a single bad row height would otherwise bubble to the app-
        // level ErrorBoundary and blank the whole window. The
        // `resetKey` reuses the dataset's reference identity so a
        // refresh / filter / sort change retries virtualization.
        <VirtualFallbackBoundary
          resetKey={shown}
          fallback={
            <PlainList
              shown={shown}
              selectedId={selectedId}
              onSelect={onSelect}
              onContextMenu={onContextMenu}
              searchSnippets={searchSnippets}
            />
          }
        >
          <VirtualList
            shown={shown}
            selectedId={selectedId}
            onSelect={onSelect}
            onContextMenu={onContextMenu}
            searchSnippets={searchSnippets}
            scrollEl={scrollEl}
          />
        </VirtualFallbackBoundary>
      ) : (
        <PlainList
          shown={shown}
          selectedId={selectedId}
          onSelect={onSelect}
          onContextMenu={onContextMenu}
          searchSnippets={searchSnippets}
        />
      )}
    </div>
  );
}
