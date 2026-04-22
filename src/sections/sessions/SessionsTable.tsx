import { useVirtualizer } from "@tanstack/react-virtual";
import React, {
  memo,
  type CSSProperties,
  type MouseEvent,
  useMemo,
  useState,
} from "react";
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

/**
 * Above this count, switch to row-level virtualization.
 *
 * Math: at ~72px per row the non-virtualized list still renders fine
 * up to a couple of laptop viewports' worth of work; past that, every
 * row is off-screen DOM the browser paints for nothing, which is the
 * condition virtualization wins on. 80 rows × 72 px ≈ 5760 px — above
 * any realistic viewport, so the crossover matches the paint budget.
 *
 * Secondary factor: jsdom returns 0 for layout metrics, so tests that
 * mount fewer than this many rows stay on the plain path and assert
 * real DOM. A dedicated virtualization test mocks layout to exercise
 * the virtualized path explicitly.
 */
export const VIRTUALIZE_THRESHOLD = 80;

/**
 * Initial row-height estimate used by the virtualizer before each row's
 * real height is measured. Biased above the common "metadata line
 * only" row (~58px) so that rows that also show a deep-search snippet
 * (~85px) don't jolt the scrollbar thumb on first paint. The real
 * height is measured post-paint via `measureElement`, so this only
 * controls the first frame.
 */
const ESTIMATED_ROW_PX = 72;

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
      <Header sort={sort} onToggle={toggleSort} />
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

function Header({
  sort,
  onToggle,
}: {
  sort: { key: SortKey; dir: SortDir };
  onToggle: (key: SortKey) => void;
}) {
  return (
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
        onToggle={onToggle}
      />
      <SortHeader
        label="Turns"
        col="turns"
        currentKey={sort.key}
        currentDir={sort.dir}
        onToggle={onToggle}
      />
      <SortHeader
        label="Tokens"
        col="tokens"
        currentKey={sort.key}
        currentDir={sort.dir}
        onToggle={onToggle}
      />
      <SortHeader
        label="Last active"
        col="last_active"
        currentKey={sort.key}
        currentDir={sort.dir}
        onToggle={onToggle}
      />
      <span />
    </div>
  );
}

interface ListProps {
  shown: SessionRow[];
  selectedId: string | null;
  onSelect: (filePath: string) => void;
  onContextMenu?: (e: MouseEvent, s: SessionRow) => void;
  searchSnippets?: Map<string, string>;
}

/**
 * Straight `<ul>` render. Used below the virtualization threshold and
 * as the fallback when the virtualizer boundary catches an error.
 */
function PlainList({
  shown,
  selectedId,
  onSelect,
  onContextMenu,
  searchSnippets,
}: ListProps) {
  return (
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
          snippet={searchSnippets?.get(s.file_path)}
        />
      ))}
    </ul>
  );
}

/**
 * Virtualized render. The `<ul>` sits inside the scroll container and
 * acts as the virtualizer's content element — its height is the total
 * virtual size, and each mounted `<li>` is absolutely positioned via
 * translateY. Listbox semantics survive because every child remains a
 * direct `<li role="option">` of the `<ul>`.
 */
function VirtualList({
  shown,
  selectedId,
  onSelect,
  onContextMenu,
  searchSnippets,
  scrollEl,
}: ListProps & { scrollEl: HTMLDivElement | null }) {
  const rowVirtualizer = useVirtualizer({
    count: shown.length,
    getScrollElement: () => scrollEl,
    estimateSize: () => ESTIMATED_ROW_PX,
    overscan: 8,
    getItemKey: (index) => shown[index].file_path,
  });

  const items = rowVirtualizer.getVirtualItems();

  return (
    <ul
      role="listbox"
      aria-label="Sessions"
      style={{
        listStyle: "none",
        margin: 0,
        padding: 0,
        position: "relative",
        height: rowVirtualizer.getTotalSize(),
      }}
    >
      {items.map((virtualRow) => {
        const s = shown[virtualRow.index];
        return (
          <SessionRowView
            key={s.file_path}
            session={s}
            active={s.file_path === selectedId}
            onSelect={onSelect}
            onContextMenu={onContextMenu}
            snippet={searchSnippets?.get(s.file_path)}
            // Primitive props keep `React.memo`'s shallow equality
            // effective on the virtualized path. An inline style
            // object literal here would change identity every render
            // and defeat memo for every visible row.
            virtualStart={virtualRow.start}
            virtualIndex={virtualRow.index}
            virtualSetSize={shown.length}
            measureRef={rowVirtualizer.measureElement}
          />
        );
      })}
    </ul>
  );
}

/**
 * Local error boundary for `VirtualList`. The virtualizer can, in rare
 * cases, throw during its measurement pass — a bad `ResizeObserver`
 * callback, a disconnected node, a `NaN` height. Without this boundary
 * the error bubbles to the top-level `ErrorBoundary` and blanks the
 * whole app window. We catch it here and drop back to the plain list,
 * losing virtualization but preserving functionality.
 *
 * `resetKey` lets the boundary recover. A measurement glitch is often
 * tied to one specific dataset shape; once `sessions` changes (filter,
 * sort, refresh) we get another shot at virtualization. Without this,
 * a single transient error would latch the section to PlainList until
 * the user navigates away.
 */
interface VirtualFallbackProps {
  children: React.ReactNode;
  fallback: React.ReactNode;
  /** Bumping this resets the failed state so the next paint can retry
   * the virtualized path. Pass something tied to the dataset shape. */
  resetKey: unknown;
}
interface VirtualFallbackState {
  failed: boolean;
  resetKey: unknown;
}
class VirtualFallbackBoundary extends React.Component<
  VirtualFallbackProps,
  VirtualFallbackState
> {
  state: VirtualFallbackState = { failed: false, resetKey: this.props.resetKey };
  static getDerivedStateFromError(): Pick<VirtualFallbackState, "failed"> {
    return { failed: true };
  }
  static getDerivedStateFromProps(
    props: VirtualFallbackProps,
    state: VirtualFallbackState,
  ): Partial<VirtualFallbackState> | null {
    if (props.resetKey !== state.resetKey) {
      return { failed: false, resetKey: props.resetKey };
    }
    return null;
  }
  componentDidCatch(err: unknown): void {
    if (import.meta.env.DEV) {
      // eslint-disable-next-line no-console
      console.error(
        "[SessionsTable] virtualizer threw — falling back to PlainList",
        err,
      );
    }
  }
  render(): React.ReactNode {
    return this.state.failed ? this.props.fallback : this.props.children;
  }
}

interface SessionRowProps {
  session: SessionRow;
  active: boolean;
  onSelect: (filePath: string) => void;
  onContextMenu?: (e: MouseEvent, s: SessionRow) => void;
  snippet?: string;
  /** When rendered under the virtualizer, `virtualStart` is the pixel
   * offset this row translates to. Passed as a primitive so
   * `React.memo`'s shallow equality works — a style object literal
   * here would change identity every render. */
  virtualStart?: number;
  /** Virtualizer's measurement callback — must be attached as a ref so
   * the library records each row's real height on paint. */
  measureRef?: (el: HTMLElement | null) => void;
  /** Index in the virtualized sequence. Required by `measureElement`
   * to reconcile the measured node back to its virtual row. Also
   * exposed as `aria-posinset` so screen readers say "row X of Y"
   * even when the DOM only carries the viewport's worth of items. */
  virtualIndex?: number;
  /** Total option count for the listbox — emitted as `aria-setsize`
   * on virtualized rows so assistive tech sees the full set, not
   * just the mounted window. Plain-path rows leave this undefined
   * since the DOM already carries every option. */
  virtualSetSize?: number;
}

const SessionRowView = memo(function SessionRowView({
  session: s,
  active,
  onSelect,
  onContextMenu,
  snippet,
  virtualStart,
  measureRef,
  virtualIndex,
  virtualSetSize,
}: SessionRowProps) {
  const [hover, setHover] = useState(false);
  const lastTs = bestTimestampMs(s.last_ts, s.last_modified_ms);
  const project = projectBasename(s.project_path) || s.slug;
  const headline =
    s.first_user_prompt?.trim() ||
    (s.is_sidechain ? "Agent subsession" : shortSessionId(s.session_id));
  const model = modelBadge(s.models);
  const tokens = formatTokens(s.tokens.total);

  // `virtualStart` decides whether this row is rendered under the
  // virtualizer. When it is, the style object is constructed here
  // from primitives so it can be recomputed cheaply when needed and
  // referentially stable when unchanged.
  const virtualStyle: CSSProperties | undefined =
    virtualStart !== undefined
      ? {
          position: "absolute",
          top: 0,
          left: 0,
          width: "100%",
          transform: `translateY(${virtualStart}px)`,
        }
      : undefined;

  return (
    <li
      ref={measureRef}
      data-index={virtualIndex}
      role="option"
      aria-selected={active}
      aria-posinset={
        virtualIndex !== undefined ? virtualIndex + 1 : undefined
      }
      aria-setsize={virtualSetSize}
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
        ...virtualStyle,
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
        {snippet && (
          <div
            data-testid="search-snippet"
            title={snippet}
            style={{
              marginTop: "var(--sp-4)",
              color: "var(--fg-muted)",
              fontSize: "var(--fs-xs)",
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
              fontStyle: "italic",
            }}
          >
            {snippet}
          </div>
        )}
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
});

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
