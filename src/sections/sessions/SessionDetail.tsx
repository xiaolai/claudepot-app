import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { Glyph } from "../../components/primitives/Glyph";
import { Input } from "../../components/primitives/Input";
import { useReachTop } from "../../hooks/useReachTop";
import { NF } from "../../icons";
import type {
  ProjectInfo,
  SessionChunk,
  SessionDetail as SessionDetailData,
} from "../../types";
import { MoveSessionModal } from "../projects/MoveSessionModal";
import { SessionChunkView } from "./SessionChunkView";
import { SessionContextPanel } from "./SessionContextPanel";
import { SessionDetailHeader } from "./components/SessionDetailHeader";
import { SessionEventView } from "./SessionEventView";
import { LiveStatusHeader } from "./LiveStatusHeader";
import { EmptyState, LoadingPane } from "./components/SessionDetailStates";
import {
  chunkMatchesSearch,
  eventMatchesSearch,
  isUnknownCommandError,
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
  onMoved: () => void;
  onError?: (msg: string) => void;
  onBack?: () => void;
}) {
  const [detail, setDetail] = useState<SessionDetailData | null>(null);
  const [chunks, setChunks] = useState<SessionChunk[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [visibleCount, setVisibleCount] = useState(INITIAL_EVENT_LIMIT);
  const [visibleChunks, setVisibleChunks] = useState(INITIAL_CHUNK_LIMIT);
  const [viewMode, setViewMode] = useState<ViewMode>("chunks");
  const [contextOpen, setContextOpen] = useState(false);
  const [moveOpen, setMoveOpen] = useState(false);
  const tokenRef = useRef(0);
  const topSentinelRef = useRef<HTMLDivElement | null>(null);

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
    if (!search.trim() || search.trim().length < 2) return indexed;
    const q = search.toLowerCase();
    return indexed.filter(({ e }) => eventMatchesSearch(e, q));
  }, [events, search]);

  // Show the newest N events first (they're at the tail of the array);
  // "Show older" expands upward.
  const visible = useMemo(() => {
    if (filtered.length <= visibleCount) return filtered;
    return filtered.slice(filtered.length - visibleCount);
  }, [filtered, visibleCount]);

  // Chunks: same "newest N" pagination semantics.
  const chunksFiltered = useMemo(() => {
    if (!chunks) return null;
    if (!search.trim() || search.trim().length < 2) return chunks;
    const q = search.toLowerCase();
    return chunks.filter((c) => chunkMatchesSearch(c, events, q));
  }, [chunks, search, events]);
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

      {/* Search bar ------------------------------------------------------ */}
      <div
        style={{
          padding: "var(--sp-10) var(--sp-28)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-10)",
          flexShrink: 0,
        }}
      >
        <Input
          glyph={NF.search}
          placeholder="Search within transcript"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          style={{ flex: 1 }}
          aria-label="Search within transcript"
        />
        {search.trim().length >= 2 && (
          <span
            style={{
              fontSize: "var(--fs-xs)",
              color: "var(--fg-faint)",
              whiteSpace: "nowrap",
            }}
          >
            {filtered.length} match{filtered.length === 1 ? "" : "es"}
          </span>
        )}
      </div>

      {/* Transcript ------------------------------------------------------ */}
      <div
        style={{
          flex: 1,
          minHeight: 0,
          overflow: "auto",
          padding: "var(--sp-18) var(--sp-28)",
        }}
      >
        {viewMode === "chunks" && visibleChunksList ? (
          visibleChunksList.length === 0 ? (
            <EmptyState>
              <Glyph g={NF.chatAlt} color="var(--fg-ghost)" />
              {search.trim()
                ? "Nothing matches that query."
                : "This session has no events yet."}
            </EmptyState>
          ) : (
            <>
              <div
                ref={topSentinelRef}
                data-testid="chunks-top-sentinel"
                aria-hidden
                style={{ height: 1 }}
              />
              {chunksFiltered &&
                chunksFiltered.length > visibleChunksList.length && (
                  <div
                    style={{
                      display: "flex",
                      justifyContent: "center",
                      marginBottom: "var(--sp-14)",
                    }}
                  >
                    <Button
                      variant="ghost"
                      onClick={() => setVisibleChunks((n) => n + CHUNK_PAGE)}
                    >
                      Show{" "}
                      {Math.min(
                        chunksFiltered.length - visibleChunksList.length,
                        CHUNK_PAGE,
                      )}{" "}
                      older chunk
                      {chunksFiltered.length - visibleChunksList.length === 1
                        ? ""
                        : "s"}
                    </Button>
                  </div>
                )}
              {visibleChunksList.map((c) => (
                <SessionChunkView
                  key={c.id}
                  chunk={c}
                  events={events}
                  searchTerm={search.trim()}
                />
              ))}
            </>
          )
        ) : filtered.length === 0 ? (
          <EmptyState>
            <Glyph g={NF.chatAlt} color="var(--fg-ghost)" />
            {search.trim()
              ? "Nothing matches that query."
              : "This session has no events yet."}
          </EmptyState>
        ) : (
          <>
            {hidden > 0 && (
              <div
                style={{
                  display: "flex",
                  justifyContent: "center",
                  marginBottom: "var(--sp-14)",
                }}
              >
                <Button
                  variant="ghost"
                  onClick={() => setVisibleCount((n) => n + EVENT_PAGE)}
                >
                  Show {Math.min(hidden, EVENT_PAGE)} older event
                  {hidden === 1 ? "" : "s"}
                </Button>
              </div>
            )}
            {visible.map(({ e, i }) => (
              <SessionEventView
                key={i}
                event={e}
                searchTerm={search.trim()}
              />
            ))}
          </>
        )}
      </div>

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

