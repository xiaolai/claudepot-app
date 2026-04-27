import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../../api";
import { Glyph } from "../../components/primitives/Glyph";
import { useReachTop } from "../../hooks/useReachTop";
import { NF } from "../../icons";
import type {
  ProjectInfo,
  SessionChunk,
  SessionDetail as SessionDetailData,
} from "../../types";
import { MoveSessionModal } from "../projects/MoveSessionModal";
import { SessionContextPanel } from "./SessionContextPanel";
import { SessionDetailBody } from "./components/SessionDetailBody";
import { SessionDetailHeader } from "./components/SessionDetailHeader";
import { LiveStatusHeader } from "./LiveStatusHeader";
import { LoadingPane } from "./components/SessionDetailStates";
import {
  chunkMatchesSearch,
  classifyMetaMatch,
  eventMatchesSearch,
  isUnknownCommandError,
  normalizeDetailQuery,
} from "./sessionDetail.search";

const INITIAL_EVENT_LIMIT = 500;
const EVENT_PAGE = 500;
const INITIAL_CHUNK_LIMIT = 150;
const CHUNK_PAGE = 150;


type ViewMode = "chunks" | "raw";

/**
 * Right-pane transcript viewer. Loads the full JSONL for the selected
 * session, renders a rich header strip (identity, path, git, tokens,
 * actions), then the event list underneath.
 *
 * Events stream in paginated chunks because a multi-day session can
 * carry 3k+ lines — rendering them all at once stalls the webview's
 * first paint. Default window is the most recent `INITIAL_EVENT_LIMIT`
 * events; "Show older" reveals the next batch in `EVENT_PAGE` steps.
 */
export function SessionDetail({
  filePath,
  projects,
  refreshSignal,
  initialSearch,
  onMoved,
  onError,
  onBack,
}: {
  /** Absolute path to the transcript on disk. Path-based because CC
   * can produce two rows that share a session_id (interrupted adopt). */
  filePath: string;
  /** Live list of projects — powers the Move target picker. */
  projects: ProjectInfo[];
  /** Bumped by the parent so the detail refetches after a move. */
  refreshSignal: number;
  /**
   * Seed for the detail's search input. Carried in from the list-level
   * filter so a user who lands here via a query doesn't have to retype.
   * Only read at mount — the parent passes `key={filePath}` to remount
   * on selection change, so subsequent keystrokes in the list filter
   * don't clobber an edit the user has already made in the detail.
   */
  initialSearch?: string;
  onMoved: () => void;
  onError?: (msg: string) => void;
  onBack?: () => void;
}) {
  const [detail, setDetail] = useState<SessionDetailData | null>(null);
  const [chunks, setChunks] = useState<SessionChunk[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState<string>(() => initialSearch ?? "");
  const [visibleCount, setVisibleCount] = useState(INITIAL_EVENT_LIMIT);
  const [visibleChunks, setVisibleChunks] = useState(INITIAL_CHUNK_LIMIT);
  const [viewMode, setViewMode] = useState<ViewMode>("chunks");
  const [contextOpen, setContextOpen] = useState(false);
  const [moveOpen, setMoveOpen] = useState(false);
  const [scrollEl, setScrollEl] = useState<HTMLDivElement | null>(null);
  const [compact, setCompact] = useState(false);
  const tokenRef = useRef(0);
  const topSentinelRef = useRef<HTMLDivElement | null>(null);

  // Auto-compact the header once the user has scrolled into the
  // transcript. A small hysteresis (engage at 16px, release at 4px)
  // keeps the boundary from flickering when the user lands on a
  // mid-position with momentum scroll. Cleared/re-attached when the
  // scroll container changes — currently SessionDetail is keyed on
  // `filePath` upstream, so the element identity tracks selection.
  useEffect(() => {
    if (!scrollEl) return;
    const ENGAGE = 16;
    const RELEASE = 4;
    const onScroll = () => {
      const top = scrollEl.scrollTop;
      setCompact((c) => {
        if (c && top < RELEASE) return false;
        if (!c && top > ENGAGE) return true;
        return c;
      });
    };
    scrollEl.addEventListener("scroll", onScroll, { passive: true });
    onScroll();
    return () => scrollEl.removeEventListener("scroll", onScroll);
  }, [scrollEl]);

  useEffect(() => {
    const myToken = ++tokenRef.current;
    setLoading(true);
    setError(null);
    // Fetch detail + chunks in parallel. Both open the same JSONL, so
    // this doubles IO; the cheapest shared-state fix is to do both here
    // rather than chain them, and to rely on the OS page cache for the
    // second open — typical sessions are <1 MB.
    //
    // Chunks may legitimately fail on an older Tauri binary that
    // doesn't ship the `session_chunks` command. We distinguish that
    // compatibility case (the invoke error mentions an unknown command
    // or missing handler) from real failures. Everything else is
    // surfaced so we don't silently hide debugger breakage.
    Promise.all([
      api.sessionReadPath(filePath),
      api.sessionChunks(filePath).catch((e: unknown) => {
        if (isUnknownCommandError(e)) return null;
        throw e;
      }),
    ])
      .then(([d, c]) => {
        if (myToken !== tokenRef.current) return;
        setDetail(d);
        setChunks(c);
        setLoading(false);
      })
      .catch((e) => {
        if (myToken !== tokenRef.current) return;
        setError(String(e));
        setLoading(false);
      });
  }, [filePath, refreshSignal]);

  const events = detail?.events ?? [];

  /**
   * One canonical form of the user's query, shared by every predicate
   * that narrows the view. Derived through `normalizeDetailQuery` —
   * see `sessionDetail.search.ts` for the length floor and trim/case
   * rules. A single source of truth prevents the "0 matches + meta
   * banner" trap that used to fire when one path trimmed and another
   * didn't.
   */
  const normalizedQuery = useMemo(
    () => normalizeDetailQuery(search),
    [search],
  );

  /**
   * A single JSONL assistant line can expand into multiple
   * `SessionEvent`s (e.g. `assistantText` + `assistantToolUse`),
   * and they all share one CC `uuid`. Using `kind+uuid` as the
   * React key therefore collides inside the same turn. Index in
   * `events` is guaranteed unique and stable for the life of the
   * loaded detail — exactly what `key` wants. We carry it through
   * filter + paginate so expand/collapse state survives "Show older".
   */
  const filtered = useMemo(() => {
    const indexed = events.map((e, i) => ({ e, i }));
    if (normalizedQuery === null) return indexed;
    return indexed.filter(({ e }) => eventMatchesSearch(e, normalizedQuery));
  }, [events, normalizedQuery]);

  // Show the newest N events first (they're at the tail of the array);
  // "Show older" expands upward.
  const visible = useMemo(() => {
    if (filtered.length <= visibleCount) return filtered;
    return filtered.slice(filtered.length - visibleCount);
  }, [filtered, visibleCount]);

  // When the transcript has zero matches but the list-level filter
  // landed the user here anyway, explain why — project_path / branch /
  // model / session-id live on the row, not on events, so the detail
  // search would otherwise show a bare "Nothing matches that query".
  // Computed once per (row, query) — cheap, and the empty-state
  // branch relies on it.
  const metaMatches = useMemo(() => {
    if (!detail || normalizedQuery === null) return [];
    return classifyMetaMatch(detail.row, normalizedQuery);
  }, [detail, normalizedQuery]);

  // Chunks: same "newest N" pagination semantics.
  const chunksFiltered = useMemo(() => {
    if (!chunks) return null;
    if (normalizedQuery === null) return chunks;
    return chunks.filter((c) => chunkMatchesSearch(c, events, normalizedQuery));
  }, [chunks, normalizedQuery, events]);
  const visibleChunksList = useMemo(() => {
    if (!chunksFiltered) return null;
    if (chunksFiltered.length <= visibleChunks) return chunksFiltered;
    return chunksFiltered.slice(chunksFiltered.length - visibleChunks);
  }, [chunksFiltered, visibleChunks]);

  // Auto-page older chunks when the top sentinel scrolls into view.
  const hasMoreChunks =
    !!chunksFiltered &&
    !!visibleChunksList &&
    chunksFiltered.length > visibleChunksList.length;
  useReachTop(
    topSentinelRef.current,
    viewMode === "chunks" && hasMoreChunks,
    () => setVisibleChunks((n) => n + CHUNK_PAGE),
  );

  const handleCopyFirstPrompt = useCallback(() => {
    const text = detail?.row.first_user_prompt;
    if (!text) return;
    navigator.clipboard.writeText(text).catch(() => {
      onError?.("Couldn't copy first prompt to clipboard.");
    });
  }, [detail, onError]);

  const handleReveal = useCallback(() => {
    if (!detail) return;
    api.revealInFinder(detail.row.file_path).catch((e) => {
      onError?.(`Couldn't reveal: ${e}`);
    });
  }, [detail, onError]);

  if (loading && !detail) {
    return (
      <LoadingPane>
        <Glyph g={NF.chatAlt} color="var(--fg-ghost)" />
        Loading session…
      </LoadingPane>
    );
  }

  if (error && !detail) {
    return (
      <LoadingPane>
        <Glyph g={NF.warn} color="var(--warn)" />
        <div style={{ color: "var(--fg)" }}>Couldn't load session</div>
        <div style={{ color: "var(--fg-faint)", fontSize: "var(--fs-xs)" }}>
          {error}
        </div>
      </LoadingPane>
    );
  }

  if (!detail) return null;

  const row = detail.row;
  const hidden = Math.max(0, filtered.length - visible.length);

  return (
    <div
      style={{
        display: "flex",
        flex: 1,
        minHeight: 0,
      }}
    >
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          flex: 1,
          minHeight: 0,
        }}
      >
      {/* Live status header — renders only when the selected session
           is currently in the LiveRuntime's aggregate list.
           Cheap no-op otherwise (the component returns null). */}
      <LiveStatusHeader sessionId={row.session_id} />

      <SessionDetailHeader
        row={row}
        chunks={chunks}
        viewMode={viewMode}
        contextOpen={contextOpen}
        compact={compact}
        onBack={onBack}
        onReveal={handleReveal}
        onCopyFirstPrompt={handleCopyFirstPrompt}
        onMoveClick={() => setMoveOpen(true)}
        onToggleViewMode={() =>
          setViewMode((m) => (m === "chunks" ? "raw" : "chunks"))
        }
        onToggleContext={() => setContextOpen((v) => !v)}
        onError={onError}
      />

      <SessionDetailBody
        viewMode={viewMode}
        events={events}
        visible={visible}
        hidden={hidden}
        matchCount={filtered.length}
        metaMatches={metaMatches}
        visibleChunksList={visibleChunksList}
        chunksFiltered={chunksFiltered}
        search={search}
        setSearch={setSearch}
        topSentinelRef={topSentinelRef}
        scrollRef={setScrollEl}
        onLoadMoreEvents={() => setVisibleCount((n) => n + EVENT_PAGE)}
        onLoadMoreChunks={() => setVisibleChunks((n) => n + CHUNK_PAGE)}
        eventPage={EVENT_PAGE}
        chunkPage={CHUNK_PAGE}
      />

      {moveOpen && row.project_from_transcript && (
        <MoveSessionModal
          sessionId={row.session_id}
          fromCwd={row.project_path}
          projects={projects}
          onClose={() => setMoveOpen(false)}
          onCompleted={() => {
            setMoveOpen(false);
            onMoved();
          }}
        />
      )}
      </div>

      {contextOpen && (
        <SessionContextPanel
          filePath={row.file_path}
          onClose={() => setContextOpen(false)}
          refreshSignal={refreshSignal}
        />
      )}
    </div>
  );
}

