// Session move (orphan/adopt/discard) + Sessions index + debugger.
// Sharded from src/api.ts; src/api/index.ts merges every
// domain slice into the canonical `api` object.

import { invoke } from "@tauri-apps/api/core";
import type {
  AdoptReport,
  DiscardReport,
  MoveSessionReport,
  OrphanedProject,
  RunningOpInfo,
  ContextStats,
  RepositoryGroup,
  SearchHit,
  SessionChunk,
  SessionDetail,
  SessionRow,
} from "../types";

export const sessionApi = {
  // ---------- Session move ----------
  /**
   * Scan ~/.claude/projects for slugs whose internal cwd no longer
   * exists on disk. Returns the set of adoption candidates — the
   * primary surface of the orphan-rescue flow.
   */
  sessionListOrphans: () => invoke<OrphanedProject[]>("session_list_orphans"),
  /**
   * Move a single session transcript from one project cwd to another.
   * Surfaces touched: primary JSONL (cwd rewrite every line), the
   * session's subagents/remote-agents subdir, history.jsonl entries
   * keyed by sessionId, and .claude.json's lastSessionId /
   * activeWorktreeSession.sessionId pointers for the source cwd.
   */
  sessionMove: (args: {
    sessionId: string;
    fromCwd: string;
    toCwd: string;
    forceLive?: boolean;
    forceConflict?: boolean;
    cleanupSource?: boolean;
  }) =>
    invoke<MoveSessionReport>("session_move", {
      sessionId: args.sessionId,
      fromCwd: args.fromCwd,
      toCwd: args.toCwd,
      forceLive: args.forceLive ?? false,
      forceConflict: args.forceConflict ?? false,
      cleanupSource: args.cleanupSource ?? false,
    }),
  /**
   * Start an async session move and return the op_id immediately.
   * Subscribe to `op-progress::<op_id>` for S1..S5 phase events;
   * call `sessionMoveStatus(opId)` once the terminal event lands to
   * read the structured `MoveSessionReport`.
   */
  sessionMoveStart: (args: {
    sessionId: string;
    fromCwd: string;
    toCwd: string;
    forceLive?: boolean;
    forceConflict?: boolean;
    cleanupSource?: boolean;
  }) =>
    invoke<string>("session_move_start", {
      sessionId: args.sessionId,
      fromCwd: args.fromCwd,
      toCwd: args.toCwd,
      forceLive: args.forceLive ?? false,
      forceConflict: args.forceConflict ?? false,
      cleanupSource: args.cleanupSource ?? false,
    }),
  /** Poll current state of an in-flight session move. null if op_id unknown. */
  sessionMoveStatus: (opId: string) =>
    invoke<RunningOpInfo | null>("session_move_status", { opId }),
  /**
   * Move every session under an orphaned slug into a live target cwd.
   * Force-bypasses the live-mtime guard since an orphan's cwd is gone
   * by definition.
   */
  sessionAdoptOrphan: (slug: string, targetCwd: string) =>
    invoke<AdoptReport>("session_adopt_orphan", { slug, targetCwd }),

  /**
   * Move an orphan project slug dir to the OS Trash (reversible).
   * Pair with ConfirmDialog on the caller side — this is destructive
   * from the user's perspective even though Trash makes it recoverable.
   */
  sessionDiscardOrphan: (slug: string) =>
    invoke<DiscardReport>("session_discard_orphan", { slug }),

  // ---------- Session index (Sessions tab) ----------
  /**
   * Walk every `~/.claude/projects/<slug>/<session>.jsonl` and produce
   * list rows with token totals, models seen, first-prompt preview,
   * and CC version. Newest-first by the last-event timestamp (falling
   * back to file mtime). Backed by a persistent SQLite cache in
   * `~/.claudepot/sessions.db` — cold first call folds every
   * transcript; subsequent calls touch only `stat()` and the delta.
   */
  sessionListAll: () => invoke<SessionRow[]>("session_list_all"),
  /**
   * Truncate the session-index cache and force the next `sessionListAll`
   * to re-parse every transcript from cold. Escape hatch for cases the
   * `(size, mtime)` guard can't see. Safe to call — no data loss; only
   * derived cache rows are dropped.
   */
  sessionIndexRebuild: () => invoke<void>("session_index_rebuild"),
  /**
   * Full transcript + row metadata for one session, keyed by its
   * UUID. Locates the slug by filename match, then streams the JSONL
   * into normalized `SessionEvent`s.
   */
  sessionRead: (sessionId: string) =>
    invoke<SessionDetail>("session_read", { sessionId }),
  /**
   * Preferred over `sessionRead` from the Sessions tab — reading by
   * path disambiguates the rare case where two .jsonl files share one
   * session_id (interrupted adopt/rescue). Path must live under
   * `<config>/projects/`.
   */
  sessionReadPath: (filePath: string) =>
    invoke<SessionDetail>("session_read_path", { filePath }),

  // ---------- Session debugger (Tier 1-3 port from claude-devtools) ----------
  /** Chunked event stream (User/Ai/System/Compact) with per-chunk linked tools. */
  sessionChunks: (filePath: string) =>
    invoke<SessionChunk[]>("session_chunks", { filePath }),
  /** Visible-context token attribution across six categories. */
  sessionContextAttribution: (filePath: string) =>
    invoke<ContextStats>("session_context_attribution", { filePath }),
  /** Export transcript and write to disk (0600 on Unix). Returns bytes written. */
  sessionExportToFile: (
    filePath: string,
    format: "md" | "json",
    outputPath: string,
  ) =>
    invoke<number>("session_export_to_file", {
      filePath,
      format,
      outputPath,
    }),
  /** Cross-session text search. Returns ranked hits. */
  sessionSearch: (query: string, limit = 25) =>
    invoke<SearchHit[]>("session_search", { query, limit }),
  /** Group all sessions by git repository (collapses worktrees). */
  sessionWorktreeGroups: () =>
    invoke<RepositoryGroup[]>("session_worktree_groups"),

};
