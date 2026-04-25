// Session move (orphan / adopt / discard) + Sessions tab index DTOs + session debugger types.
// Sharded from src/types.ts to keep each domain's DTOs in its own
// file; src/types/index.ts re-exports them. Mirrors src-tauri/src/dto.rs.

// ---------- Session move ----------

export interface OrphanedProject {
  slug: string;
  cwdFromTranscript: string | null;
  sessionCount: number;
  totalSizeBytes: number;
  suggestedAdoptionTarget: string | null;
}

export interface MoveSessionReport {
  sessionId: string | null;
  fromSlug: string;
  toSlug: string;
  jsonlLinesRewritten: number;
  subagentFilesMoved: number;
  remoteAgentFilesMoved: number;
  historyEntriesMoved: number;
  historyEntriesUnmapped: number;
  claudeJsonPointersCleared: number;
  sourceDirRemoved: boolean;
}

export interface AdoptFailure {
  sessionId: string;
  error: string;
}

export interface AdoptReport {
  sessionsAttempted: number;
  sessionsMoved: number;
  sessionsFailed: AdoptFailure[];
  sourceDirRemoved: boolean;
  perSession: MoveSessionReport[];
}

export interface DiscardReport {
  sessionsDiscarded: number;
  totalSizeBytes: number;
  dirRemoved: boolean;
}

// ---------- Session index (Sessions tab) ----------

export interface TokenUsage {
  input: number;
  output: number;
  cache_creation: number;
  cache_read: number;
  total: number;
}

/**
 * One row in the Sessions tab. Produced by a full-file scan of the
 * JSONL, so counts and token totals are authoritative. `project_path`
 * comes from the first JSONL `cwd` field when available; otherwise
 * from a lossy `unsanitize(slug)` fallback (hence
 * `project_from_transcript` as the reliability flag).
 */
export interface SessionRow {
  session_id: string;
  slug: string;
  file_path: string;
  file_size_bytes: number;
  last_modified_ms: number | null;
  project_path: string;
  project_from_transcript: boolean;
  /** RFC3339 of the earliest dated event. Null for empty sessions. */
  first_ts: string | null;
  last_ts: string | null;
  event_count: number;
  message_count: number;
  user_message_count: number;
  assistant_message_count: number;
  first_user_prompt: string | null;
  models: string[];
  tokens: TokenUsage;
  git_branch: string | null;
  cc_version: string | null;
  /** CC's internal display slug (e.g. "brave-otter-88"). */
  display_slug: string | null;
  has_error: boolean;
  is_sidechain: boolean;
}

/** Discriminated union over the JSONL event types CC writes. */
export type SessionEvent =
  | {
      kind: "userText";
      ts: string | null;
      uuid: string | null;
      text: string;
    }
  | {
      kind: "userToolResult";
      ts: string | null;
      uuid: string | null;
      tool_use_id: string;
      content: string;
      is_error: boolean;
    }
  | {
      kind: "assistantText";
      ts: string | null;
      uuid: string | null;
      model: string | null;
      text: string;
      usage: TokenUsage | null;
      stop_reason: string | null;
    }
  | {
      kind: "assistantToolUse";
      ts: string | null;
      uuid: string | null;
      model: string | null;
      tool_name: string;
      tool_use_id: string;
      /** Trimmed, newline-collapsed, 240-char cap. Use for display. */
      input_preview: string;
      /** Raw JSON of the tool input. Use for substring search only. */
      input_full: string;
    }
  | {
      kind: "assistantThinking";
      ts: string | null;
      uuid: string | null;
      text: string;
    }
  | {
      kind: "summary";
      ts: string | null;
      uuid: string | null;
      text: string;
    }
  | {
      kind: "system";
      ts: string | null;
      uuid: string | null;
      subtype: string | null;
      detail: string;
    }
  | {
      kind: "attachment";
      ts: string | null;
      uuid: string | null;
      name: string | null;
      mime: string | null;
    }
  | {
      kind: "fileSnapshot";
      ts: string | null;
      uuid: string | null;
      file_count: number;
    }
  | {
      kind: "other";
      ts: string | null;
      uuid: string | null;
      raw_type: string;
    }
  | {
      // Task-summary marker emitted by Claude after a /compact or
      // when an agent finalizes its run. Serialized by Rust via
      // `#[serde(rename = "taskSummary")]` on `SessionEventDto`.
      kind: "taskSummary";
      ts: string | null;
      uuid: string | null;
      summary: string;
    }
  | {
      kind: "malformed";
      line_number: number;
      error: string;
      preview: string;
    };

export interface SessionDetail {
  row: SessionRow;
  events: SessionEvent[];
}

// ---------------------------------------------------------------------------
// Session debugger (Tier 1-3 port)
// ---------------------------------------------------------------------------

export type MessageCategory =
  | "user"
  | "system"
  | "compact"
  | "hardNoise"
  | "ai";

/**
 * Paired tool call + result. Emitted as part of `SessionChunk["ai"]`'s
 * `tool_executions`; also consumed directly by the specialized tool
 * viewers (Edit / Read / Write / Bash).
 */
export interface LinkedTool {
  tool_use_id: string;
  tool_name: string;
  model: string | null;
  call_ts: string | null;
  /** Trimmed, newline-collapsed, 240-char cap. Use for display. */
  input_preview: string;
  /** Raw JSON of the tool input. Use for substring search only. */
  input_full: string;
  result_ts: string | null;
  result_content: string | null;
  is_error: boolean;
  duration_ms: number | null;
  call_index: number;
  result_index: number | null;
}

export interface ChunkMetrics {
  duration_ms: number;
  tokens: {
    input: number;
    output: number;
    cache_creation: number;
    cache_read: number;
    /** Rust DTO adds `total` as a computed convenience. */
    total?: number;
  };
  message_count: number;
  tool_call_count: number;
  thinking_count: number;
}

interface BaseChunk {
  id: number;
  start_ts: string | null;
  end_ts: string | null;
  metrics: ChunkMetrics;
}

export type SessionChunk =
  | (BaseChunk & { chunkType: "user"; event_index: number })
  | (BaseChunk & {
      chunkType: "ai";
      event_indices: number[];
      tool_executions: LinkedTool[];
    })
  | (BaseChunk & { chunkType: "system"; event_index: number })
  | (BaseChunk & { chunkType: "compact"; event_index: number });

export interface Subagent {
  id: string;
  file_path: string;
  file_size_bytes: number;
  start_ts: string | null;
  end_ts: string | null;
  metrics: ChunkMetrics;
  parent_task_id: string | null;
  agent_type: string | null;
  description: string | null;
  is_parallel: boolean;
  events: SessionEvent[];
}

export interface ContextPhase {
  phase_number: number;
  start_index: number;
  end_index: number;
  start_ts: string | null;
  end_ts: string | null;
  summary: string | null;
}

export interface ContextPhaseInfo {
  phases: ContextPhase[];
  compaction_count: number;
}

export type ContextCategory =
  | "claude-md"
  | "mentioned-file"
  | "tool-output"
  | "thinking-text"
  | "team-coordination"
  | "user-message";

export interface TokensByCategory {
  claude_md: number;
  mentioned_file: number;
  tool_output: number;
  thinking_text: number;
  team_coordination: number;
  user_message: number;
}

export interface ContextInjection {
  event_index: number;
  category: ContextCategory;
  label: string;
  tokens: number;
  ts: string | null;
  phase: number;
}

export interface ContextStats {
  totals: TokensByCategory;
  injections: ContextInjection[];
  phases: ContextPhase[];
  reported_total_tokens: number;
}

export interface SearchHit {
  session_id: string;
  slug: string;
  file_path: string;
  project_path: string;
  role: "user" | "assistant";
  snippet: string;
  match_offset: number;
  last_ts: string | null;
  /**
   * Relevance score in (0, 1]. Higher = better.
   * 1.0 phrase match · 0.7 word-prefix · 0.4 substring.
   */
  score: number;
}

export interface RepositoryGroup {
  repo_root: string | null;
  label: string;
  sessions: SessionRow[];
  branches: string[];
  worktree_paths: string[];
}
