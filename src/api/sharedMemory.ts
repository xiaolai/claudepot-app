// Shared Memory API — cross-harness durable memory + indexed
// transcript search + Codex/Claude live MCP installer.
//
// Backs the Shared Memory section UI (WI-007) and the Settings →
// MCP Installer pane (WI-009).

import { invoke } from "@tauri-apps/api/core";

// ─── search / read ───────────────────────────────────────────

export interface SearchArgs {
  query: string;
  source_kind?: "claude_code" | "codex" | null;
  project_path?: string | null;
  since_ms?: number | null;
  until_ms?: number | null;
  limit?: number | null;
  offset?: number | null;
}

export interface SearchHit {
  exchange_id: string;
  file_path: string;
  session_id: string;
  source_kind: "claude_code" | "codex";
  project_path: string;
  git_branch: string | null;
  timestamp_ms: number | null;
  line_start: number | null;
  line_end: number | null;
  snippet: string;
  turn_index: number;
}

export interface SearchResponse {
  hits: SearchHit[];
  has_more: boolean;
}

export interface ReadLocatorArgs {
  file_path: string;
  exchange_id?: string | null;
  max_bytes?: number | null;
}

export interface ConversationRead {
  file_path: string;
  exchange_id: string | null;
  line_start: number;
  line_end: number;
  body: string;
  truncated: boolean;
}

// ─── memories ────────────────────────────────────────────────

export type MemoryScope = "global" | "project";
export type MemoryKind =
  | "fact"
  | "preference"
  | "pattern"
  | "constraint"
  | "summary";
export type CreatedByKind = "user" | "agent" | "import" | "system";

export interface ListMemoriesArgs {
  scope?: MemoryScope | null;
  project_path?: string | null;
  kind?: MemoryKind | null;
  include_archived?: boolean | null;
  limit?: number | null;
}

export interface Memory {
  id: string;
  scope: MemoryScope;
  project_path: string | null;
  kind: MemoryKind;
  content: string;
  created_by_kind: CreatedByKind;
  created_by: string;
  confidence: number | null;
  created_at_ms: number;
  updated_at_ms: number;
  archived_at_ms: number | null;
  /** proposed / accepted / rejected / suspect — this list surfaces all
   *  review states, so a consumer must distinguish them. */
  review_state: ReviewStateName;
}

export interface CreateMemoryArgs {
  scope: MemoryScope;
  project_path?: string | null;
  kind: MemoryKind;
  content: string;
  created_by: string;
  confidence?: number | null;
}

// ─── decisions ───────────────────────────────────────────────

export type DecisionStatus = "active" | "superseded" | "archived";

export interface ListDecisionsArgs {
  project_path?: string | null;
  status?: DecisionStatus | null;
  limit?: number | null;
}

export interface Decision {
  id: string;
  project_path: string | null;
  topic: string | null;
  decision: string;
  rationale: string | null;
  status: DecisionStatus;
  created_by_kind: CreatedByKind;
  created_by: string;
  created_at_ms: number;
  supersedes_id: string | null;
}

export interface LogDecisionArgs {
  decision: string;
  rationale?: string | null;
  topic?: string | null;
  project_path?: string | null;
  created_by: string;
  supersedes_id?: string | null;
}

// ─── evidence ────────────────────────────────────────────────

export interface ListEvidenceArgs {
  project_path?: string | null;
  limit?: number | null;
}

export interface Evidence {
  id: string;
  project_path: string | null;
  topic: string | null;
  summary: string;
  verification: string;
  /** JSON array of repo-relative paths that were changed. */
  files_changed_json: string;
  confidence: number;
  created_by_kind: CreatedByKind;
  created_by: string;
  created_at_ms: number;
}

// ─── memory links ────────────────────────────────────────────

export type LinkRelation = "evidence" | "origin" | "related" | "supersedes";

export interface MemoryLinksArgs {
  /** Exactly one identifies the parent whose links to read. */
  memory_id?: string | null;
  decision_id?: string | null;
  evidence_id?: string | null;
}

export interface MemoryLink {
  id: string;
  memory_id: string | null;
  decision_id: string | null;
  evidence_id: string | null;
  exchange_id: string | null;
  file_path: string | null;
  relation: LinkRelation;
}

// ─── discovery ───────────────────────────────────────────────

export interface ListSessionsArgs {
  source_kind?: "claude_code" | "codex" | null;
  project_path?: string | null;
  since_ms?: number | null;
  limit?: number | null;
  offset?: number | null;
}

export interface SessionSummary {
  file_path: string;
  session_id: string;
  source_kind: "claude_code" | "codex";
  project_path: string;
  git_branch: string | null;
  first_ts_ms: number | null;
  last_ts_ms: number | null;
  message_count: number;
  tokens_input: number;
  tokens_output: number;
}

export interface ProjectSummary {
  project_path: string;
  session_count: number;
  last_activity_ms: number | null;
}

// ─── installer / MCP health ──────────────────────────────────

export type SnippetScope = "user" | "project";

export interface InstallSnippetArgs {
  scope?: SnippetScope;
  /** Required when scope = "project". */
  project_path?: string;
  /** Power-user escape hatch: write to this exact path; bypasses scope. */
  out?: string;
}

export interface SnippetInstallResult {
  scope: SnippetScope;
  path: string;
  bytes_written: number;
  include_line: string;
  /** Files the user should paste `include_line` into. */
  target_files: string[];
}

export interface McpHealth {
  tool_visible: boolean;
  tool_count: number;
  error: string | null;
}

// ─── knowledge compiler: lesson triage ───────────────────────

export type ReviewStateName = "proposed" | "accepted" | "rejected" | "suspect";

export interface LessonRow {
  id: string;
  review_state: ReviewStateName;
  kind: string;
  /** The claim, in the distiller's words. */
  content: string;
  /** The imperative one-liner a future agent would see. */
  directive: string | null;
  confidence: number | null;
  /** `{"files":[…],"evidence":"…","commit":"…"}` */
  anchor_json: string | null;
  suspect_reason: string | null;
  /** The transcript this was learned from — the "show me what burned me" link. */
  origin_file_path: string | null;
  origin_exchange_id: string | null;
  /** What an accepted claim compiled into: guard / directive / note. */
  compile_target: string | null;
  /** Where the compiled guard landed, e.g. `scripts/repo-invariants.sh:6`. */
  guard_ref: string | null;
  project_path: string | null;
  created_at_ms: number;
}

export interface LessonCounts {
  proposed: number;
  accepted: number;
  rejected: number;
  suspect: number;
  /** Accepted claims compiled into a binding check. */
  enforced: number;
}

/** One coverage-grid row: a project and its review counts. */
export interface ProjectCounts {
  project_path: string;
  counts: LessonCounts;
}

export interface LessonListArgs {
  project_path?: string | null;
  /** A review state, or "all" to list every state (Know view). */
  state?: ReviewStateName | "all" | null;
  limit?: number | null;
}

export interface LessonAcceptArgs {
  id: string;
  /** Accept without an anchor (the lesson can never go suspect). When
   * false/omitted, the backend resolves the lesson's project HEAD. */
  no_anchor?: boolean;
}

// ─── recurrence tracking (Phase 3) ───────────────────────────

export type RecurrenceDetectedBy = "anchor" | "similarity";

export interface RecurrenceEvent {
  id: string;
  matched_memory_id: string;
  project_path: string;
  /** The recurring claim, in the new session's words. */
  new_content: string;
  new_exchange_id: string | null;
  new_file_path: string | null;
  detected_by: RecurrenceDetectedBy;
  detected_at_ms: number;
  status: "pending" | "confirmed" | "dismissed";
  confirmed_at_ms: number | null;
  /** The matched lesson's claim — what we already learned. */
  matched_content: string | null;
  /** The matched lesson's review state (accepted / suspect). */
  matched_state: string | null;
}

export interface RecurrenceCounts {
  /** Confirmed recurrences within the trailing window. */
  confirmed_window: number;
  /** Candidates awaiting a human decision. */
  pending: number;
  window_days: number;
}

// ─── API surface ─────────────────────────────────────────────

export const sharedMemoryApi = {
  search: (args: SearchArgs) =>
    invoke<SearchResponse>("shared_memory_search", { args }),
  readLocator: (args: ReadLocatorArgs) =>
    invoke<ConversationRead>("shared_memory_read_locator", { args }),

  listMemories: (args: ListMemoriesArgs = {}) =>
    invoke<Memory[]>("shared_memory_list_memories", { args }),
  createMemory: (args: CreateMemoryArgs) =>
    invoke<Memory>("shared_memory_create_memory", { args }),
  archiveMemory: (id: string) =>
    invoke<boolean>("shared_memory_archive_memory", { id }),

  listDecisions: (args: ListDecisionsArgs = {}) =>
    invoke<Decision[]>("shared_memory_list_decisions", { args }),
  logDecision: (args: LogDecisionArgs) =>
    invoke<Decision>("shared_memory_log_decision", { args }),
  archiveDecision: (id: string) =>
    invoke<boolean>("shared_memory_archive_decision", { id }),

  listEvidence: (args: ListEvidenceArgs = {}) =>
    invoke<Evidence[]>("shared_memory_list_evidence", { args }),
  memoryLinks: (args: MemoryLinksArgs) =>
    invoke<MemoryLink[]>("shared_memory_memory_links", { args }),

  listSessions: (args: ListSessionsArgs = {}) =>
    invoke<SessionSummary[]>("shared_memory_list_sessions", { args }),
  listProjects: (limit?: number) =>
    invoke<ProjectSummary[]>("shared_memory_list_projects", { limit }),

  installSnippet: (args: InstallSnippetArgs = {}) =>
    invoke<SnippetInstallResult>("shared_memory_install_snippet", { args }),
  snippetBody: () => invoke<string>("shared_memory_snippet_body"),
  mcpHealth: (claudepotBinary?: string) =>
    invoke<McpHealth>("shared_memory_mcp_health", {
      claudepotBinary,
    }),

  // Knowledge compiler.
  lessonList: (args: LessonListArgs = {}) =>
    invoke<LessonRow[]>("lesson_list", { args }),
  lessonCounts: (projectPath?: string) =>
    invoke<LessonCounts>("lesson_counts", { projectPath }),
  lessonCountsByProject: () =>
    invoke<ProjectCounts[]>("lesson_counts_by_project"),
  lessonAccept: (args: LessonAcceptArgs) =>
    invoke<boolean>("lesson_accept", { args }),
  lessonReject: (id: string) => invoke<boolean>("lesson_reject", { id }),

  // Recurrence tracking.
  recurrenceList: (projectPath?: string) =>
    invoke<RecurrenceEvent[]>("recurrence_list", { projectPath }),
  recurrenceConfirm: (id: string) =>
    invoke<boolean>("recurrence_confirm", { id }),
  recurrenceDismiss: (id: string) =>
    invoke<boolean>("recurrence_dismiss", { id }),
  recurrenceCounts: () => invoke<RecurrenceCounts>("recurrence_counts"),
};
