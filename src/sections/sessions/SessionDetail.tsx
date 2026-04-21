import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { Glyph } from "../../components/primitives/Glyph";
import { IconButton } from "../../components/primitives/IconButton";
import { Input } from "../../components/primitives/Input";
import { Tag } from "../../components/primitives/Tag";
import { CopyButton } from "../../components/CopyButton";
import { NF } from "../../icons";
import type {
  ProjectInfo,
  SessionChunk,
  SessionDetail as SessionDetailData,
  SessionEvent,
} from "../../types";
import { formatRelativeTime, formatSize } from "../projects/format";
import { MoveSessionModal } from "../projects/MoveSessionModal";
import {
  bestTimestampMs,
  formatTokens,
  modelBadge,
  projectBasename,
  shortSessionId,
} from "./format";
import { SessionChunkView } from "./SessionChunkView";
import { SessionContextPanel } from "./SessionContextPanel";
import { SessionEventView } from "./SessionEventView";
import { SessionExportMenu } from "./SessionExportMenu";

const INITIAL_EVENT_LIMIT = 500;
const EVENT_PAGE = 500;
const INITIAL_CHUNK_LIMIT = 150;
const CHUNK_PAGE = 150;

/**
 * Hook: when `el` scrolls into the viewport, call `onReach`. Used to
 * auto-page older chunks when the user scrolls to the top of the
 * transcript. `prefers-reduced-motion` users still get the behavior
 * — it's not animation, just pagination.
 */
function useReachTop(
  el: HTMLElement | null,
  enabled: boolean,
  onReach: () => void,
) {
  useEffect(() => {
    if (!el || !enabled) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) onReach();
      },
      { threshold: 0 },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [el, enabled, onReach]);
}

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
    // Fetch detail + chunks in parallel — both hit the same JSONL but
    // Tauri serializes invokes, so the second one is cheap. Chunks may
    // fail (older Tauri build) — we degrade to raw event mode.
    Promise.all([
      api.sessionReadPath(filePath),
      api.sessionChunks(filePath).catch(() => null),
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
  const lastTs = bestTimestampMs(row.last_ts, row.last_modified_ms);
  const firstTs = row.first_ts ? Date.parse(row.first_ts) : null;
  const project = projectBasename(row.project_path) || row.slug;
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
      {/* Header strip ---------------------------------------------------- */}
      <div
        style={{
          padding: "var(--sp-20) var(--sp-28) var(--sp-14)",
          borderBottom: "var(--bw-hair) solid var(--line)",
          flexShrink: 0,
          background: "var(--bg)",
        }}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--sp-8)",
            marginBottom: "var(--sp-6)",
          }}
        >
          {onBack && (
            <IconButton
              glyph={NF.chevronL}
              onClick={onBack}
              title="Back to session list"
              aria-label="Back to session list"
            />
          )}
          <div
            style={{
              fontSize: "var(--fs-2xs)",
              color: "var(--fg-faint)",
              letterSpacing: "var(--ls-wide)",
              textTransform: "uppercase",
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-6)",
            }}
          >
            <span>{project}</span>
            <Glyph g={NF.chevronR} style={{ fontSize: "var(--fs-3xs)" }} />
            <span className="mono" title={row.session_id}>
              {shortSessionId(row.session_id)}
            </span>
            <CopyButton text={row.session_id} />
          </div>
        </div>

        <h2
          style={{
            margin: 0,
            fontSize: "var(--fs-lg)",
            fontWeight: 600,
            color: "var(--fg)",
            letterSpacing: "var(--ls-normal)",
            textTransform: "none",
            overflow: "hidden",
            textOverflow: "ellipsis",
            display: "-webkit-box",
            WebkitLineClamp: 2,
            WebkitBoxOrient: "vertical",
          }}
          title={row.first_user_prompt ?? undefined}
        >
          {row.first_user_prompt?.trim() ||
            (row.is_sidechain ? "Agent subsession" : "(untitled session)")}
        </h2>

        <div
          style={{
            marginTop: "var(--sp-10)",
            display: "flex",
            flexWrap: "wrap",
            gap: "var(--sp-8)",
          }}
        >
          {row.has_error && (
            <Tag tone="warn" glyph={NF.warn}>
              error
            </Tag>
          )}
          {row.is_sidechain && <Tag tone="ghost">agent</Tag>}
          {row.models.length > 0 && (
            <Tag tone="accent" title={row.models.join(", ")}>
              {modelBadge(row.models)}
            </Tag>
          )}
          {row.git_branch && (
            <Tag tone="neutral" glyph={NF.branch}>
              {row.git_branch}
            </Tag>
          )}
          {row.cc_version && <Tag tone="ghost">cc {row.cc_version}</Tag>}
          {row.tokens.total > 0 && (
            <Tag
              tone="neutral"
              title={`input ${row.tokens.input} · output ${row.tokens.output} · cache r/w ${row.tokens.cache_read}/${row.tokens.cache_creation}`}
            >
              {formatTokens(row.tokens.total)} tok
            </Tag>
          )}
          {row.message_count > 0 && (
            <Tag tone="neutral">
              {row.message_count} turn{row.message_count === 1 ? "" : "s"}
            </Tag>
          )}
          <Tag tone="ghost">{formatSize(row.file_size_bytes)}</Tag>
        </div>

        <div
          style={{
            marginTop: "var(--sp-10)",
            display: "flex",
            flexWrap: "wrap",
            gap: "var(--sp-12) var(--sp-16)",
            alignItems: "center",
            color: "var(--fg-muted)",
            fontSize: "var(--fs-xs)",
          }}
        >
          <span
            title={row.project_path}
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: "var(--sp-6)",
              maxWidth: "100%",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            <Glyph g={NF.folder} style={{ fontSize: "var(--fs-2xs)" }} />
            <span
              className="mono"
              style={{
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {row.project_path}
            </span>
            <CopyButton text={row.project_path} />
          </span>
          {firstTs != null && (
            <span title={row.first_ts ?? ""}>
              Started {formatRelativeTime(firstTs)}
            </span>
          )}
          {lastTs != null && (
            <span title={row.last_ts ?? ""}>
              Last event {formatRelativeTime(lastTs)}
            </span>
          )}
        </div>

        <div
          style={{
            marginTop: "var(--sp-14)",
            display: "flex",
            flexWrap: "wrap",
            gap: "var(--sp-8)",
          }}
        >
          <Button
            variant="ghost"
            glyph={NF.folderOpen}
            glyphColor="var(--fg-muted)"
            onClick={handleReveal}
          >
            Reveal
          </Button>
          <Button
            variant="ghost"
            glyph={NF.arrowR}
            glyphColor="var(--fg-muted)"
            onClick={() => setMoveOpen(true)}
            disabled={!row.project_from_transcript}
            title={
              row.project_from_transcript
                ? "Move this session's transcript to another project"
                : "Can't move: no cwd recorded in the transcript"
            }
          >
            Move to project…
          </Button>
          {row.first_user_prompt && (
            <Button
              variant="ghost"
              glyph={NF.copy}
              glyphColor="var(--fg-muted)"
              onClick={handleCopyFirstPrompt}
            >
              Copy first prompt
            </Button>
          )}
          <SessionExportMenu filePath={row.file_path} onError={onError} />
          {chunks !== null && (
            <Button
              variant="ghost"
              glyph={viewMode === "chunks" ? NF.layers : NF.fileText}
              glyphColor="var(--fg-muted)"
              onClick={() =>
                setViewMode((m) => (m === "chunks" ? "raw" : "chunks"))
              }
              title={
                viewMode === "chunks"
                  ? "Switch to raw event stream"
                  : "Switch to chunked view"
              }
            >
              {viewMode === "chunks" ? "Raw events" : "Chunked"}
            </Button>
          )}
          <Button
            variant="ghost"
            glyph={NF.sliders}
            glyphColor="var(--fg-muted)"
            onClick={() => setContextOpen((v) => !v)}
            aria-pressed={contextOpen}
            title="Toggle visible-context panel"
          >
            {contextOpen ? "Hide context" : "Context"}
          </Button>
        </div>
      </div>

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

function LoadingPane({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: "var(--sp-8)",
        padding: "var(--sp-48)",
        color: "var(--fg-muted)",
        fontSize: "var(--fs-sm)",
      }}
    >
      {children}
    </div>
  );
}

function EmptyState({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: "var(--sp-8)",
        padding: "var(--sp-48)",
        color: "var(--fg-muted)",
        fontSize: "var(--fs-sm)",
      }}
    >
      {children}
    </div>
  );
}

function chunkMatchesSearch(
  chunk: SessionChunk,
  events: SessionEvent[],
  q: string,
): boolean {
  const indices =
    chunk.chunkType === "ai"
      ? chunk.event_indices
      : [chunk.event_index];
  for (const idx of indices) {
    const ev = events[idx];
    if (ev && eventMatchesSearch(ev, q)) return true;
  }
  // AI chunks: also match against linked tool calls.
  if (chunk.chunkType === "ai") {
    for (const t of chunk.tool_executions) {
      if (
        t.tool_name.toLowerCase().includes(q) ||
        t.input_preview.toLowerCase().includes(q) ||
        (t.result_content ?? "").toLowerCase().includes(q)
      ) {
        return true;
      }
    }
  }
  return false;
}

function eventMatchesSearch(e: SessionEvent, q: string): boolean {
  switch (e.kind) {
    case "userText":
    case "assistantText":
    case "assistantThinking":
    case "summary":
      return e.text.toLowerCase().includes(q);
    case "userToolResult":
      return e.content.toLowerCase().includes(q) || e.tool_use_id.includes(q);
    case "assistantToolUse":
      return (
        e.tool_name.toLowerCase().includes(q) ||
        e.input_preview.toLowerCase().includes(q)
      );
    case "system":
      return (
        (e.subtype ?? "").toLowerCase().includes(q) ||
        e.detail.toLowerCase().includes(q)
      );
    case "attachment":
      return (e.name ?? "").toLowerCase().includes(q);
    case "fileSnapshot":
      return false;
    case "other":
      return e.raw_type.toLowerCase().includes(q);
    case "malformed":
      return e.preview.toLowerCase().includes(q);
  }
}
